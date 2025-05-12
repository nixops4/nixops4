use std::{
    collections::{BTreeMap, HashMap},
    fmt::{Debug, Display},
    sync::{Arc, Mutex},
};

use crate::{
    bob::{self, Thunk},
    eval_client::EvalSender,
    interrupt::InterruptState,
    provider, to_eval_options,
};
use crate::{state, Options};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use nixops4_core::eval_api::{
    self, AssignRequest, DeploymentRequest, DeploymentType, EvalRequest, EvalResponse,
    FlakeRequest, Id, IdNum, NamedProperty, Property, QueryResponseValue, ResourceInputState,
    ResourceRequest, ResourceType,
};
use nixops4_resource::schema::v0;
use nixops4_resource_runner::{ResourceProviderClient, ResourceProviderConfig};
use pubsub_rs::Pubsub;
use serde_json::Value;
use tracing::{info_span, Instrument as _};

#[derive(clap::Parser, Debug)]
pub(crate) struct Args {
    #[arg(default_value = "default")]
    deployment: String,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Goal {
    Apply(),
    ListResources(),
    GetResourceProviderInfo(Id<ResourceType>, String),
    ListResourceInputs(Id<ResourceType>, String),
    GetResourceInputValue(Id<ResourceType>, String, String),
    // This goes directly to ApplyResource, but provides useful context for cyclic dependencies
    GetResourceOutputValue(Id<ResourceType>, String, String),
    ApplyResource(Id<ResourceType>, String),
    RunState(Id<ResourceType>, String),
}
impl Display for Goal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO: use a proper display
        // write!(f, "[{:?}]", self)
        match self {
            Goal::Apply() => write!(f, "Apply deployment"),
            Goal::ListResources() => write!(f, "List resources"),
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
enum Outcome {
    Done(),
    ResourcesListed(Vec<String>),
    ResourceProviderInfo(eval_api::ResourceProviderInfo),
    ResourceInputsListed(Vec<String>),
    ResourceInputValue(Value),
    ResourceOutputValue, /*Value ignored because passed eagerly before ResourceOutputs, a dependency of this.*/
    ResourceOutputs(v0::ExtantResource),
    RunState(Arc<state::StateHandle>),
}
struct ApplyState {
    resource_ids: BTreeMap<String, Id<ResourceType>>,
}
struct ApplyContext {
    options: crate::Options,
    eval_sender: EvalSender,
    deployment_id: Id<DeploymentType>,
    interrupt_state: InterruptState,
    state: Mutex<ApplyState>,
    pub id_subscriptions: Pubsub<IdNum, EvalResponse>,
}
impl ApplyContext {}
fn clone_result<T: Clone>(r: &Result<T>) -> Result<T> {
    match r {
        Ok(v) => Ok(v.clone()),
        Err(e) => bail!("{}", e),
    }
}

#[async_trait]
impl bob::BobClosure for ApplyContext {
    type Output = Result<Outcome>;

    type Key = Goal;

    async fn work(&self, context: bob::BobContext<Self>, key: Self::Key) -> Self::Output {
        let closure = context.closure();

        let r = match key {
            Goal::ListResources() => {
                (async {
                    let msg_id = closure.eval_sender.next_id();
                    let rx = closure.id_subscriptions.subscribe(vec![msg_id.num()]).await;
                    self.eval_sender
                        .query(msg_id, EvalRequest::ListResources, closure.deployment_id)
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
                                        break Ok(Outcome::ResourcesListed(resource_names))
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
                })
                .instrument(info_span!("Listing resources"))
                .await
            }
            Goal::ListResourceInputs(id, name) => {
                (async {
                    let msg_id = closure.eval_sender.next_id();
                    let rx = closure.id_subscriptions.subscribe(vec![msg_id.num()]).await;
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
                })
                .instrument(info_span!("Listing resource inputs", resource = name))
                .await
            }
            Goal::GetResourceInputValue(id, resource_name, input_name) => {
                let resource_name_2 = resource_name.clone();
                let input_name_2 = input_name.clone();
                (async {
                    let msg_id = closure.eval_sender.next_id();
                    let rx = closure.id_subscriptions.subscribe(vec![msg_id.num()]).await;
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
                                                let dep_id = closure.state.lock().unwrap().resource_ids
                                                        .get(x.dependency.resource_name())
                                                        .unwrap().clone();
                                                if let Some(property) = x.dependency.property_name() {
                                                    let r = context.require(Goal::GetResourceOutputValue(
                                                        dep_id,
                                                        x.dependency.resource_name().clone(),
                                                        property.clone(),
                                                    )).await.map_err(|e| {
                                                        anyhow::anyhow!(
                                                            "Dependency cycle detected while getting resource input value: {}",
                                                            e
                                                        )
                                                    })?;
                                                    let _ = clone_result(r.as_ref())?;
                                                    // Ignore output value, because ApplyResource already pushes all its outputs eagerly.
                                                    // Just let it loop back and let it try to evaluate the requested input again
                                                }
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
                })
                .instrument(info_span!(
                    "Getting resource input value",
                    resource = resource_name_2,
                    input = input_name_2
                ))
                .await
            }
            Goal::GetResourceOutputValue(id, name, property) => {
                let name_2 = name.clone();
                let property_2 = property.clone();
                (async move {
                    // Apply the resource
                    let r = context
                        .require(Goal::ApplyResource(id, name.clone()))
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Dependency cycle detected while getting resource output value: {}",
                                e
                            )
                        })?;
                    let outcome = clone_result(r.as_ref())?;
                    match outcome {
                        Outcome::ResourceOutputs(outputs) => {
                            if let Some(_value) = outputs
                                .output_properties
                                .map(|v| v.0)
                                .unwrap_or_default()
                                .get(&property)
                            {
                                Ok(Outcome::ResourceOutputValue)
                            } else {
                                bail!("Resource {} does not have output {}", name, property)
                            }
                        }
                        _ => panic!("Unexpected outcome from ApplyResource: {:?}", outcome),
                    }
                })
                .instrument(info_span!(
                    "Getting resource output value",
                    resource = name_2,
                    property = property_2
                ))
                .await
            }
            Goal::ApplyResource(id, name) => {
                let name_2 = name.clone();
                let context = context.clone();
                (async move {
                    let provider_info_thunk = context
                        .spawn(Goal::GetResourceProviderInfo(id, name.clone()))
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Dependency cycle detected while applying resource: {}",
                                e
                            )
                        })?;

                    let resource_inputs_list_id = context
                        .spawn(Goal::ListResourceInputs(id, name.clone()))
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Dependency cycle detected while applying resource: {}",
                                e
                            )
                        })?;

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
                            .await
                            .map_err(|e| {
                                anyhow::anyhow!(
                                    "Dependency cycle detected while evaluating input: {}",
                                    e
                                )
                            })?;
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
                        Some(state_resource_name) => {
                            let state_resource_id = closure
                                .state
                                .lock()
                                .unwrap()
                                .resource_ids
                                .get(state_resource_name)
                                .unwrap()
                                .clone();
                            let thunk = context
                                .spawn(Goal::RunState(
                                    state_resource_id,
                                    state_resource_name.clone(),
                                ))
                                .await
                                .map_err(|e| {
                                    anyhow::anyhow!(
                                        "Dependency cycle detected while applying resource: {}",
                                        e
                                    )
                                })?;
                            Some((state_resource_name, state_resource_id, thunk))
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

                    let state = match state_provider {
                        Some((state_resource_id, state_resource_name, thunk)) => {
                            let outcome = clone_result(thunk.force().await.as_ref())?;
                            match outcome {
                                Outcome::RunState(i) => Some(i),
                                _ => panic!("Unexpected outcome from RunState: {:?}", outcome),
                            }
                        }
                        None => None,
                    };

                    let span = info_span!("creating resource", name = name);

                    if closure.options.verbose {
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
                        None => {
                            let outputs = provider
                                .create(provider_info.resource_type.as_str(), &inputs)
                                .await
                                .with_context(|| {
                                    format!("Failed to create stateless resource {}", name)
                                })?;
                            let r = provider.close_wait().await?;
                            if !r.success() {
                                // We did get outputs, so this seems unlikely
                                bail!("Provider exited unexpectedly: {}", r);
                            }
                            outputs
                        }
                        Some(state) => match state.past.deployment.resources.get(&name) {
                            Some(past_resource) => {
                                if past_resource.type_ != provider_info.resource_type {
                                    bail!(
                                        "Resource type change is not supported: {} != {}",
                                        past_resource.type_,
                                        provider_info.resource_type
                                    );
                                }
                                let outputs = provider
                                    .update(
                                        provider_info.resource_type.as_str(),
                                        &inputs,
                                        &past_resource.input_properties,
                                        &past_resource.output_properties,
                                    )
                                    .await
                                    .with_context(|| {
                                        format!("Failed to update resource {}", name)
                                    })?;
                                let current_resource = state::ResourceState {
                                    type_: provider_info.resource_type.clone(),
                                    input_properties: inputs.clone(),
                                    output_properties: outputs.clone(),
                                };
                                if &current_resource != past_resource {
                                    state
                                        .resource_event(
                                            &name,
                                            "update",
                                            Some(past_resource),
                                            &current_resource,
                                        )
                                        .await?;
                                }
                                outputs
                            }
                            None => {
                                let outputs = provider
                                    .create(provider_info.resource_type.as_str(), &inputs)
                                    .await
                                    .with_context(|| {
                                        format!("Failed to create stateless resource {}", name)
                                    })?;
                                let current_resource = state::ResourceState {
                                    type_: provider_info.resource_type.clone(),
                                    input_properties: inputs.clone(),
                                    output_properties: outputs.clone(),
                                };
                                state
                                    .resource_event(&name, "create", None, &current_resource)
                                    .await?;
                                // let r = provider.close_wait().await?;
                                // if !r.success() {
                                //     bail!("Provider exited unexpectedly: {}", r);
                                // }
                                outputs
                            }
                        },
                    };

                    drop(span);

                    if closure.options.verbose {
                        eprintln!("Resource outputs: {:?}", outputs);
                    }

                    // Send the outputs eagerly, to avoid roundtrips and costly
                    // re-evaluation when some output is "missing" but not really
                    for (output_name, output_value) in outputs.iter() {
                        let output_prop = NamedProperty {
                            resource: name.clone(),
                            name: output_name.clone(),
                        };
                        closure
                            .eval_sender
                            .send(&EvalRequest::PutResourceOutput(
                                output_prop,
                                output_value.clone(),
                            ))
                            .await?;
                    }

                    let resource = v0::ExtantResource {
                        input_properties: v0::InputProperties(inputs),
                        output_properties: Some(v0::OutputProperties(outputs)),
                        type_: v0::ResourceType(provider_info.resource_type),
                    };

                    Ok(Outcome::ResourceOutputs(resource))
                })
                .instrument(info_span!("Applying resource", resource = name_2))
                .await
            }
            Goal::GetResourceProviderInfo(id, resource_name) => {
                (async {
                    let msg_id = closure.eval_sender.next_id();
                    let rx = closure.id_subscriptions.subscribe(vec![msg_id.num()]).await;
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
                })
                .instrument(info_span!(
                    "Getting resource provider info",
                    resource = resource_name
                ))
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

                    let handle = state::StateHandle::open(&provider_info, &resource).await?;

                    eprintln!("Read state {}: \n{:?}", name, &handle.past);

                    Ok(Outcome::RunState(handle))
                })
                .instrument(info_span!(
                    "Running state provider resource",
                    resource = name
                ))
                .await
            }
            Goal::Apply() => {
                (async {
                    let r = context.require(Goal::ListResources()).await;
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
                    if resources.is_empty() {
                        eprintln!("Deployment contains no resources; nothing to apply.");
                    } else {
                        eprintln!(
                            "The following resources will be checked, created and/or updated:"
                        );
                        for r in &resources {
                            eprintln!("  - {}", r);
                        }
                    }
                    closure.interrupt_state.check_interrupted()?;

                    // Assign ids to the resources

                    let resource_ids: BTreeMap<String, Id<ResourceType>> = resources
                        .iter()
                        .map(|name| (name.clone(), self.eval_sender.next_id()))
                        .collect();
                    for (r, id) in resource_ids.iter() {
                        closure
                            .eval_sender
                            .send(&EvalRequest::LoadResource(AssignRequest {
                                assign_to: *id,
                                payload: ResourceRequest {
                                    deployment: closure.deployment_id,
                                    name: r.clone(),
                                },
                            }))
                            .await?;
                        closure
                            .state
                            .lock()
                            .unwrap()
                            .resource_ids
                            .insert(r.clone(), *id);
                    }

                    let mut resource_thunk_map = BTreeMap::new();
                    for (resource_name, id) in resource_ids.iter() {
                        let r = context
                            .spawn(Goal::ApplyResource(*id, resource_name.clone()))
                            .await
                            .map_err(|e| {
                                anyhow::anyhow!(
                                    "Dependency cycle detected while applying resource: {}",
                                    e
                                )
                            })?;
                        resource_thunk_map.insert(*id, r);
                    }
                    let mut resource_map = BTreeMap::new();
                    for (id, outcome) in force_map(resource_thunk_map).await {
                        match clone_result(&outcome)? {
                            Outcome::ResourceOutputs(i) => {
                                resource_map.insert(id, i);
                            }
                            _ => panic!("Unexpected outcome from ApplyResource: {:?}", outcome),
                        }
                    }

                    eprintln!("The following resources were created:");

                    // This is of questionable value, and we should probably only print values that are explicitly requested.
                    for (resource_name, resource_id) in resource_ids.iter() {
                        let empty = serde_json::Map::new();
                        let outputs = match resource_map.get(resource_id) {
                            Some(x) => x
                                .output_properties
                                .as_ref()
                                .map(|v| v.0.clone())
                                .unwrap_or_default()
                                .clone(),
                            None => serde_json::Map::new(),
                        };
                        let inputs = resource_map
                            .get(resource_id)
                            .map_or(&empty, |v| &v.input_properties);
                        eprintln!("  - resource {}", resource_name);
                        for (k, v) in inputs.iter() {
                            eprintln!("    - input {}: {}", k, indented_json(v));
                        }
                        for (k, v) in outputs.iter() {
                            eprintln!("    - output {}: {}", k, indented_json(v));
                        }
                    }

                    Ok(Outcome::Done())
                })
                .instrument(info_span!("Applying"))
                .await
            }
        };
        r
    }
}

