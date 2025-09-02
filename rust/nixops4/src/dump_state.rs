use crate::{
    control::task_tracker::TaskTracker,
    eval_client::EvalSender,
    interrupt::InterruptState,
    to_eval_options,
    work::{Goal, MutationCapability, Outcome, WorkContext},
    Options,
};
use anyhow::{bail, Context, Result};
use nixops4_core::eval_api::{
    AssignRequest, ComponentHandle, ComponentPath, EvalRequest, EvalResponse, FlakeRequest,
    RootRequest,
};
use pubsub_rs::Pubsub;
use std::sync::Arc;

pub(crate) async fn dump_state(
    interrupt_state: &InterruptState,
    options: &Options,
    resource_path_str: &str,
) -> Result<String> {
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

        let r = {
            let h: tokio::task::JoinHandle<Result<()>> = tokio::spawn(async move {
                while let Some(msg) = r.recv().await {
                    match &msg {
                        EvalResponse::Error(id, _) => {
                            id_subscriptions.publish(id.num(), msg).await;
                        }
                        EvalResponse::QueryResponse(id, _) => {
                            id_subscriptions.publish(id.num(), msg).await;
                        }
                        EvalResponse::TracingEvent(_value) => {
                            // Already handled in an EvalSender::with thread => ignore
                        }
                    }
                }
                Ok(())
            });

            // Resolve parent composite path to get its ID
            let parent_id = if parent_path.0.is_empty() {
                // Resource is at the root level
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
                    Err(e) => bail!("Failed to resolve parent path: {}", e),
                }
            };

            // Load the resource member to get its ID
            let resource_id = match tasks
                .run(Goal::LoadMember(parent_id, resource_name, None))
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
                Ok(Outcome::MemberLoaded(Err(preview_item))) => {
                    bail!(
                        "Cannot resolve resource at {}: structural dependency {:?}",
                        resource_path,
                        preview_item
                    )
                }
                Ok(other) => bail!("Unexpected outcome from LoadMember: {:?}", other),
                Err(e) => bail!("Failed to load resource: {}", e),
            };

            // Run the state provider for this resource
            let state_handle = match tasks
                .run(Goal::RunState(
                    resource_id,
                    resource_path.clone(),
                    MutationCapability,
                ))
                .await
                .as_ref()
            {
                Ok(Outcome::RunState(handle)) => handle.clone(),
                Ok(other) => bail!("Unexpected outcome from RunState: {:?}", other),
                Err(e) => bail!("Failed to run state provider: {}", e),
            };

            // Access the current state and dump it
            let state = state_handle.current.lock().await;
            let r = serde_json::to_string_pretty(&*state)?;

            // Clean up state providers after dump completes
            work_context.clean_up_state_providers().await?;

            s.close().await;
            h.await??;

            Ok(r)
        };

        r
    })
    .await
}
