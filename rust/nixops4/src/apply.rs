use crate::{
    control::task_tracker::TaskTracker,
    eval_client::EvalSender,
    interrupt::InterruptState,
    to_eval_options,
    work::{Goal, WorkContext},
    Options,
};
use anyhow::{bail, Context, Result};
use nixops4_core::eval_api::{
    AssignRequest, DeploymentRequest, EvalRequest, EvalResponse, FlakeRequest,
};
use pubsub_rs::Pubsub;
use std::sync::Arc;

#[derive(clap::Parser, Debug)]
pub(crate) struct Args {
    #[arg(default_value = "default")]
    deployment: String,
}

/// Run the `apply` command.
pub(crate) async fn apply(
    interrupt_state: &InterruptState,
    options: &Options, /* global options; apply options tbd, extra param */
    args: &Args,
) -> Result<()> {
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
            let r = tasks
                .run(Goal::Apply(nixops4_core::eval_api::DeploymentPath::root()))
                .await;

            // TODO: These cleanup operations should collect all errors and report them together
            // instead of stopping at the first error, since we want all cleanup to complete

            // Clean up state providers after apply completes - fatal error if this fails
            work_context.clean_up_state_providers().await?;

            s.close().await;
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