async fn force_map<Key: Ord, Value>(
    provider_map: BTreeMap<Key, Arc<Thunk<Value>>>,
) -> BTreeMap<Key, Arc<Value>> {
    let mut result_map = BTreeMap::new();
    for (id, thunk) in provider_map {
        result_map.insert(id, thunk.force().await);
    }
    result_map
}

pub(crate) async fn apply_async(
    interrupt_state: &InterruptState,
    options: &Options, /* global options; apply options tbd, extra param */
    args: &Args,
) -> Result<()> {
    let options = options;
    let eval_options = to_eval_options(options);
    EvalSender::with(&eval_options, |s, mut r| async {
        let flake_id = s.next_id();
        // TODO: use better file path string type more
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        s.send(&EvalRequest::LoadFlake(AssignRequest {
            assign_to: flake_id,
            payload: FlakeRequest {
                abspath: cwd,
                input_overrides: eval_options.flake_input_overrides.clone(),
            },
        }))
        .await?;

        let deployment_id = s.next_id();
        s.send(&EvalRequest::LoadDeployment(AssignRequest {
            assign_to: deployment_id,
            payload: DeploymentRequest {
                flake: flake_id,
                name: args.deployment.to_string(),
            },
        }))
        .await?;

        let apply_context = Arc::new(Box::new(ApplyContext {
            options: options.clone(),
            eval_sender: s,
            deployment_id: deployment_id,
            state: Mutex::new(ApplyState {
                resource_ids: BTreeMap::new(),
            }),
            interrupt_state: interrupt_state.clone(),
            id_subscriptions: Pubsub::new(),
        }));
        let apply_context_2 = apply_context.clone();

        let bob = bob::BobState::new_arc(apply_context.clone());

        let r = {
            let h: tokio::task::JoinHandle<Result<()>> = tokio::spawn(async move {
                while let Some(msg) = r.recv().await {
                    match &msg {
                        EvalResponse::Error(id, _) => {
                            apply_context.id_subscriptions.publish(id.num(), msg).await;
                        }
                        EvalResponse::QueryResponse(id, _) => {
                            apply_context.id_subscriptions.publish(id.num(), msg).await;
                        }
                        EvalResponse::TracingEvent(_value) => {
                            // Already handled in an EvalSender::with thread => ignore
                        }
                    }
                }
                Ok(())
            });
            let r = bob.run(Goal::Apply()).await;
            apply_context_2.eval_sender.close().await;
            h.await??;
            r
        };
        match r.as_ref() {
            Ok(_) => Ok(()),
            Err(e) => {
                bail!("{}", e);
            }
        }
    })
    .await
}

fn indented_json(v: &Value) -> String {
    let s = serde_json::to_string_pretty(v).unwrap();
    s.replace("\n", "\n            ")
}
