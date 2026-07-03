use crate::{
    application::to_eval_options,
    control::task_tracker::TaskTracker,
    eval_client::EvalSender,
    interrupt::InterruptState,
    provider,
    state::StateHandle,
    work::{clone_anyhow_from_arc, Goal, Outcome, WorkContext},
    Options,
};
use anyhow::{bail, Context, Result};
use nixops4_core::eval_api::{
    AssignRequest, ComponentHandle, ComponentPath, EvalRequest, EvalResponse, FlakeRequest,
    ResourceProviderInfo, RootRequest,
};
use nixops4_resource::schema::v0;
use nixops4_resource_runner::{ResourceProviderClient, ResourceProviderConfig};
use pubsub_rs::Pubsub;
use std::sync::Arc;

/// Destroy a resource, removing it from both its provider and state.
///
/// Two-phase design:
///   Phase 1: Read-only eval queries (no MutationCapability) to gather provider
///            info, state provider info, and current resource state.
///   Phase 2: Imperative destroy at the CLI level — spawn provider, call destroy,
///            update state, close everything.
pub(crate) async fn destroy_resource(
    interrupt_state: &InterruptState,
    options: &Options,
    resource_path_str: &str,
) -> Result<()> {
    let resource_path: ComponentPath =
        resource_path_str.parse().context("parsing resource path")?;

    let (parent_path, resource_name) = resource_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Resource path must not be empty"))?;
    let resource_name = resource_name.to_string();

    let eval_options = to_eval_options(options);

    EvalSender::with(&eval_options.clone(), |s, mut r| async move {
        let flake_id = s.next_id();
        let cwd = std::env::current_dir()
            .context("getting current directory")?
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

        let root_id = s.next_id();
        s.send(&EvalRequest::LoadRoot(AssignRequest {
            assign_to: root_id,
            payload: RootRequest { flake: flake_id },
        }))
        .await?;

        let work_context = WorkContext {
            root_composite_id: root_id,
            options: options.clone(),
            interrupt_state: interrupt_state.clone(),
            eval_sender: s.clone(),
            state: Default::default(),
            id_subscriptions: Pubsub::new(),
        };

        let id_subscriptions = work_context.id_subscriptions.clone();
        let work_context = Arc::new(work_context);
        let tasks = TaskTracker::new(work_context.clone());

        let h: tokio::task::JoinHandle<Result<()>> = tokio::spawn(async move {
            while let Some(msg) = r.recv().await {
                match &msg {
                    EvalResponse::Error(id, _) => {
                        id_subscriptions.publish(id.num(), msg).await;
                    }
                    EvalResponse::QueryResponse(id, _) => {
                        id_subscriptions.publish(id.num(), msg).await;
                    }
                    EvalResponse::TracingEvent(_value) => {}
                }
            }
            Ok(())
        });

        // ======== Phase 1: Read-only info gathering ========

        // Resolve parent composite path
        let parent_id = if parent_path.0.is_empty() {
            root_id
        } else {
            match tasks
                .run(Goal::ResolveCompositePath(parent_path))
                .await
                .as_ref()
            {
                Ok(Outcome::CompositeResolved(id)) => *id,
                Ok(other) => {
                    bail!("Unexpected outcome from ResolveCompositePath: {:?}", other)
                }
                Err(e) => {
                    return Err(clone_anyhow_from_arc(e).context("Failed to resolve parent path"))
                }
            }
        };

        // Load the resource member to get its ID
        let resource_id = match tasks
            .run(Goal::LoadMember(parent_id, resource_name.clone(), None))
            .await
            .as_ref()
        {
            Ok(Outcome::MemberLoaded(Ok(ComponentHandle::Resource(id)))) => *id,
            Ok(Outcome::MemberLoaded(Ok(ComponentHandle::Composite(_)))) => {
                bail!(
                    "Expected resource at {}, but found composite",
                    resource_path
                )
            }
            Ok(Outcome::MemberLoaded(Err(dep))) => {
                bail!(
                    "Cannot resolve resource at {}: structural dependency (depends on {}.{})",
                    resource_path,
                    dep.depends_on.resource,
                    dep.depends_on.name,
                )
            }
            Ok(other) => bail!("Unexpected outcome from LoadMember: {:?}", other),
            Err(e) => return Err(clone_anyhow_from_arc(e).context("Failed to load resource")),
        };

        // Get resource provider info (read-only, no mutation capability)
        let provider_info: ResourceProviderInfo = match tasks
            .run(Goal::GetResourceProviderInfo(
                resource_id,
                resource_path.clone(),
                None,
            ))
            .await
            .as_ref()
        {
            Ok(Outcome::ResourceProviderInfo(info)) => info.clone(),
            Ok(other) => {
                bail!(
                    "Unexpected outcome from GetResourceProviderInfo: {:?}",
                    other
                )
            }
            Err(e) => {
                return Err(clone_anyhow_from_arc(e).context("Failed to get resource provider info"))
            }
        };

        // Gather state info and build the ExtantResource for destroy
        let (extant_resource, state_handle) = match &provider_info.state {
            Some(state_resource_path) => {
                gather_stateful_resource(&tasks, root_id, &resource_path, state_resource_path)
                    .await?
            }
            None => {
                gather_stateless_resource(&tasks, resource_id, &resource_path, &provider_info)
                    .await?
            }
        };

        // ======== Phase 2: Imperative destroy ========

        // Spawn resource provider
        let provider_argv = provider::parse_provider(&provider_info.provider)?;
        let mut resource_provider = ResourceProviderClient::new(ResourceProviderConfig {
            provider_executable: provider_argv.executable,
            provider_args: provider_argv.args,
        })
        .await
        .context("Failed to start resource provider")?;

        // Destroy the resource through its provider
        resource_provider
            .destroy(extant_resource)
            .await
            .with_context(|| format!("Failed to destroy resource {}", resource_path))?;

        // Close resource provider
        resource_provider
            .close_wait()
            .await
            .context("Failed to close resource provider")?;

        // If stateful, remove resource from state
        if let Some(state_handle) = state_handle {
            state_handle
                .resource_destroy_event(&resource_path)
                .await
                .with_context(|| {
                    format!("Failed to remove resource {} from state", resource_path)
                })?;

            state_handle
                .close()
                .await
                .context("Failed to close state provider")?;
        }

        eprintln!("Destroyed resource {}", resource_path);

        s.close().await;
        h.await??;

        Ok(())
    })
    .await
}

