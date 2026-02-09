use crate::control::task_tracker::TaskTracker;
use crate::eval_client::{self, EvalSender};
use crate::interrupt::InterruptState;
use crate::options::Options;
use crate::work::WorkContext;
use anyhow::{Context, Result};
use nixops4_core::eval_api::{AssignRequest, EvalRequest, EvalResponse, FlakeRequest, RootRequest};
use pubsub_rs::Pubsub;
use std::future::Future;
use std::process::exit;
use std::sync::Arc;

/// Create the single-threaded tokio runtime used by the CLI.
///
/// Panics if the runtime cannot be created.
pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to initialize tokio runtime")
}

/// Handle a Result, printing the error and exiting with code 1 on failure.
pub fn handle_result(r: Result<()>) {
    match r {
        Ok(()) => {}
        Err(e) => {
            eprintln!("nixops4 error: {:?}", e);
            exit(1);
        }
    }
}

/// Convert CLI options to eval client options.
pub fn to_eval_options(options: &Options) -> eval_client::Options {
    eval_client::Options {
        verbose: options.verbose,
        show_trace: options.show_trace,
        flake_input_overrides: options
            .override_input
            .chunks(2)
            .map(|pair| {
                assert!(
                    pair.len() == 2,
                    "override_input must have an even number of elements (clap num_args = 2)"
                );
                (pair[0].to_string(), pair[1].to_string())
            })
            .collect(),
        force_quiet: false,
    }
}

/// Run a command with evaluation context.
///
/// Sets up the evaluator subprocess, loads the flake and root, creates
/// a `WorkContext` and `TaskTracker`, and spawns the response handler.
/// After the provided function completes, cleans up resources.
pub async fn with_eval<F, Fut, R>(
    interrupt_state: &InterruptState,
    options: &Options,
    f: F,
) -> Result<R>
where
    F: FnOnce(Arc<WorkContext>, TaskTracker<WorkContext>) -> Fut,
    Fut: Future<Output = Result<R>>,
{
    let eval_options = to_eval_options(options);
    with_eval_impl(interrupt_state, options, &eval_options, f).await
}

/// Internal implementation that takes explicit eval_options.
async fn with_eval_impl<F, Fut, R>(
    interrupt_state: &InterruptState,
    options: &Options,
    eval_options: &eval_client::Options,
    f: F,
) -> Result<R>
where
    F: FnOnce(Arc<WorkContext>, TaskTracker<WorkContext>) -> Fut,
    Fut: Future<Output = Result<R>>,
{
    let eval_options = eval_options.clone();
    let flake_input_overrides = eval_options.flake_input_overrides.clone();

    EvalSender::with(&eval_options, |s, mut r| async move {
        let flake_id = s.next_id();
        let cwd = std::env::current_dir()
            .context("getting current directory")?
            .to_string_lossy()
            .to_string();
        s.send(&EvalRequest::LoadFlake(AssignRequest {
            assign_to: flake_id,
            payload: FlakeRequest {
                abspath: cwd,
                input_overrides: flake_input_overrides,
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
                    EvalResponse::TracingEvent(_) => {
                        // Already handled in an EvalSender::with thread
                    }
                }
            }
            Ok(())
        });

        let result = f(work_context.clone(), tasks).await;

        let cleanup_result = work_context.clean_up_state_providers().await;

        s.close().await;
        h.await??;

        and_cleanup(result, cleanup_result)
    })
    .await
}

/// Combine a primary result with a cleanup result, preserving both errors if both fail.
fn and_cleanup<T>(primary: Result<T>, cleanup: Result<()>) -> Result<T> {
    match (primary, cleanup) {
        (Ok(r), Ok(())) => Ok(r),
        (Ok(_), Err(e)) => Err(e),
        (Err(e), Ok(())) => Err(e),
        (Err(e1), Err(e2)) => Err(e1.context(format!("Additionally, cleanup failed: {}", e2))),
    }
}
