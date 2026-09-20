use crate::{
    application,
    interrupt::InterruptState,
    provider,
    work::{get_resource_provider_info, resolve_resource_path, unapplied_resource, READ_ONLY},
    Options,
};
use anyhow::{bail, Context, Result};
use nixops4_core::eval_api::ComponentPath;

/// Destroy a resource, i.e. removing whatever it represents and dropping it from the state.
///
/// Consists of two phases:
/// 1. Read-only eval queries (no `MutationCapability`) to gather provider info,
///    state resource info, and current resource state.
/// 2. Call destroy imperatively, without the work scheduling abstractions
pub(crate) async fn destroy_resource(
    interrupt_state: &InterruptState,
    options: &Options,
    resource_path: &ComponentPath,
) -> Result<()> {
    let resource_path = resource_path.clone();

    application::with_eval(interrupt_state, options, |work_context, tasks| async move {
        // ======== Phase 1: Read-only info gathering ========

        let resource_id =
            resolve_resource_path(&tasks, work_context.root_composite_id, &resource_path).await?;

        let provider_info =
            get_resource_provider_info(&tasks, resource_id, resource_path.clone(), READ_ONLY)
                .await?
                .await
                .with_context(|| format!("Failed to get provider info for {}", resource_path))?;

        // The provider only has to exist to be asked to destroy, and starting
        // it draws on no part of the state, so it starts while the state is
        // read.
        let starting_provider = provider::start_provider(&provider_info);

        // Gather state info and build the ExtantResource for destroy
        let gathering = unapplied_resource(
            &tasks,
            work_context.root_composite_id,
            resource_id,
            &resource_path,
            &provider_info,
        );

        let (gathered, started_provider) = tokio::join!(gathering, starting_provider);
        let (extant_resource, state_handle) = gathered?;

        // ======== Phase 2: Imperative destroy ========

        let mut resource_provider =
            started_provider.context("Failed to start resource provider")?;

        // Destroy the resource through its provider. The provider is closed
        // whether or not that succeeded, because a provider that refused still
        // has to be shut down, and its exit status is the only thing that
        // catches a provider dying rather than answering.
        let destroyed = resource_provider
            .destroy(extant_resource)
            .await
            .with_context(|| format!("Failed to destroy resource {}", resource_path));

        let closed = resource_provider
            .close_wait()
            .await
            .context("Failed to close resource provider")
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    bail!("Resource provider exited with {}", status)
                }
            });

        // The destroy is what the command was asked to do, so its error is the
        // one to report; a bad exit status is only worth raising on its own.
        destroyed?;
        closed?;

        // If stateful, remove resource from state
        if let Some(state_handle) = state_handle {
            state_handle
                .resource_destroy_event(&resource_path)
                .await
                .with_context(|| {
                    format!("Failed to remove resource {} from state", resource_path)
                })?;
        }

        eprintln!("Destroyed resource {}", resource_path);

        Ok(())
    })
    .await
}
