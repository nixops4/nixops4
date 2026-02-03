use crate::{
    control::task_tracker::TaskTracker,
    eval_client::EvalSender,
    interrupt::InterruptState,
    to_eval_options,
    work::{clone_anyhow_from_arc, Goal, MutationCapability, Outcome, WorkContext},
    Options,
};
use anyhow::{bail, Context, Result};
use nixops4_core::eval_api::{
    AssignRequest, ComponentPath, EvalRequest, EvalResponse, FlakeRequest, RootRequest,
};
use pubsub_rs::Pubsub;
use std::sync::Arc;

#[derive(clap::Parser, Debug)]
pub(crate) struct Args {
    /// Component paths to apply, including nested members and transitive dependencies (empty = entire root)
    #[arg()]
    paths: Vec<String>,
}

/// Check if path `a` is an ancestor of path `b`
fn is_ancestor(a: &ComponentPath, b: &ComponentPath) -> bool {
    if a.0.len() >= b.0.len() {
        return false;
    }
    b.0.starts_with(&a.0)
}

/// Validate that no path is an ancestor of another
fn validate_no_overlapping_paths(paths: &[ComponentPath]) -> Result<()> {
    for (i, a) in paths.iter().enumerate() {
        for (j, b) in paths.iter().enumerate() {
            if i != j && is_ancestor(a, b) {
                bail!("Overlapping paths: '{}' is an ancestor of '{}'", a, b);
            }
        }
    }
    Ok(())
}

/// Run the `apply` command.
pub(crate) async fn apply(
    interrupt_state: &InterruptState,
    options: &Options, /* global options; apply options tbd, extra param */
    args: &Args,
) -> Result<()> {
    // Parse paths and validate
    let paths: Vec<ComponentPath> = if args.paths.is_empty() {
        vec![ComponentPath::root()]
    } else {
        args.paths.iter().map(|s| s.parse().unwrap()).collect()
    };
    validate_no_overlapping_paths(&paths)?;

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

            // Spawn all path applications concurrently
            let handles: Vec<_> = paths
                .into_iter()
                .map(|path| {
                    let tasks = tasks.clone();
                    tokio::spawn(async move {
                        // Resolve path to get the target composite ID
                        let target_id = match tasks
                            .run(Goal::ResolveCompositePath(path.clone()))
                            .await
                            .as_ref()
                        {
                            Ok(Outcome::CompositeResolved(id)) => *id,
                            Ok(other) => {
                                return Err(anyhow::anyhow!(
                                    "Unexpected outcome from ResolveCompositePath: {:?}",
                                    other
                                ))
                            }
                            Err(e) => return Err(clone_anyhow_from_arc(e)),
                        };

                        // Apply with matching id/path
                        match tasks
                            .run(Goal::Apply(target_id, path, MutationCapability))
                            .await
                            .as_ref()
                        {
                            Ok(_) => Ok(()),
                            Err(e) => Err(clone_anyhow_from_arc(e)),
                        }
                    })
                })
                .collect();

            // Await all handles and collect errors
            let mut errors = Vec::new();
            for handle in handles {
                match handle.await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => errors.push(e),
                    Err(e) => errors.push(anyhow::anyhow!("Task panicked: {}", e)),
                }
            }

            let result: Result<(), anyhow::Error> = if errors.is_empty() {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "{}",
                    errors
                        .iter()
                        .map(|e| format!("{:#}", e))
                        .collect::<Vec<_>>()
                        .join("\n")
                ))
            };

            // TODO: These cleanup operations should collect all errors and report them together
            // instead of stopping at the first error, since we want all cleanup to complete

            // Clean up state providers after apply completes - fatal error if this fails
            work_context.clean_up_state_providers().await?;

            s.close().await;
            h.await??;
            result
        };
        r
    })
    .await
}
