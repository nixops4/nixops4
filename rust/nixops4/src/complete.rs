//! Dynamic shell completion for component paths.

use crate::application;
use crate::work::{resolve_composite_path, Goal, Outcome};
use clap_complete::engine::CompletionCandidate;
use nixops4_core::eval_api::{ComponentHandle, ComponentPath};
use std::ffi::OsStr;

// Public functions
// ----------------
// `clap_complete` expects this function signature, so we have
// to preapply our mode parameter. Consequently, `CompletionMode` is private.

/// Completer for apply: both composites and resources are valid.
pub fn component_path_completer_all(current: &OsStr) -> Vec<CompletionCandidate> {
    component_path_completer_inner(current, CompletionMode::AllValid).unwrap_or_default()
}

/// Completer for destroy: only resources are valid targets.
#[allow(dead_code)] // anticipated use in destroy command
pub fn component_path_completer_resource(current: &OsStr) -> Vec<CompletionCandidate> {
    component_path_completer_inner(current, CompletionMode::ResourceOnly).unwrap_or_default()
}

/// Completer for members list: only composites are valid targets.
pub fn component_path_completer_composite(current: &OsStr) -> Vec<CompletionCandidate> {
    component_path_completer_inner(current, CompletionMode::CompositeOnly).unwrap_or_default()
}

// Implementation
// --------------

/// Which component types are valid final targets.
#[derive(Clone, Copy)]
enum CompletionMode {
    /// Both composites and resources are valid (e.g., apply).
    AllValid,
    /// Only resources are valid (e.g., destroy).
    ResourceOnly,
    /// Only composites are valid (e.g., members list).
    CompositeOnly,
}

fn component_path_completer_inner(
    current: &OsStr,
    mode: CompletionMode,
) -> anyhow::Result<Vec<CompletionCandidate>> {
    let rt = application::try_runtime()?;
    let current = current.to_string_lossy();

    rt.block_on(async move {
        application::with_eval_dead_quiet(|work_context, tasks| async move {
            let root_id = work_context.root_composite_id;
            complete_with_mode(&tasks, root_id, current.as_ref(), mode).await
        })
        .await
    })
}

fn format_path(parent: Option<&str>, name: &str) -> String {
    match parent {
        Some(p) => format!("{}.{}", p, name),
        None => name.to_string(),
    }
}

/// Parse current input into parent path and prefix.
fn parse_current(current: &str) -> (Option<&str>, &str) {
    if let Some(dot_pos) = current.rfind('.') {
        (Some(&current[..dot_pos]), &current[dot_pos + 1..])
    } else {
        (None, current)
    }
}

/// List and filter member names at a path.
///
/// Crucially this must not determine component kind, so that it evaluates quickly.
async fn list_filtered_members(
    tasks: &crate::control::task_tracker::TaskTracker<crate::work::WorkContext>,
    composite_id: nixops4_core::eval_api::Id<nixops4_core::eval_api::CompositeType>,
    path: &nixops4_core::eval_api::ComponentPath,
    prefix: &str,
) -> anyhow::Result<Vec<String>> {
    let result = tasks
        .run(Goal::ListMembers(composite_id, path.clone(), None))
        .await;

    let names = match result.as_ref() {
        Ok(Outcome::MembersListed(Ok(names))) => names.clone(),
        _ => return Ok(vec![]),
    };

    Ok(names
        .into_iter()
        .filter(|n| n.starts_with(prefix))
        .collect())
}

async fn complete_with_mode(
    tasks: &crate::control::task_tracker::TaskTracker<crate::work::WorkContext>,
    root_id: nixops4_core::eval_api::Id<nixops4_core::eval_api::CompositeType>,
    current: &str,
    mode: CompletionMode,
) -> anyhow::Result<Vec<CompletionCandidate>> {
    let (parent_path_str, prefix) = parse_current(current);
    let parent_component_path: ComponentPath = parent_path_str.map_or(ComponentPath::root(), |s| {
        s.parse().unwrap_or_else(|_| ComponentPath::root())
    });
    let composite_id = resolve_composite_path(tasks, parent_component_path.clone()).await?;

    let parent_str = (!parent_component_path.is_root()).then(|| parent_component_path.to_string());
    let parent_path = parent_str.as_deref();

    let filtered =
        list_filtered_members(tasks, composite_id, &parent_component_path, prefix).await?;

    match filtered.len() {
        0 => {
            match mode {
                CompletionMode::CompositeOnly => {
                    // No members - suggest parent without dot (if current has parent)
                    Ok(Vec::from_iter(
                        parent_path.map(|p| CompletionCandidate::new(p.to_string())),
                    ))
                }
                _ => Ok(vec![]),
            }
        }
        1 => {
            let name = &filtered[0];
            let load_result = tasks
                .run(Goal::LoadMember(composite_id, name.clone(), None))
                .await;
            let full_path = format_path(parent_path, name);

            match load_result.as_ref() {
                Ok(Outcome::MemberLoaded(Ok(ComponentHandle::Composite(_)))) => {
                    match mode {
                        CompletionMode::ResourceOnly => {
                            // Need resource, so must drill down - do so immediately
                            let child_current = format!("{}.", full_path);
                            Box::pin(complete_with_mode(tasks, root_id, &child_current, mode)).await
                        }
                        CompletionMode::AllValid | CompletionMode::CompositeOnly => {
                            // Offer as target and with dot to drill down
                            Ok(vec![
                                CompletionCandidate::new(full_path.clone()),
                                CompletionCandidate::new(format!("{}.", full_path)),
                            ])
                        }
                    }
                }
                Ok(Outcome::MemberLoaded(Ok(ComponentHandle::Resource(_)))) => {
                    match mode {
                        CompletionMode::AllValid | CompletionMode::ResourceOnly => {
                            // Single resource - suggest as final completion
                            Ok(vec![CompletionCandidate::new(full_path)])
                        }
                        CompletionMode::CompositeOnly => {
                            // Not a valid target, fall back to parent
                            if let Some(p) = parent_path {
                                Ok(vec![CompletionCandidate::new(p.to_string())])
                            } else {
                                Ok(vec![])
                            }
                        }
                    }
                }
                _ => Ok(vec![]),
            }
        }
        _multiple => {
            // No need to check kinds because user will tab again before these
            // become final.
            // Later, when the user narrows it down to a single candidate, we check
            // the kind with just one LoadMember call. This results in the best
            // performance.
            Ok(filtered
                .into_iter()
                .map(|name| CompletionCandidate::new(format_path(parent_path, &name)))
                .collect())
        }
    }
}
