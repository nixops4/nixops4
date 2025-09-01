use crate::{
    control::{
        task_tracker::{Cycle, TaskContext, TaskWork},
        thunk::Thunk,
    },
    eval_client,
    interrupt::InterruptState,
    provider,
};
use anyhow::{bail, Context as _, Result};
use nixops4_core::eval_api::{
    self, AssignRequest, DeploymentPath, DeploymentType, EvalRequest, EvalResponse, Id, IdNum,
    NamedProperty, NestedDeploymentRequest, Property, QueryResponseValue, ResourceInputState,
    ResourcePath, ResourceRequest, ResourceType,
};
use nixops4_resource_runner::{ResourceProviderClient, ResourceProviderConfig};
use pubsub_rs::Pubsub;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fmt::Display,
    sync::Arc,
};
use std::{future::Future, pin::Pin};
use tokio::sync::Mutex;
use tracing::{info_span, Instrument as _};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Goal {
    Apply(DeploymentPath),
    ListResources(DeploymentPath),
    AssignDeploymentId(DeploymentPath),
    AssignResourceId(ResourcePath),
    GetResourceProviderInfo(Id<ResourceType>, ResourcePath),
    ListResourceInputs(Id<ResourceType>, ResourcePath),
    GetResourceInputValue(Id<ResourceType>, ResourcePath, String),
    // This goes directly to ApplyResource, but provides useful context for cyclic dependencies
    GetResourceOutputValue(Id<ResourceType>, ResourcePath, String),
    ApplyResource(Id<ResourceType>, ResourcePath),
    RunState(Id<ResourceType>, ResourcePath),
}
impl Display for Goal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Goal::Apply(path) => write!(f, "Apply deployment {:?}", path.0),
            Goal::ListResources(path) => write!(f, "List resources in deployment {:?}", path.0),
            Goal::AssignDeploymentId(path) => {
                write!(f, "Assign deployment ID for path {:?}", path.0)
            }
            Goal::AssignResourceId(name) => write!(f, "Assign resource ID to {}", name),
            Goal::GetResourceProviderInfo(_, name) => {
                write!(f, "Get resource provider info for {}", name)
            }
            Goal::ListResourceInputs(_, name) => {
                write!(f, "List resource inputs for {}", name)
            }
            Goal::GetResourceInputValue(_, name, input) => {
                write!(
                    f,
                    "Get resource input value for resource {} input {}",
                    name, input
                )
            }
            Goal::GetResourceOutputValue(_, name, property) => {
                write!(
                    f,
                    "Get resource output value from resource {} property {}",
                    name, property
                )
            }
            Goal::ApplyResource(_, name) => {
                write!(f, "Apply resource {}", name)
            }
            Goal::RunState(_, name) => {
                write!(f, "Run state provider resource {}", name)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum Outcome {
    Done(),
    ResourcesListed(Vec<ResourcePath>),
    DeploymentId(Id<DeploymentType>),
    ResourceId(Id<ResourceType>),
    ResourceProviderInfo(eval_api::ResourceProviderInfo),
    ResourceInputsListed(Vec<String>),
    ResourceInputValue(Value),
    ResourceOutputValue, /*Value ignored because passed eagerly before ResourceOutputs, a dependency of this.*/
    ResourceOutputs(nixops4_resource::schema::v0::ExtantResource),
    RunState(Arc<crate::state::StateHandle>),
}

pub struct WorkContext {
    pub options: crate::Options,
    pub eval_sender: eval_client::EvalSender,
    pub root_deployment_id: Id<DeploymentType>,
    pub interrupt_state: InterruptState,
    pub state: Mutex<WorkState>,
    pub id_subscriptions: Pubsub<IdNum, EvalResponse>,
}
pub struct WorkState {
    cleanup_tasks:
        Vec<Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send>>,
}
impl Default for WorkState {
    fn default() -> Self {
        Self {
            cleanup_tasks: Vec::new(),
        }
    }
}

impl WorkContext {
    pub async fn clean_up_state_providers(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        let cleanup_tasks = std::mem::take(&mut state.cleanup_tasks);
        drop(state); // Release lock before executing tasks

        let mut errors = Vec::new();
        for cleanup_task in cleanup_tasks {
            let future = cleanup_task();
            if let Err(e) = future.await {
                errors.push(e);
            }
        }

        if !errors.is_empty() {
            let error_messages: Vec<String> = errors.iter().map(|e| format!("{:#}", e)).collect();
            bail!(
                "Failed to close {} state provider(s):\n\n{}",
                errors.len(),
                error_messages.join("\n\n======== NEXT PROVIDER FAILURE ========\n")
            );
        }

        Ok(())
    }

    async fn list_resources(
        &self,
        context: &TaskContext<Self>,
        deployment_path: &DeploymentPath,
    ) -> Result<Vec<ResourcePath>> {
        let r = context
            .require(Goal::ListResources(deployment_path.clone()))
            .await;
        let outcome = match r {
            Err(e) => {
                bail!("Error while listing resources: {}", e);
            }
            Ok(v) => clone_result(v.as_ref()),
        }?;
        let resources = match outcome {
            Outcome::ResourcesListed(items) => items,
            outcome => {
                bail!("Unexpected outcome from ListResources, {:?}", outcome)
            }
        };
        Ok(resources)
    }

    // -----------------------------------------------------------------------
    // NOTE: perform_* functions should only be called from the work() function
    // and should not be called directly, so that we can ensure that work is
    // deduplicated and that we don't have cycles.
    // -----------------------------------------------------------------------

    async fn perform_apply(
        &self,
        context: TaskContext<Self>,
        deployment_path: DeploymentPath,
    ) -> Result<Outcome> {
        let resources = self.list_resources(&context, &deployment_path).await?;
        if resources.is_empty() {
            eprintln!("Deployment contains no resources; nothing to apply.");
        } else {
            eprintln!("The following resources will be checked, created and/or updated:");
            for r in &resources {
                eprintln!("  - {}", r);
            }
        }
        self.interrupt_state.check_interrupted()?;

        // Get resource IDs using the task tracker
        let mut resource_id_thunks = BTreeMap::new();
        for resource_name in &resources {
            let thunk = context
                .spawn(Goal::AssignResourceId(resource_name.clone()))
                .await?;
            resource_id_thunks.insert(resource_name.clone(), thunk);
        }

        // Force all resource IDs and collect the mapping
        let resource_ids: BTreeMap<ResourcePath, Id<ResourceType>> =
            Thunk::force_into_map(resource_id_thunks)
                .await
                .into_iter()
                .map(|(name, outcome)| {
                    match clone_result(&outcome).expect("Resource ID assignment failed") {
                        Outcome::ResourceId(id) => (name, id),
                        _ => panic!("Unexpected outcome from AssignResourceId"),
                    }
                })
                .collect();

        let mut resource_thunk_map = BTreeMap::new();
        for (resource_name, id) in resource_ids.iter() {
            let r = context
                .spawn(Goal::ApplyResource(*id, resource_name.clone()))
                .await?;
            resource_thunk_map.insert(*id, r);
        }
        let mut resource_map = BTreeMap::new();
        for (id, outcome) in Thunk::force_into_map(resource_thunk_map).await {
            match clone_result(&outcome)? {
                Outcome::ResourceOutputs(i) => {
                    resource_map.insert(id, i);
                }
                _ => panic!("Unexpected outcome from ApplyResource: {:?}", outcome),
            }
        }

        // TODO: this application logic doesn't belong here

        eprintln!("The following resources were created:");

        // This is of questionable value, and we should probably only print values that are explicitly requested.
        for (resource_name, resource_id) in resource_ids.iter() {
            if let Some(resource) = resource_map.get(resource_id) {
                eprintln!("  - resource {}", resource_name);
                for (k, v) in resource.input_properties.iter() {
                    eprintln!("    - input {}: {}", k, indented_json(v));
                }
                if let Some(output_properties) = &resource.output_properties {
                    for (k, v) in output_properties.iter() {
                        eprintln!("    - output {}: {}", k, indented_json(v));
                    }
                }
            }
        }

        Ok(Outcome::Done())
    }

    async fn perform_list_resources(
        &self,
        context: TaskContext<Self>,
        deployment_path: DeploymentPath,
    ) -> Result<Outcome> {
        // First resolve the deployment ID from the deployment path
        let deployment_id_thunk = context
            .require(Goal::AssignDeploymentId(deployment_path.clone()))
            .await?;
        let deployment_id = match clone_result(&deployment_id_thunk)? {
            Outcome::DeploymentId(id) => id,
            _ => panic!("Unexpected outcome from AssignDeploymentId"),
        };

        let msg_id = self.eval_sender.next_id();
        let rx = self.id_subscriptions.subscribe(vec![msg_id.num()]).await;
        self.eval_sender
            .query(msg_id, EvalRequest::ListResources, deployment_id)
            .await?;

        loop {
            let (_id, r) = rx
                .recv()
                .await
                .context("waiting for ListResources response")?;
            match r {
                EvalResponse::Error(_id, e) => bail!("Evaluation error: {}", e),
                EvalResponse::QueryResponse(_id, query_response_value) => {
                    match query_response_value {
                        QueryResponseValue::ListResources((_dt, resource_names)) => {
                            let resource_paths = resource_names
                                .into_iter()
                                .map(|name| ResourcePath {
                                    deployment_path: deployment_path.clone(),
                                    resource_name: name,
                                })
                                .collect();
                            break Ok(Outcome::ResourcesListed(resource_paths));
                        }
                        _ => bail!(
                            "Unexpected response to ListResources, {:?}",
                            query_response_value
                        ),
                    }
                }
                EvalResponse::TracingEvent(_) => {}
            }
        }
    }

    async fn perform_get_resource_id(
        &self,
        context: TaskContext<Self>,
        name: ResourcePath,
    ) -> Result<Outcome> {
        // First resolve the deployment ID from the deployment path
        let deployment_id_thunk = context
            .require(Goal::AssignDeploymentId(name.deployment_path.clone()))
            .await?;
        let deployment_id = match clone_result(&deployment_id_thunk)? {
            Outcome::DeploymentId(id) => id,
            _ => panic!("Unexpected outcome from AssignDeploymentId"),
        };

        let id = self.eval_sender.next_id();
        self.eval_sender
            .send(&EvalRequest::LoadResource(AssignRequest {
                assign_to: id,
                payload: ResourceRequest {
                    deployment: deployment_id,
                    name: name.resource_name.clone(),
                },
            }))
            .await?;
        Ok(Outcome::ResourceId(id))
    }

    async fn perform_assign_deployment_id(
        &self,
        context: TaskContext<Self>,
        deployment_path: DeploymentPath,
    ) -> Result<Outcome> {
        if deployment_path.is_root() {
            // For root deployment, return the root deployment ID
            Ok(Outcome::DeploymentId(self.root_deployment_id))
        } else {
            // For nested deployments, recursively resolve the parent first
            let mut parent_path = deployment_path.0.clone();
            let current_name = parent_path.pop().unwrap(); // Get the last component
            let parent_deployment_path = DeploymentPath(parent_path);

            // Get the parent deployment ID
            let parent_id_thunk = context
                .require(Goal::AssignDeploymentId(parent_deployment_path))
                .await?;
            let parent_id = match clone_result(&parent_id_thunk)? {
                Outcome::DeploymentId(id) => id,
                _ => panic!("Unexpected outcome from AssignDeploymentId"),
            };

            // Now load the nested deployment from the parent
            let id = self.eval_sender.next_id();
            self.eval_sender
                .send(&EvalRequest::LoadNestedDeployment(AssignRequest {
                    assign_to: id,
                    payload: NestedDeploymentRequest {
                        parent_deployment: parent_id,
                        name: current_name,
                    },
                }))
                .await?;
            Ok(Outcome::DeploymentId(id))
        }
    }

    async fn perform_get_resource_provider_info(
        &self,
        _context: TaskContext<Self>,
        id: Id<ResourceType>,
        _resource_name: ResourcePath,
    ) -> Result<Outcome> {
        let msg_id = self.eval_sender.next_id();
        let rx = self.id_subscriptions.subscribe(vec![msg_id.num()]).await;
        self.eval_sender
            .query(msg_id, EvalRequest::GetResource, id)
            .await?;

        loop {
            let (_id, r) = rx
                .recv()
                .await
                .context("waiting for GetResourceProviderInfo response")?;
            match r {
                EvalResponse::Error(_id, e) => bail!("Evaluation error: {}", e),
                EvalResponse::QueryResponse(_id, query_response_value) => {
                    match query_response_value {
                        QueryResponseValue::ResourceProviderInfo(info) => {
                            break Ok(Outcome::ResourceProviderInfo(info))
                        }
                        _ => bail!(
                            "Unexpected response to GetResourceProviderInfo, {:?}",
                            query_response_value
                        ),
                    }
                }
                EvalResponse::TracingEvent(_) => {}
            }
        }
    }

    async fn perform_list_resource_inputs(
        &self,
        _context: TaskContext<Self>,
        id: Id<ResourceType>,
    ) -> Result<Outcome> {
        let msg_id = self.eval_sender.next_id();
        let rx = self.id_subscriptions.subscribe(vec![msg_id.num()]).await;
        self.eval_sender
            .query(msg_id, EvalRequest::ListResourceInputs, id)
            .await?;

        loop {
            let (_id, r) = rx
                .recv()
                .await
                .context("waiting for ListResourceInputs response")?;
            match r {
                EvalResponse::Error(_id, e) => bail!("Evaluation error: {}", e),
                EvalResponse::QueryResponse(_id, query_response_value) => {
                    match query_response_value {
                        QueryResponseValue::ListResourceInputs((_id, input_names)) => {
                            break Ok(Outcome::ResourceInputsListed(input_names))
                        }
                        _ => bail!(
                            "Unexpected response to ListResourceInputs, {:?}",
                            query_response_value
                        ),
                    }
                }
                EvalResponse::TracingEvent(_) => {}
            }
        }
    }

    async fn perform_get_resource_input_value(
        &self,
        context: TaskContext<Self>,
        id: Id<ResourceType>,
        _resource_name: ResourcePath,
        input_name: String,
    ) -> Result<Outcome> {
        let msg_id = self.eval_sender.next_id();
        let rx = self.id_subscriptions.subscribe(vec![msg_id.num()]).await;
        loop {
            self.eval_sender
                .query(
                    msg_id,
                    EvalRequest::GetResourceInput,
                    Property {
                        resource: id,
                        name: input_name.clone(),
                    },
                )
                .await?;

            let (_id, r) = rx
                .recv()
                .await
                .context("waiting for GetResourceInputValue response")?;
            match r {
                EvalResponse::Error(_id, e) => bail!("Evaluation error: {}", e),
                EvalResponse::QueryResponse(_id, query_response_value) => {
                    match query_response_value {
                        QueryResponseValue::ResourceInputState((_, state)) => {
                            match state {
                                ResourceInputState::ResourceInputValue((_, value)) => {
                                    break Ok(Outcome::ResourceInputValue(value))
                                }
                                ResourceInputState::ResourceInputDependency(x) => {
                                    // Get the resource ID using the task tracker
                                    let dep_id_thunk = context
                                        .require(Goal::AssignResourceId(
                                            x.dependency.resource.clone(),
                                        ))
                                        .await
                                        .with_context(|| {
                                            format!(
                                                "while getting resource ID for dependency: {}",
                                                &x.dependency.resource
                                            )
                                        })?;

                                    let dep_id = match clone_result(&dep_id_thunk)? {
                                        Outcome::ResourceId(id) => id,
                                        _ => panic!("Unexpected outcome from AssignResourceId"),
                                    };

                                    let property = x.dependency.name.clone();
                                    let resource = x.dependency.resource.clone();
                                    let r = context
                                        .require(Goal::GetResourceOutputValue(
                                            dep_id, resource, property,
                                        ))
                                        .await
                                        .with_context(|| {
                                            format!(
                                                "while getting resource input value: {}.{}",
                                                &x.dependency.resource, &x.dependency.name
                                            )
                                        })?;
                                    // Propagate any errors
                                    let _outcome = clone_result(&r)?;

                                    // Ignore output value, because ApplyResource already pushes all its outputs eagerly.
                                    // Just let it loop back and let it try to evaluate the requested input again
                                }
                            }
                        }
                        _ => bail!(
                            "Unexpected response to GetResourceInputValue, {:?}",
                            query_response_value
                        ),
                    }
                }
                EvalResponse::TracingEvent(_) => {}
            }
        }
    }

    async fn perform_get_resource_output_value(
        &self,
        context: TaskContext<Self>,
        id: Id<ResourceType>,
        resource_name: ResourcePath,
        property: String,
    ) -> Result<Outcome> {
        // Apply the resource
        let r = context
            .require(Goal::ApplyResource(id, resource_name.clone()))
            .await?;
        let outcome = clone_result(&r)?;
        match outcome {
            Outcome::ResourceOutputs(extant_resource) => {
                if let Some(output_properties) = &extant_resource.output_properties {
                    if output_properties.contains_key(&property) {
                        Ok(Outcome::ResourceOutputValue)
                    } else {
                        bail!(
                            "Resource {} does not have output {}",
                            resource_name,
                            property
                        )
                    }
                } else {
                    bail!("Resource {} has no output properties", resource_name)
                }
            }
            _ => panic!("Unexpected outcome from ApplyResource: {:?}", outcome),
        }
    }

    async fn perform_apply_resource(
        &self,
        context: TaskContext<Self>,
        id: Id<ResourceType>,
        name: ResourcePath,
    ) -> Result<Outcome> {
        let provider_info_thunk = context
            .spawn(Goal::GetResourceProviderInfo(id, name.clone()))
            .await?;

        let resource_inputs_list_id = context
            .spawn(Goal::ListResourceInputs(id, name.clone()))
            .await?;

        let inputs_list = {
            let outcome = clone_result(resource_inputs_list_id.force().await.as_ref())?;
            match outcome {
                Outcome::ResourceInputsListed(i) => i,
                _ => {
                    panic!("Unexpected outcome from ListResourceInputs: {:?}", outcome)
                }
            }
        };

        let mut inputs_thunks = HashMap::new();
        for input_name in inputs_list {
            let input_id = context
                .spawn(Goal::GetResourceInputValue(
                    id,
                    name.clone(),
                    input_name.clone(),
                ))
                .await?;
            inputs_thunks.insert(input_name, input_id);
        }

        let provider_info = {
            let outcome = clone_result(provider_info_thunk.force().await.as_ref())?;
            match outcome {
                Outcome::ResourceProviderInfo(i) => i,
                _ => panic!(
                    "Unexpected outcome from GetResourceProviderInfo: {:?}",
                    outcome
                ),
            }
        };

        let state_provider = match &provider_info.state {
            Some(state_resource_path) => {
                // Get the state resource ID using the task tracker
                let state_id_thunk = context
                    .require(Goal::AssignResourceId(state_resource_path.clone()))
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!("Dependency cycle detected while applying resource: {}", e)
                    })?;

                let state_resource_id = match clone_result(&state_id_thunk)? {
                    Outcome::ResourceId(id) => id,
                    _ => panic!("Unexpected outcome from AssignResourceId"),
                };

                let thunk = context
                    .spawn(Goal::RunState(
                        state_resource_id,
                        state_resource_path.clone(),
                    ))
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!("Dependency cycle detected while applying resource: {}", e)
                    })?;
                Some((state_resource_path, state_resource_id, thunk))
            }
            None => None,
        };

        let state = match state_provider {
            Some((_state_resource_id, _state_resource_name, thunk)) => {
                let outcome = clone_result(thunk.force().await.as_ref())?;
                match outcome {
                    Outcome::RunState(i) => Some(i),
                    _ => panic!("Unexpected outcome from RunState: {:?}", outcome),
                }
            }
            None => None,
        };

        let inputs = {
            let mut inputs = serde_json::Map::new();
            for (input_name, input_thunk) in inputs_thunks {
                let outcome = clone_result(input_thunk.force().await.as_ref())?;
                match outcome {
                    Outcome::ResourceInputValue(i) => {
                        inputs.insert(input_name, i);
                    }
                    _ => panic!(
                        "Unexpected outcome from GetResourceInputValue: {:?}",
                        outcome
                    ),
                }
            }
            inputs
        };

        let span = info_span!("creating resource", name = name.to_string().as_str());

        if self.options.verbose {
            eprintln!("Provider details for {}: {:?}", name, &provider_info);
            eprintln!("Resource inputs for {}: {:?}", name, inputs);
        }

        let provider_argv = provider::parse_provider(&provider_info.provider)?;
        // Run the provider
        let mut provider = ResourceProviderClient::new(ResourceProviderConfig {
            provider_executable: provider_argv.executable,
            provider_args: provider_argv.args,
        })
        .await?;

        let outputs = match state {
            None => provider
                .create(provider_info.resource_type.as_str(), &inputs, false)
                .await
                .with_context(|| format!("Failed to create stateless resource {}", name))?,
            Some(ref state_handle) => {
                let past_resource_opt = state_handle
                    .current
                    .lock()
                    .await
                    .deployment
                    .get_resource(&name)
                    .cloned();
                match past_resource_opt {
                    Some(past_resource) => {
                        if past_resource.type_ != provider_info.resource_type {
                            bail!(
                                "Resource type change is not supported: {} != {}",
                                past_resource.type_,
                                provider_info.resource_type
                            );
                        }

                        // Skip update if inputs haven't changed
                        let outputs = if inputs == past_resource.input_properties {
                            tracing::info!(
                                "Skipping update for resource {}: inputs unchanged",
                                name
                            );
                            past_resource.output_properties.clone()
                        } else {
                            tracing::info!("Updating resource {}: inputs changed", name);
                            provider
                                .update(
                                    provider_info.resource_type.as_str(),
                                    &inputs,
                                    &past_resource.input_properties,
                                    &past_resource.output_properties,
                                )
                                .await
                                .with_context(|| format!("Failed to update resource {}", name))?
                        };
                        let current_resource = crate::state::ResourceState {
                            type_: provider_info.resource_type.clone(),
                            input_properties: inputs.clone(),
                            output_properties: outputs.clone(),
                        };
                        if &current_resource != &past_resource {
                            state_handle
                                .resource_event(
                                    &name,
                                    "update",
                                    Some(&past_resource),
                                    &current_resource,
                                )
                                .await?;
                        }
                        outputs
                    }
                    None => {
                        let outputs = provider
                            .create(provider_info.resource_type.as_str(), &inputs, true)
                            .await
                            .with_context(|| format!("Failed to create resource {}", name))?;
                        let current_resource = crate::state::ResourceState {
                            type_: provider_info.resource_type.clone(),
                            input_properties: inputs.clone(),
                            output_properties: outputs.clone(),
                        };
                        state_handle
                            .resource_event(&name, "create", None, &current_resource)
                            .await?;
                        outputs
                    }
                }
            }
        };

        drop(span);

        if self.options.verbose {
            eprintln!("Resource outputs: {:?}", outputs);
        }

        // Send the outputs eagerly, to avoid roundtrips and costly
        // re-evaluation when some output is "missing" but not really
        for (output_name, output_value) in outputs.iter() {
            let output_prop = NamedProperty {
                resource: name.clone(),
                name: output_name.clone(),
            };
            self.eval_sender
                .send(&EvalRequest::PutResourceOutput(
                    output_prop,
                    output_value.clone(),
                ))
                .await?;
        }

        // Close the provider properly
        // We might want to reuse them in the future, but for now we launch one
        // per resource, except when it's a state provider (elsewhere).
        provider.close_wait().await?;

        let resource = nixops4_resource::schema::v0::ExtantResource {
            input_properties: nixops4_resource::schema::v0::InputProperties(inputs),
            output_properties: Some(nixops4_resource::schema::v0::OutputProperties(outputs)),
            type_: nixops4_resource::schema::v0::ResourceType(provider_info.resource_type),
        };

        Ok(Outcome::ResourceOutputs(resource))
    }
}

