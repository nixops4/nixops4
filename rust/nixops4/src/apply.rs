use crate::{
    application,
    interrupt::InterruptState,
    work::{clone_anyhow_from_arc, resolve_composite_path, Goal, MutationCapability},
    Options,
};
use anyhow::{bail, Result};
use nixops4_core::eval_api::ComponentPath;

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

    application::with_eval(interrupt_state, options, |_work_ctx, tasks| async move {
        // Spawn all path applications concurrently
        let handles: Vec<_> = paths
            .into_iter()
            .map(|path| {
                let tasks = tasks.clone();
                tokio::spawn(async move {
                    let target_id = resolve_composite_path(&tasks, path.clone()).await?;

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

        result
    })
    .await
}