/// For a stateful resource: resolve the state provider, read state, and look up the resource.
async fn gather_stateful_resource(
    tasks: &TaskTracker<WorkContext>,
    root_id: nixops4_core::eval_api::Id<nixops4_core::eval_api::CompositeType>,
    resource_path: &ComponentPath,
    state_resource_path: &ComponentPath,
) -> Result<(v0::ExtantResource, Option<Arc<StateHandle>>)> {
    // Resolve state resource path
    let (state_parent_path, state_resource_name) = state_resource_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("State resource path must not be empty"))?;

    let state_parent_id = if state_parent_path.0.is_empty() {
        root_id
    } else {
        match tasks
            .run(Goal::ResolveCompositePath(state_parent_path))
            .await
            .as_ref()
        {
            Ok(Outcome::CompositeResolved(id)) => *id,
            Ok(other) => {
                bail!(
                    "Unexpected outcome from ResolveCompositePath for state: {:?}",
                    other
                )
            }
            Err(e) => {
                return Err(clone_anyhow_from_arc(e).context("Failed to resolve state parent path"))
            }
        }
    };

    let state_resource_id = match tasks
        .run(Goal::LoadMember(
            state_parent_id,
            state_resource_name.to_string(),
            None,
        ))
        .await
        .as_ref()
    {
        Ok(Outcome::MemberLoaded(Ok(ComponentHandle::Resource(id)))) => *id,
        Ok(other) => {
            bail!(
                "Unexpected outcome from LoadMember for state resource: {:?}",
                other
            )
        }
        Err(e) => return Err(clone_anyhow_from_arc(e).context("Failed to load state resource")),
    };

    // Get state provider info
    let state_provider_info: ResourceProviderInfo = match tasks
        .run(Goal::GetResourceProviderInfo(
            state_resource_id,
            state_resource_path.clone(),
            None,
        ))
        .await
        .as_ref()
    {
        Ok(Outcome::ResourceProviderInfo(info)) => info.clone(),
        Ok(other) => {
            bail!(
                "Unexpected outcome from GetResourceProviderInfo for state: {:?}",
                other
            )
        }
        Err(e) => return Err(clone_anyhow_from_arc(e).context("Failed to get state provider info")),
    };

    // List and get state resource inputs
    let state_input_names: Vec<String> = match tasks
        .run(Goal::ListResourceInputs(
            state_resource_id,
            state_resource_path.clone(),
            None,
        ))
        .await
        .as_ref()
    {
        Ok(Outcome::ResourceInputsListed(names)) => names.clone(),
        Ok(other) => bail!("Unexpected outcome from ListResourceInputs: {:?}", other),
        Err(e) => {
            return Err(clone_anyhow_from_arc(e).context("Failed to list state resource inputs"))
        }
    };

    let mut state_inputs = serde_json::Map::new();
    for input_name in &state_input_names {
        match tasks
            .run(Goal::GetResourceInputValue(
                state_resource_id,
                state_resource_path.clone(),
                input_name.clone(),
                None,
            ))
            .await
            .as_ref()
        {
            Ok(Outcome::ResourceInputValue(value)) => {
                state_inputs.insert(input_name.clone(), value.clone());
            }
            Ok(other) => bail!("Unexpected outcome from GetResourceInputValue: {:?}", other),
            Err(e) => {
                return Err(clone_anyhow_from_arc(e)
                    .context(format!("Failed to get state input '{}'", input_name)))
            }
        }
    }

    // Construct the state provider's ExtantResource (input_properties only, no outputs needed)
    let state_provider_resource = v0::ExtantResource {
        type_: v0::ResourceType(state_provider_info.resource_type.clone()),
        input_properties: v0::InputProperties(state_inputs),
        output_properties: None,
    };

    // Open state handle (reads state via state_read)
    let state_handle = StateHandle::open(&state_provider_info, &state_provider_resource)
        .await
        .context("Failed to open state provider for destroy")?;

    // Look up the target resource in state
    let resource_state = state_handle
        .current
        .lock()
        .await
        .deployment
        .get_resource(resource_path)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Resource {} not found in state", resource_path))?;

    // Build ExtantResource from state data
    let extant_resource = v0::ExtantResource {
        type_: v0::ResourceType(resource_state.type_.clone()),
        input_properties: v0::InputProperties(resource_state.input_properties.clone()),
        output_properties: Some(v0::OutputProperties(
            resource_state.output_properties.clone(),
        )),
    };

    Ok((extant_resource, Some(state_handle)))
}