#[async_trait::async_trait]
impl TaskWork for WorkContext {
    type Output = Arc<std::result::Result<Outcome, Arc<anyhow::Error>>>;

    type Key = Goal;

    type CycleError = anyhow::Error;

    fn cycle_error(&self, cycle: Cycle<Self::Key>) -> Self::CycleError {
        anyhow::anyhow!("Cycle detected: {:?}", cycle)
    }

    async fn work(&self, context: TaskContext<Self>, key: Self::Key) -> Self::Output {
        let r = match key {
            Goal::Apply(deployment_path) => {
                self.perform_apply(context, deployment_path)
                    .instrument(info_span!("Applying deployment"))
                    .await
            }
            Goal::ListResources(deployment_path) => {
                self.perform_list_resources(context, deployment_path)
                    .instrument(info_span!("Listing resources"))
                    .await
            }
            Goal::AssignDeploymentId(path) => {
                self.perform_assign_deployment_id(context, path)
                    .instrument(info_span!("Assigning deployment ID"))
                    .await
            }
            Goal::AssignResourceId(name) => {
                self.perform_get_resource_id(context, name)
                    .instrument(info_span!("Getting resource ID"))
                    .await
            }

            Goal::GetResourceProviderInfo(id, resource_name) => {
                self.perform_get_resource_provider_info(context, id, resource_name.clone())
                    .instrument(info_span!(
                        "Getting resource provider info",
                        resource = ?resource_name
                    ))
                    .await
            }

            Goal::ListResourceInputs(id, name) => {
                self.perform_list_resource_inputs(context, id)
                    .instrument(info_span!("Listing resource inputs", resource = ?name))
                    .await
            }

            Goal::GetResourceInputValue(id, resource_name, input_name) => {
                let resource_name_2 = resource_name.clone();
                let input_name_2 = input_name.clone();
                self.perform_get_resource_input_value(context, id, resource_name, input_name)
                    .instrument(info_span!(
                        "Getting resource input value",
                        resource = ?resource_name_2,
                        input = input_name_2
                    ))
                    .await
            }

            Goal::GetResourceOutputValue(id, name, property) => {
                let name_2 = name.clone();
                let property_2 = property.clone();
                self.perform_get_resource_output_value(context, id, name, property)
                    .instrument(info_span!(
                        "Getting resource output value",
                        resource = ?name_2,
                        property = property_2
                    ))
                    .await
            }

            Goal::ApplyResource(id, name) => {
                let name_2 = name.clone();
                let context = context.clone();
                self.perform_apply_resource(context, id, name)
                    .instrument(info_span!("Applying resource", resource = ?name_2))
                    .await
            }

            Goal::RunState(id, name) => {
                (async {
                    // Apply the resource
                    let r = context
                        .require(Goal::ApplyResource(id, name.clone()))
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Dependency cycle detected while applying resource: {}",
                                e
                            )
                        })?;
                    let resource = {
                        let outcome = clone_result(r.as_ref())?;
                        match outcome {
                            Outcome::ResourceOutputs(i) => i,
                            _ => panic!("Unexpected outcome from ApplyResource: {:?}", outcome),
                        }
                    };

                    // Get the provider info
                    let provider_info_thunk = context
                        .spawn(Goal::GetResourceProviderInfo(id, name.clone()))
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Dependency cycle detected while applying resource: {}",
                                e
                            )
                        })?;
                    let provider_info = {
                        let outcome = clone_result(provider_info_thunk.force().await.as_ref())?;
                        match outcome {
                            Outcome::ResourceProviderInfo(i) => i,
                            _ => panic!(
                                "Unexpected outcome from GetResourceProviderInfo: {:?}",
                                outcome
                            ),
                        }
                    };

                    let handle = crate::state::StateHandle::open(&provider_info, &resource).await?;

                    // Register cleanup task for this state handle
                    {
                        let handle_for_cleanup = handle.clone();
                        let cleanup_task = Box::new(
                            move || -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
                                Box::pin(async move { handle_for_cleanup.close().await })
                            },
                        );
                        self.state.lock().await.cleanup_tasks.push(cleanup_task);
                    }

                    Ok(Outcome::RunState(handle))
                })
                .instrument(info_span!(
                    "Running state provider resource",
                    resource = ?name
                ))
                .await
            }
        };
        Arc::new(r.map_err(Arc::new))
    }
}

fn clone_result<T: Clone>(r: &std::result::Result<T, Arc<anyhow::Error>>) -> Result<T> {
    match r {
        Ok(v) => Ok(v.clone()),
        Err(e) => bail!("{}", e),
    }
}

fn indented_json(v: &Value) -> String {
    let s = serde_json::to_string_pretty(v).unwrap();
    s.replace("\n", "\n            ")
}
