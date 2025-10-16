use crate::{
    control::task_tracker::TaskTracker,
    eval_client::EvalSender,
    interrupt::InterruptState,
    to_eval_options,
    work::{Goal, Outcome, WorkContext},
    Options,
};
use anyhow::{bail, Context, Result};
use futures::future;
use nixops4_core::eval_api::{
    AssignRequest, DeploymentPath, DeploymentRequest, EvalRequest, EvalResponse, FlakeRequest, Id,
    ResourcePath,
};
use pubsub_rs::Pubsub;
use std::sync::Arc;

#[derive(clap::Parser, Debug)]
pub(crate) struct Args {
    #[arg(default_value = "default")]
    deployment: String,
    #[arg(short, long, num_args=3, value_names = &["RESOURCE_PROVIDER", "IMPORT_NAME", "IMPORT_PROPERTIES"])]
    resources: Vec<String>,
}

pub(crate) async fn import_resources(
    interrupt_state: &InterruptState,
    options: &Options,
    args: &Args,
) -> Vec<Result<String>> {
    let resources: Vec<(String, String, String)> = args
        .resources
        .chunks(3)
        .map(|resource| {
            (
                resource[0].clone(), // resource provider
                resource[1].clone(), // resource name
                resource[2].clone(), // import properties
            )
        })
        .collect();

    eprintln!("DEBUG resources: {:?}", resources);

    let results = resources
        .into_iter()
        .map(|resource| import_resource(interrupt_state, options, args, resource));
    future::join_all(results).await
}

pub(crate) async fn import_resource(
    interrupt_state: &InterruptState,
    options: &Options,
    args: &Args,
    resource: (String, String, String),
) -> Result<String> {
    let eval_options = to_eval_options(options);

    eprintln!("DEBUG resource: {:?}", resource);

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
                name: args.deployment.to_string(),
            },
        }))
        .await?;

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

            let resource_id_result = tasks
                .run(Goal::AssignResourceId(ResourcePath {
                    deployment_path: DeploymentPath::root(),
                    resource_name: resource.1.clone(),
                }))
                .await;

            let resource_id = match resource_id_result.as_ref() {
                Ok(Outcome::ResourceId(id)) => *id,
                Ok(other) => {
                    bail!("Unexpected outcome when assigning resource ID: {:?}", other)
                }
                Err(e) => bail!("Failed to assign resource ID: {}", e),
            };

            let imported_resource = tasks
                .run(Goal::ImportResource(
                    resource_id,
                    ResourcePath {
                        deployment_path: DeploymentPath::root(),
                        resource_name: resource.1,
                    },
                    resource.2,
                ))
                .await;

            let result = match imported_resource.as_ref() {
                Ok(Outcome::ImportedResource(handle)) => handle.clone(),
                Ok(other) => bail!("Unexpected outcome from ImportResource: {:?}", other),
                Err(e) => bail!("Failed to import resource: {}", e),
            };

            let r = serde_json::to_string_pretty(&result)?;

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