/// For a stateless resource: get inputs from eval and build ExtantResource.
async fn gather_stateless_resource(
    tasks: &TaskTracker<WorkContext>,
    resource_id: nixops4_core::eval_api::Id<nixops4_core::eval_api::ResourceType>,
    resource_path: &ComponentPath,
    provider_info: &ResourceProviderInfo,
) -> Result<(v0::ExtantResource, Option<Arc<StateHandle>>)> {
    let input_names: Vec<String> = match tasks
        .run(Goal::ListResourceInputs(
            resource_id,
            resource_path.clone(),
            None,
        ))
        .await
        .as_ref()
    {
        Ok(Outcome::ResourceInputsListed(names)) => names.clone(),
        Ok(other) => bail!("Unexpected outcome from ListResourceInputs: {:?}", other),
        Err(e) => return Err(clone_anyhow_from_arc(e).context("Failed to list resource inputs")),
    };

    let mut inputs = serde_json::Map::new();
    for input_name in &input_names {
        match tasks
            .run(Goal::GetResourceInputValue(
                resource_id,
                resource_path.clone(),
                input_name.clone(),
                None,
            ))
            .await
            .as_ref()
        {
            Ok(Outcome::ResourceInputValue(value)) => {
                inputs.insert(input_name.clone(), value.clone());
            }
            Ok(other) => bail!("Unexpected outcome from GetResourceInputValue: {:?}", other),
            Err(e) => {
                return Err(clone_anyhow_from_arc(e)
                    .context(format!("Failed to get resource input '{}'", input_name)))
            }
        }
    }

    let extant_resource = v0::ExtantResource {
        type_: v0::ResourceType(provider_info.resource_type.clone()),
        input_properties: v0::InputProperties(inputs),
        output_properties: None,
    };

    Ok((extant_resource, None))
}
