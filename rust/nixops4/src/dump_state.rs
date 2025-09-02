use crate::{
    control::task_tracker::TaskTracker,
    eval_client::EvalSender,
    interrupt::InterruptState,
    to_eval_options,
    work::{Goal, Outcome, WorkContext},
    Options,
};
use anyhow::{bail, Context, Result};
use nixops4_core::eval_api::{
    AssignRequest, DeploymentPath, DeploymentRequest, EvalRequest, EvalResponse, FlakeRequest,
    ResourcePath,
};
use pubsub_rs::Pubsub;
use std::sync::Arc;

pub(crate) async fn dump_state(
    interrupt_state: &InterruptState,
    options: &Options,
    resource_path: &str,
    deployment: &str,
) -> Result<String> {
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

        let deployment_id = s.next_id();
        s.send(&EvalRequest::LoadDeployment(AssignRequest {
            assign_to: deployment_id,
            payload: DeploymentRequest {
                flake: flake_id,
                name: deployment.to_string(),
            },
        }))
        .await?;

        // TODO: make WorkContext make work ask for confirmation whenever there's a dependency on something that isn't our target state file. We don't want to accidentally apply a bunch.
        let work_context = WorkContext {
            root_deployment_id: deployment_id,
            options: options.clone(),
            interrupt_state: interrupt_state.clone(),
            eval_sender: s.clone(),
            state: Default::default(),
            id_subscriptions: Pubsub::new(),
        };

        let id_subscriptions = work_context.id_subscriptions.clone();
        let work_context = Arc::new(Box::new(work_context));
        let tasks = TaskTracker::new_arc(work_context.clone());

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

            // Parse the resource path
            let parsed_resource_path = parse_resource_path(resource_path)?;

            // First, we need to get the resource type ID
            let resource_id_result = tasks
                .run(Goal::AssignResourceId(parsed_resource_path.clone()))
                .await;

            let resource_id = match resource_id_result.as_ref() {
                Ok(Outcome::ResourceId(id)) => *id,
                Ok(other) => bail!("Unexpected outcome when assigning resource ID: {:?}", other),
                Err(e) => bail!("Failed to assign resource ID: {}", e),
            };

            // Now run the state provider for this specific resource
            let state_result = tasks
                .run(Goal::RunState(resource_id, parsed_resource_path.clone()))
                .await;

            let state_handle = match state_result.as_ref() {
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

fn parse_resource_path(resource_path: &str) -> Result<ResourcePath> {
    // Parse resource path like "deployment.subdeployment.resource" or just "resource"
    let parts: Vec<&str> = resource_path.split('.').collect();

    if parts.is_empty() {
        bail!("Empty resource path");
    }

    let resource_name = parts.last().unwrap().to_string();
    let deployment_path = if parts.len() > 1 {
        DeploymentPath(
            parts[..parts.len() - 1]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        )
    } else {
        DeploymentPath(vec![])
    };

    Ok(ResourcePath {
        deployment_path,
        resource_name,
    })
}
