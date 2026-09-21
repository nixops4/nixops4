use crate::{
    control::{
        task_tracker::{Cycle, TaskContext, TaskWork},
        thunk::Thunk,
    },
    eval_client,
    interrupt::InterruptState,
    provider,
};
use anyhow::{bail, Context as _, Result};
use nixops4_core::eval_api::{
    self, AnyType, AssignRequest, ComponentHandle, ComponentPath, ComponentRequest, CompositeType,
    EvalRequest, EvalResponse, Id, IdNum, MessageType, NamedProperty, Property, QueryRequest,
    QueryResponseValue, ResourceProviderInfo, ResourceType, StepResult,
};
use pubsub_rs::Pubsub;
use serde_json::Value;
use std::{collections::BTreeMap, fmt::Display, sync::Arc};
use std::{future::Future, pin::Pin};
use tokio::sync::Mutex;
use tracing::{info_span, Instrument as _};

type CleanupTask = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send>;

/// Capability token that grants permission to perform mutations (create/update/delete).
///
/// This type ensures at compile time that mutation operations can only be
/// performed when explicitly authorized. Goals that perform mutations require
/// this token, while Preview variants allow read-only introspection.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct MutationCapability;

/// How a resource is accounted for.
pub(crate) enum Tracking {
    /// It declares no state, so the declaration is all there is.
    Untracked,
    /// It declares a state resource that holds no record of it yet.
    Unrecorded {
        handle: Arc<crate::state::StateHandle>,
    },
    /// It declares a state resource, and this is what that recorded.
    Recorded {
        handle: Arc<crate::state::StateHandle>,
        recorded: crate::state::ResourceState,
    },
}

/// An item in the deployment preview, representing either a known resource
/// or an unknown structure blocked by a structural dependency.
#[derive(Clone, Debug)]
pub enum PreviewItem {
    /// A resource that will be applied (full path from root)
    Resource(ComponentPath),
    /// A structural dependency that must be resolved before the full structure is known.
    StructuralDependency(StructuralDependency),
}

/// A dependency that occurs not merely between inputs and outputs of resources,
/// but in the structure of the resource graph itself.
///
/// See https://nixops.dev/manual/development/concept/resource.html#structural-dependencies
#[derive(Clone, Debug)]
pub struct StructuralDependency {
    /// The composite path where the unknown structure exists (if known).
    pub path: Option<ComponentPath>,
    /// The resource output this structure depends on.
    pub depends_on: NamedProperty,
}

impl Display for PreviewItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreviewItem::Resource(path) => write!(f, "{}", path),
            PreviewItem::StructuralDependency(dep) => match &dep.path {
                Some(p) if p.is_root() => {
                    write!(
                        f,
                        "*  (structure depends on {}.{})",
                        dep.depends_on.resource, dep.depends_on.name
                    )
                }
                Some(p) => {
                    write!(
                        f,
                        "{}.*  (depends on {}.{})",
                        p, dep.depends_on.resource, dep.depends_on.name
                    )
                }
                None => {
                    write!(
                        f,
                        "*  (depends on {}.{})",
                        dep.depends_on.resource, dep.depends_on.name
                    )
                }
            },
        }
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Goal {
    /// Resolve a component path to a composite ID. Navigates from root.
    /// Used by apply.rs to get the target composite before calling Apply.
    ResolveCompositePath(ComponentPath),
    /// Preview a composite without making changes. Returns all known resources
    /// and any structural dependencies that block full discovery.
    /// INVARIANT: composite_id must be the ID of the composite at composite_path.
    Preview(Id<CompositeType>, ComponentPath),
    /// Apply a composite, creating/updating/deleting resources as needed.
    /// INVARIANT: composite_id must be the ID of the composite at composite_path.
    Apply(Id<CompositeType>, ComponentPath, MutationCapability),
    /// List member names in a composite. If listing has a structural dependency, resolves it.
    /// Option<MutationCapability>: Some = retry on dependency (apply), None = return dependency (preview)
    /// Note: composite_path is only used for error reporting; composite_id is the source of truth.
    ListMembers(Id<CompositeType>, ComponentPath, Option<MutationCapability>),
    /// Assign an ID to a member component. Sends LoadComponent to eval, memoized.
    /// This is the fine-grained memoizable unit - no mutation_cap so same ID is reused.
    AssignMemberId(Id<CompositeType>, String),
    /// Load a member, potentially resolving dependencies. Defers to AssignMemberId for the ID.
    LoadMember(Id<CompositeType>, String, Option<MutationCapability>),
    /// Get resource provider info for a loaded resource.
    /// Note: path is only used for logging; id is the source of truth.
    GetResourceProviderInfo(Id<ResourceType>, ComponentPath, Option<MutationCapability>),
    /// List resource inputs.
    /// Note: path is only used for logging; id is the source of truth.
    ListResourceInputs(Id<ResourceType>, ComponentPath, Option<MutationCapability>),
    /// Get a specific resource input value.
    /// Note: path is only used for logging; id is the source of truth.
    GetResourceInputValue(
        Id<ResourceType>,
        ComponentPath,
        String,
        Option<MutationCapability>,
    ),
    /// Get a resource output value (goes to ApplyResource).
    /// Note: path is only used for logging; id is the source of truth.
    GetResourceOutputValue(Id<ResourceType>, ComponentPath, String, MutationCapability),
    /// Apply a single resource.
    /// Note: path is only used for logging; id is the source of truth.
    ApplyResource(Id<ResourceType>, ComponentPath, MutationCapability),
    /// Run a state provider resource.
    /// Note: path is only used for logging; id is the source of truth.
    RunState(Id<ResourceType>, ComponentPath, Option<MutationCapability>),
}
impl Display for Goal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Goal::ResolveCompositePath(path) => {
                write!(f, "Resolve composite path {}", path)
            }
            Goal::Preview(_id, path) => {
                write!(f, "Preview composite {}", path)
            }
            Goal::Apply(_id, path, _cap) => {
                write!(f, "Apply composite {}", path)
            }
            Goal::ListMembers(_id, path, cap) => {
                if cap.is_some() {
                    write!(f, "List members in composite {}", path)
                } else {
                    write!(f, "List members (preview) in composite {}", path)
                }
            }
            Goal::AssignMemberId(parent_id, name) => {
                write!(
                    f,
                    "Assign ID for member '{}' in composite {}",
                    name,
                    parent_id.num()
                )
            }
            Goal::LoadMember(parent_id, name, _cap) => {
                write!(
                    f,
                    "Load member '{}' from composite {}",
                    name,
                    parent_id.num()
                )
            }
            Goal::GetResourceProviderInfo(_, path, _cap) => {
                write!(f, "Get resource provider info for {}", path)
            }
            Goal::ListResourceInputs(_, path, _cap) => {
                write!(f, "List resource inputs for {}", path)
            }
            Goal::GetResourceInputValue(_, path, input, _cap) => {
                write!(
                    f,
                    "Get resource input value for resource {} input {}",
                    path, input
                )
            }
            Goal::GetResourceOutputValue(_, path, property, _cap) => {
                write!(
                    f,
                    "Get resource output value from resource {} property {}",
                    path, property
                )
            }
            Goal::ApplyResource(_, path, _cap) => {
                write!(f, "Apply resource {}", path)
            }
            Goal::RunState(_, path, _cap) => {
                write!(f, "Run state provider resource {}", path)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum Outcome {
    Done(),
    /// Composite ID resolved from a path
    CompositeResolved(Id<CompositeType>),
    /// Preview result containing all known resources and structural dependencies
    Preview(Vec<PreviewItem>),
    /// Member names listed, or blocked by structural dependency (for preview)
    MembersListed(Result<Vec<String>, StructuralDependency>),
    /// An ID assigned to a member (from AssignMemberId)
    MemberIdAssigned(Id<AnyType>),
    /// A single member loaded, or blocked by structural dependency (for preview)
    MemberLoaded(Result<ComponentHandle, StructuralDependency>),
    ResourceProviderInfo(eval_api::ResourceProviderInfo),
    ResourceInputsListed(Vec<String>),
    ResourceInputValue(Value),
    ResourceOutputValue, /*Value ignored because passed eagerly before ResourceOutputs, a dependency of this.*/
    ResourceOutputs(nixops4_resource::schema::v0::ExtantResource),
    RunState(Arc<crate::state::StateHandle>),
}

pub struct WorkContext {
    pub options: crate::options::Options,
    pub eval_sender: eval_client::EvalSender,
    pub root_composite_id: Id<CompositeType>,
    pub interrupt_state: InterruptState,
    pub state: Mutex<WorkState>,
    pub id_subscriptions: Pubsub<IdNum, EvalResponse>,
}
#[derive(Default)]
pub struct WorkState {
    cleanup_tasks: Vec<CleanupTask>,
}

impl WorkContext {
    pub async fn clean_up_state_providers(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        let cleanup_tasks = std::mem::take(&mut state.cleanup_tasks);
        drop(state); // Release lock before executing tasks

        let mut errors = Vec::new();
        for cleanup_task in cleanup_tasks {
            let future = cleanup_task();
            if let Err(e) = future.await {
                errors.push(e);
            }
        }

        if !errors.is_empty() {
            let error_messages: Vec<String> = errors.iter().map(|e| format!("{:#}", e)).collect();
            bail!(
                "Failed to close {} state provider(s):\n\n{}",
                errors.len(),
                error_messages.join("\n\n======== NEXT PROVIDER FAILURE ========\n")
            );
        }

        Ok(())
    }

    /// List member names via ListMembers goal.
    async fn list_member_names(
        &self,
        context: &TaskContext<Self>,
        composite_id: Id<CompositeType>,
        composite_path: &ComponentPath,
        mutation_cap: Option<MutationCapability>,
    ) -> Result<Result<Vec<String>, StructuralDependency>> {
        list_members(context, composite_id, composite_path.clone(), mutation_cap)
            .await
            .with_context(|| format!("Error while listing members of {}", composite_path))?
            .await
    }

    /// Load a single member via LoadMember goal.
    async fn load_member(
        &self,
        context: &TaskContext<Self>,
        composite_id: Id<CompositeType>,
        name: &str,
        mutation_cap: Option<MutationCapability>,
    ) -> Result<ComponentHandle> {
        match load_member(context, composite_id, name.to_string(), mutation_cap)
            .await
            .with_context(|| format!("Error while loading member '{}'", name))?
            .await?
        {
            Ok(handle) => Ok(handle),
            Err(dep) => bail!(
                "Structural dependency while loading member '{}' (depends on {}.{})",
                name,
                dep.depends_on.resource,
                dep.depends_on.name,
            ),
        }
    }

    /// Load all members in parallel and partition by kind, preserving typed IDs.
    /// Returns Err(PreviewItem) if any member has a structural dependency (preview mode).
    async fn load_and_partition_members(
        &self,
        context: &TaskContext<Self>,
        composite_id: Id<CompositeType>,
        names: Vec<String>,
        mutation_cap: Option<MutationCapability>,
    ) -> Result<
        Result<
            (
                BTreeMap<String, Id<ResourceType>>,
                BTreeMap<String, Id<CompositeType>>,
            ),
            StructuralDependency,
        >,
    > {
        // Ask for all of them before awaiting any, so that they load in parallel
        let mut loading = BTreeMap::new();
        for name in names {
            let member = load_member(context, composite_id, name.clone(), mutation_cap).await?;
            loading.insert(name, member);
        }

        // Await all and partition by kind, keeping typed IDs
        let mut resources = BTreeMap::new();
        let mut composites = BTreeMap::new();
        for (name, member) in loading {
            let handle = match member.await? {
                Ok(handle) => handle,
                // Structural dependency - return it
                Err(dep) => return Ok(Err(dep)),
            };
            match handle {
                ComponentHandle::Resource(id) => {
                    resources.insert(name, id);
                }
                ComponentHandle::Composite(id) => {
                    composites.insert(name, id);
                }
            }
        }
        Ok(Ok((resources, composites)))
    }

    /// List all members of a composite, partitioned into resources and composites.
    /// With mutation_cap Some: resolves structural dependencies.
    /// With mutation_cap None: returns structural dependency as preview item.
    async fn list_members_partitioned(
        &self,
        context: &TaskContext<Self>,
        composite_id: Id<CompositeType>,
        composite_path: &ComponentPath,
        mutation_cap: Option<MutationCapability>,
    ) -> Result<
        Result<
            (
                BTreeMap<ComponentPath, Id<ResourceType>>,
                BTreeMap<String, Id<CompositeType>>,
            ),
            StructuralDependency,
        >,
    > {
        let names = match self
            .list_member_names(context, composite_id, composite_path, mutation_cap)
            .await?
        {
            Ok(names) => names,
            Err(item) => return Ok(Err(item)),
        };

        let (resources, composites) = match self
            .load_and_partition_members(context, composite_id, names, mutation_cap)
            .await?
        {
            Ok(partitioned) => partitioned,
            Err(dep) => return Ok(Err(dep)),
        };

        let resources_with_paths = resources
            .into_iter()
            .map(|(name, id)| (composite_path.child(name), id))
            .collect();

        Ok(Ok((resources_with_paths, composites)))
    }

    /// Helper to get a composite ID from a path.
    /// Recursively resolves parent paths via LoadMember.
    async fn get_composite_id(
        &self,
        context: &TaskContext<Self>,
        composite_path: &ComponentPath,
    ) -> Result<Id<CompositeType>> {
        if composite_path.is_root() {
            return Ok(self.root_composite_id);
        }
        let (parent_path, name) = composite_path.parent().unwrap();
        let parent_id = Box::pin(self.get_composite_id(context, &parent_path)).await?;
        let handle = self.load_member(context, parent_id, name, None).await?;
        match handle {
            ComponentHandle::Composite(id) => Ok(id),
            ComponentHandle::Resource(_) => {
                bail!(
                    "Expected composite at {}, but found resource",
                    composite_path
                )
            }
        }
    }

    /// Helper to get a resource ID from a path.
    async fn get_resource_id(
        &self,
        context: &TaskContext<Self>,
        resource_path: &ComponentPath,
    ) -> Result<Id<ResourceType>> {
        let (parent_path, name) = resource_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Cannot get resource ID for root path"))?;
        let parent_id = self.get_composite_id(context, &parent_path).await?;
        let handle = self.load_member(context, parent_id, name, None).await?;
        match handle {
            ComponentHandle::Resource(id) => Ok(id),
            ComponentHandle::Composite(_) => {
                bail!(
                    "Expected resource at {}, but found composite",
                    resource_path
                )
            }
        }
    }

    /// Open the state resource that `resource_path` declares, and read what it
    /// records about it.
    ///
    /// With a `mutation_cap` the state resource is applied before it is
    /// opened, because a state resource is a resource like any other and may
    /// not exist yet, or need to be updated.
    async fn tracking(
        &self,
        context: &TaskContext<Self>,
        resource_path: &ComponentPath,
        provider_info: &ResourceProviderInfo,
        mutation_cap: Option<MutationCapability>,
    ) -> Result<Tracking> {
        let state_resource_path = match &provider_info.state {
            Some(path) => path,
            None => return Ok(Tracking::Untracked),
        };

        let state_resource_id = self
            .get_resource_id(context, state_resource_path)
            .await
            .with_context(|| {
                format!(
                    "Invalid state reference '{}' for resource {}",
                    state_resource_path, resource_path
                )
            })?;

        let state = run_state(
            context,
            state_resource_id,
            state_resource_path.clone(),
            mutation_cap,
        )
        .await
        .with_context(|| {
            format!(
                "Failed to start state provider '{}' for resource {}",
                state_resource_path, resource_path
            )
        })?;

        let handle = state.await?;

        Ok(match handle.recorded(resource_path).await {
            Some(recorded) => Tracking::Recorded { handle, recorded },
            None => Tracking::Unrecorded { handle },
        })
    }

    /// Resolve a dependency by requiring the resource output value.
    async fn resolve_dependency(
        &self,
        context: &TaskContext<Self>,
        dep: &NamedProperty,
        mutation_cap: Option<MutationCapability>,
    ) -> Result<()> {
        // TODO: pass the command name (e.g. "state dump") into the error
        // so users know which invocation triggered read-only mode.
        let cap = mutation_cap.with_context(|| {
            format!(
                "the current command uses the deployment in read-only mode, \
                 but this depends on output {}.{}, which requires applying \
                 that resource first",
                dep.resource, dep.name
            )
        })?;
        let dep_id = self.get_resource_id(context, &dep.resource).await?;
        let r = context
            .require(Goal::GetResourceOutputValue(
                dep_id,
                dep.resource.clone(),
                dep.name.clone(),
                cap,
            ))
            .await
            .with_context(|| format!("resolving dependency {}.{}", dep.resource, dep.name))?;
        clone_result(&r)?;
        Ok(())
    }

    /// Low-level helper to send ListMembers request and receive response.
    async fn eval_list_members(
        &self,
        composite_id: Id<CompositeType>,
    ) -> Result<StepResult<Vec<String>>> {
        let msg_id = self.eval_sender.next_id();
        let rx = self.id_subscriptions.subscribe(vec![msg_id.num()]).await;
        self.eval_sender
            .query(msg_id, EvalRequest::ListMembers, composite_id)
            .await?;

        loop {
            let (_id, r) = rx
                .recv()
                .await
                .context("waiting for ListMembers response")?;
            match r {
                EvalResponse::Error(_id, e) => bail!("Evaluation error: {}", e),
                EvalResponse::QueryResponse(_id, query_response_value) => {
                    match query_response_value {
                        QueryResponseValue::ListMembers(step_result) => {
                            return Ok(step_result);
                        }
                        _ => bail!(
                            "Unexpected response to ListMembers, {:?}",
                            query_response_value
                        ),
                    }
                }
                EvalResponse::TracingEvent(_) => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // NOTE: perform_* functions should only be called from the work() function
    // and should not be called directly, so that we can ensure that work is
    // deduplicated and that we don't have cycles.
    // -----------------------------------------------------------------------

    /// Resolve a component path to a composite ID by navigating from root.
    async fn perform_resolve_composite_path(
        &self,
        context: TaskContext<Self>,
        path: ComponentPath,
    ) -> Result<Outcome> {
        let id = self.get_composite_id(&context, &path).await?;
        Ok(Outcome::CompositeResolved(id))
    }

    /// Preview a composite, collecting all known resources and structural dependencies.
    /// INVARIANT: composite_id must be the ID of the composite at composite_path.
    async fn perform_preview(
        &self,
        context: TaskContext<Self>,
        composite_id: Id<CompositeType>,
        composite_path: ComponentPath,
    ) -> Result<Outcome> {
        let mut items = Vec::new();

        // List and partition members (preview mode: returns structural dependency if blocked)
        let (resources, nested_composites) = match self
            .list_members_partitioned(&context, composite_id, &composite_path, None)
            .await?
        {
            Ok(partitioned) => partitioned,
            Err(structural_dep) => {
                items.push(PreviewItem::StructuralDependency(structural_dep));
                // Can't continue if we don't know the structure
                return Ok(Outcome::Preview(items));
            }
        };

        // Add resources to preview items
        for path in resources.keys() {
            items.push(PreviewItem::Resource(path.clone()));
        }

        // Recursively preview nested composites
        let mut nested_thunks = Vec::new();
        for (name, nested_id) in &nested_composites {
            let nested_path = composite_path.child(name.clone());
            nested_thunks.push(preview(&context, *nested_id, nested_path).await?);
        }

        // Collect results from nested composites
        for nested in nested_thunks {
            items.extend(nested.await?);
        }

        Ok(Outcome::Preview(items))
    }

    /// INVARIANT: composite_id must be the ID of the composite at composite_path.
    async fn perform_apply(
        &self,
        context: TaskContext<Self>,
        composite_id: Id<CompositeType>,
        composite_path: ComponentPath,
        mutation_cap: MutationCapability,
    ) -> Result<Outcome> {
        // First, preview to get all known resources and structural dependencies
        let preview_items = preview(&context, composite_id, composite_path.clone())
            .await?
            .await?;

        // Print preview items grouped together
        if preview_items.is_empty() {
            eprintln!("Composite contains no resources; nothing to apply.");
        } else {
            eprintln!("The following resources (and nested composites, if any) will be applied:");
            for item in &preview_items {
                eprintln!("  - {}", item);
            }
        }
        self.interrupt_state.check_interrupted()?;

        // List and partition members, resolving structural dependencies
        // IDs come directly from LoadMember - no separate lookup needed
        let (resources, nested_composites) = self
            .list_members_partitioned(&context, composite_id, &composite_path, Some(mutation_cap))
            .await?
            .expect("structural dependencies should be resolved with mutation_cap");

        // Apply nested composites
        let mut nested_thunks = Vec::new();
        for (name, nested_id) in &nested_composites {
            let nested_path = composite_path.child(name.clone());
            nested_thunks.push(apply(&context, *nested_id, nested_path, mutation_cap).await?);
        }

        // Apply resources (IDs already available from partitioning)
        let mut applying = BTreeMap::new();
        for (resource_path, id) in &resources {
            let resource =
                apply_resource(&context, *id, resource_path.clone(), mutation_cap).await?;
            applying.insert(*id, resource);
        }

        let mut resource_map = BTreeMap::new();
        for (id, resource) in applying {
            resource_map.insert(id, resource.await?);
        }

        // Wait for nested composites to complete
        for nested in nested_thunks {
            nested.await?;
        }

        // TODO: this application logic doesn't belong here

        eprintln!("The following resources were created:");

        // This is of questionable value, and we should probably only print values that are explicitly requested.
        for (resource_path, resource_id) in &resources {
            if let Some(resource) = resource_map.get(resource_id) {
                eprintln!("  - resource {}", resource_path);
                for (k, v) in resource.input_properties.iter() {
                    eprintln!("    - input {}: {}", k, indented_json(v));
                }
                if let Some(output_properties) = &resource.output_properties {
                    for (k, v) in output_properties.iter() {
                        eprintln!("    - output {}: {}", k, indented_json(v));
                    }
                }
            }
        }

        Ok(Outcome::Done())
    }

    /// List member names in a composite, retrying on dependency.
    /// See [`StepResult::Needs`] for caching/retry semantics.
    /// With mutation_cap None: returns dependency as preview item.
    async fn perform_list_members(
        &self,
        context: TaskContext<Self>,
        composite_id: Id<CompositeType>,
        composite_path: ComponentPath,
        mutation_cap: Option<MutationCapability>,
    ) -> Result<Outcome> {
        loop {
            let step = self.eval_list_members(composite_id).await?;
            match step {
                StepResult::Done(names) => {
                    return Ok(Outcome::MembersListed(Ok(names)));
                }
                StepResult::Needs(dep) => match mutation_cap {
                    Some(cap) => self.resolve_dependency(&context, &dep, Some(cap)).await?,
                    None => {
                        return Ok(Outcome::MembersListed(Err(StructuralDependency {
                            path: Some(composite_path),
                            depends_on: dep,
                        })));
                    }
                },
            }
        }
    }

    /// Assign an ID to a member and kick off component loading.
    ///
    /// This is memoized to ensure stable IDs across Preview and Apply.
    /// Sends AssignMember to start evaluation early; LoadMember will then
    /// use GetComponentKind to wait for the result.
    async fn perform_assign_member_id(
        &self,
        _context: TaskContext<Self>,
        parent_id: Id<CompositeType>,
        name: String,
    ) -> Result<Outcome> {
        let id: Id<AnyType> = self.eval_sender.next_id();

        // Fire and forget - kick off evaluation early
        self.eval_sender
            .send(&EvalRequest::AssignMember(AssignRequest {
                assign_to: id,
                payload: ComponentRequest {
                    parent: parent_id,
                    name,
                },
            }))
            .await?;

        Ok(Outcome::MemberIdAssigned(id))
    }

    /// Send GetComponentKind request and wait for response.
    async fn eval_get_component_kind(
        &self,
        id: Id<AnyType>,
    ) -> Result<StepResult<ComponentHandle>> {
        let message_id: Id<MessageType> = self.eval_sender.next_id();
        let rx = self
            .id_subscriptions
            .subscribe(vec![message_id.num()])
            .await;
        self.eval_sender
            .send(&EvalRequest::GetComponentKind(QueryRequest::new(
                message_id, id,
            )))
            .await?;

        // Wait for ComponentKind response
        loop {
            let (_id, r) = rx
                .recv()
                .await
                .context("waiting for GetComponentKind response")?;
            match r {
                EvalResponse::Error(_id, e) => bail!("while listing resources: {}", e),
                EvalResponse::QueryResponse(_id, query_response_value) => {
                    match query_response_value {
                        QueryResponseValue::ComponentKind(step_result) => {
                            return Ok(step_result);
                        }
                        _ => bail!("Unexpected response type for GetComponentKind"),
                    }
                }
                EvalResponse::TracingEvent(_) => {}
            }
        }
    }

    /// Load a member, using AssignMemberId for stable ID assignment.
    /// See [`StepResult::Needs`] for caching/retry semantics.
    /// With mutation_cap None: returns dependency as error (for preview).
    async fn perform_load_member(
        &self,
        context: TaskContext<Self>,
        parent_id: Id<CompositeType>,
        name: String,
        mutation_cap: Option<MutationCapability>,
    ) -> Result<Outcome> {
        // Get stable ID from AssignMemberId (also kicks off eval)
        let id = assign_member_id(&context, parent_id, name.clone())
            .await?
            .await?;

        loop {
            let step = self.eval_get_component_kind(id).await?;
            match step {
                StepResult::Done(handle) => {
                    return Ok(Outcome::MemberLoaded(Ok(handle)));
                }
                StepResult::Needs(dep) => match mutation_cap {
                    Some(cap) => self.resolve_dependency(&context, &dep, Some(cap)).await?,
                    None => {
                        return Ok(Outcome::MemberLoaded(Err(StructuralDependency {
                            path: None,
                            depends_on: dep,
                        })));
                    }
                },
            }
        }
    }

    /// Low-level helper to send GetResource request and receive response.
    async fn eval_get_resource_provider_info(
        &self,
        id: Id<ResourceType>,
    ) -> Result<StepResult<ResourceProviderInfo>> {
        let msg_id = self.eval_sender.next_id();
        let rx = self.id_subscriptions.subscribe(vec![msg_id.num()]).await;
        self.eval_sender
            .query(msg_id, EvalRequest::GetResource, id)
            .await?;

        loop {
            let (_id, r) = rx
                .recv()
                .await
                .context("waiting for GetResourceProviderInfo response")?;
            match r {
                EvalResponse::Error(_id, e) => bail!("while getting provider info: {}", e),
                EvalResponse::QueryResponse(_id, query_response_value) => {
                    match query_response_value {
                        QueryResponseValue::ResourceProviderInfo(step_result) => {
                            break Ok(step_result);
                        }
                        _ => bail!(
                            "Unexpected response to GetResourceProviderInfo, {:?}",
                            query_response_value
                        ),
                    }
                }
                EvalResponse::TracingEvent(_) => {}
            }
        }
    }

    /// Get resource provider info, retrying on dependency.
    /// See [`StepResult::Needs`] for caching/retry semantics.
    /// With mutation_cap None: errors on unresolved dependency instead of resolving it.
    async fn perform_get_resource_provider_info(
        &self,
        context: TaskContext<Self>,
        id: Id<ResourceType>,
        _resource_path: ComponentPath,
        mutation_cap: Option<MutationCapability>,
    ) -> Result<Outcome> {
        loop {
            match self.eval_get_resource_provider_info(id).await? {
                StepResult::Done(info) => return Ok(Outcome::ResourceProviderInfo(info)),
                StepResult::Needs(dep) => {
                    self.resolve_dependency(&context, &dep, mutation_cap)
                        .await?;
                }
            }
        }
    }

    /// Low-level helper to send ListResourceInputs request and receive response.
    async fn eval_list_resource_inputs(
        &self,
        id: Id<ResourceType>,
    ) -> Result<StepResult<Vec<String>>> {
        let msg_id = self.eval_sender.next_id();
        let rx = self.id_subscriptions.subscribe(vec![msg_id.num()]).await;
        self.eval_sender
            .query(msg_id, EvalRequest::ListResourceInputs, id)
            .await?;

        loop {
            let (_id, r) = rx
                .recv()
                .await
                .context("waiting for ListResourceInputs response")?;
            match r {
                EvalResponse::Error(_id, e) => bail!("while listing resource inputs: {}", e),
                EvalResponse::QueryResponse(_id, query_response_value) => {
                    match query_response_value {
                        QueryResponseValue::ListResourceInputs(step_result) => {
                            break Ok(step_result);
                        }
                        _ => bail!(
                            "Unexpected response to ListResourceInputs, {:?}",
                            query_response_value
                        ),
                    }
                }
                EvalResponse::TracingEvent(_) => {}
            }
        }
    }

    /// List resource inputs, retrying on dependency.
    /// See [`StepResult::Needs`] for caching/retry semantics.
    /// With mutation_cap None: errors on unresolved dependency instead of resolving it.
    async fn perform_list_resource_inputs(
        &self,
        context: TaskContext<Self>,
        id: Id<ResourceType>,
        _resource_path: ComponentPath,
        mutation_cap: Option<MutationCapability>,
    ) -> Result<Outcome> {
        loop {
            match self.eval_list_resource_inputs(id).await? {
                StepResult::Done(input_names) => {
                    return Ok(Outcome::ResourceInputsListed(input_names))
                }
                StepResult::Needs(dep) => {
                    self.resolve_dependency(&context, &dep, mutation_cap)
                        .await?;
                }
            }
        }
    }

    /// With mutation_cap None: bails on structural dependency (read-only mode).
    async fn perform_get_resource_input_value(
        &self,
        context: TaskContext<Self>,
        id: Id<ResourceType>,
        _resource_path: ComponentPath,
        input_name: String,
        mutation_cap: Option<MutationCapability>,
    ) -> Result<Outcome> {
        let msg_id = self.eval_sender.next_id();
        let rx = self.id_subscriptions.subscribe(vec![msg_id.num()]).await;
        loop {
            self.eval_sender
                .query(
                    msg_id,
                    EvalRequest::GetResourceInput,
                    Property {
                        resource: id,
                        name: input_name.clone(),
                    },
                )
                .await?;

            let (_id, r) = rx
                .recv()
                .await
                .context("waiting for GetResourceInputValue response")?;
            match r {
                EvalResponse::Error(_id, e) => bail!(
                    "while evaluating resource input value {}: {}",
                    input_name,
                    e
                ),
                EvalResponse::QueryResponse(_id, query_response_value) => {
                    match query_response_value {
                        QueryResponseValue::ResourceInputValue(step_result) => match step_result {
                            StepResult::Done(value) => {
                                break Ok(Outcome::ResourceInputValue(value))
                            }
                            StepResult::Needs(dep) => {
                                self.resolve_dependency(&context, &dep, mutation_cap)
                                    .await?;
                            }
                        },
                        _ => bail!(
                            "Unexpected response to GetResourceInputValue, {:?}",
                            query_response_value
                        ),
                    }
                }
                EvalResponse::TracingEvent(_) => {}
            }
        }
    }

    async fn perform_get_resource_output_value(
        &self,
        context: TaskContext<Self>,
        id: Id<ResourceType>,
        resource_path: ComponentPath,
        property: String,
        mutation_cap: MutationCapability,
    ) -> Result<Outcome> {
        // Apply the resource
        let extant_resource = apply_resource(&context, id, resource_path.clone(), mutation_cap)
            .await?
            .await?;
        // ExtantResource is the provider protocol type for a resource both before
        // and after apply, so output_properties stays an Option here even
        // though, having just applied, it can only be None if the provider
        // itself sent no outputs.
        match &extant_resource.output_properties {
            Some(output_properties) => {
                if output_properties.contains_key(&property) {
                    Ok(Outcome::ResourceOutputValue)
                } else {
                    bail!(
                        "Resource {} does not have output {}",
                        resource_path,
                        property
                    )
                }
            }
            None => bail!("Resource {} has no output properties", resource_path),
        }
    }

    /// Spawn tasks to resolve provider info and input values for a resource.
    /// Returns provider info (forced) and a thunk for the input map (not yet
    /// forced), so callers can spawn further work before forcing the inputs.
    async fn resolve_resource_inputs(
        &self,
        context: &TaskContext<Self>,
        id: Id<ResourceType>,
        resource_path: &ComponentPath,
        mutation_cap: Option<MutationCapability>,
    ) -> Result<(
        ResourceProviderInfo,
        Thunk<std::result::Result<serde_json::Map<String, serde_json::Value>, Arc<anyhow::Error>>>,
    )> {
        // Both requests go out before either answer is awaited.
        let provider_info =
            get_resource_provider_info(context, id, resource_path.clone(), mutation_cap).await?;
        let value_futs =
            request_resource_input_values(context, id, resource_path.clone(), mutation_cap).await?;

        let provider_info = provider_info.await?;

        // Returned unforced, like the request above, so that a caller can
        // overlap its own work with the values arriving. The apply path
        // starts the resource's state resource in the meantime.
        let inputs_thunk = Thunk::new(async move {
            let mut inputs = serde_json::Map::new();
            for (name, value_fut) in value_futs {
                match value_fut.await {
                    Ok(value) => {
                        inputs.insert(name, value);
                    }
                    Err(e) => return Err(Arc::new(e)),
                }
            }
            Ok(inputs)
        });

        Ok((provider_info, inputs_thunk))
    }

    /// Send resource outputs to the evaluator and return the final `ExtantResource`.
    async fn publish_resource_outputs(
        &self,
        resource_path: &ComponentPath,
        inputs: serde_json::Map<String, serde_json::Value>,
        outputs: serde_json::Map<String, serde_json::Value>,
        resource_type: String,
    ) -> Result<Outcome> {
        if self.options.verbose {
            eprintln!("Resource outputs: {:?}", outputs);
        }

        // Send the outputs eagerly, to avoid roundtrips and costly
        // re-evaluation when some output is "missing" but not really
        for (output_name, output_value) in outputs.iter() {
            let output_prop = NamedProperty {
                resource: resource_path.clone(),
                name: output_name.clone(),
            };
            self.eval_sender
                .send(&EvalRequest::PutResourceOutput(
                    output_prop,
                    output_value.clone(),
                ))
                .await?;
        }

        let resource = nixops4_resource::schema::v0::ExtantResource {
            input_properties: nixops4_resource::schema::v0::InputProperties(inputs),
            output_properties: Some(nixops4_resource::schema::v0::OutputProperties(outputs)),
            type_: nixops4_resource::schema::v0::ResourceType(resource_type),
        };

        Ok(Outcome::ResourceOutputs(resource))
    }

    async fn perform_apply_resource(
        &self,
        context: TaskContext<Self>,
        id: Id<ResourceType>,
        resource_path: ComponentPath,
        mutation_cap: MutationCapability,
    ) -> Result<Outcome> {
        let (provider_info, inputs_thunk) = self
            .resolve_resource_inputs(&context, id, &resource_path, Some(mutation_cap))
            .await?;

        let (state, mut saved_past_resource) = match self
            .tracking(&context, &resource_path, &provider_info, Some(mutation_cap))
            .await?
        {
            Tracking::Untracked => (None, None),
            Tracking::Unrecorded { handle } => (Some(handle), None),
            Tracking::Recorded { handle, recorded } => (Some(handle), Some(recorded)),
        };

        let inputs = clone_result(inputs_thunk.force().await)?;

        // Skip optimization: see "Skipping Unchanged Resources" in
        // doc/manual/src/concept/resource.md for the assumptions.
        if let Some(ref past_resource) = saved_past_resource {
            if past_resource.type_ != provider_info.resource_type {
                bail!(
                    "Resource type change is not supported: {} != {}",
                    past_resource.type_,
                    provider_info.resource_type
                );
            }
            if inputs == past_resource.input_properties {
                tracing::info!(
                    "Skipping update for resource {}: inputs unchanged",
                    resource_path
                );
                return self
                    .publish_resource_outputs(
                        &resource_path,
                        inputs,
                        past_resource.output_properties.clone(),
                        provider_info.resource_type,
                    )
                    .await;
            }
        }

        let span = info_span!(
            "creating resource",
            name = resource_path.to_string().as_str()
        );

        if self.options.verbose {
            eprintln!(
                "Provider details for {}: {:?}",
                resource_path, provider_info
            );
            eprintln!("Resource inputs for {}: {:?}", resource_path, inputs);
        }

        let mut provider = provider::start_provider(&provider_info).await?;

        let outputs = match state {
            None => provider
                .create(provider_info.resource_type.as_str(), &inputs, false)
                .await
                .with_context(|| {
                    format!("Failed to create stateless resource {}", resource_path)
                })?,
            Some(ref state_handle) => {
                match saved_past_resource.take() {
                    Some(past_resource) => {
                        // Type check already done above; inputs differ if we reach here
                        tracing::debug!("Updating resource {}: inputs changed", resource_path);
                        let outputs = provider
                            .update(
                                provider_info.resource_type.as_str(),
                                &inputs,
                                &past_resource.input_properties,
                                &past_resource.output_properties,
                            )
                            .await
                            .with_context(|| {
                                format!("Failed to update resource {}", resource_path)
                            })?;
                        let current_resource = crate::state::ResourceState {
                            type_: provider_info.resource_type.clone(),
                            input_properties: inputs.clone(),
                            output_properties: outputs.clone(),
                        };
                        if current_resource != past_resource {
                            state_handle
                                .resource_event(
                                    &resource_path,
                                    "update",
                                    Some(&past_resource),
                                    &current_resource,
                                )
                                .await?;
                        }
                        outputs
                    }
                    None => {
                        let outputs = provider
                            .create(provider_info.resource_type.as_str(), &inputs, true)
                            .await
                            .with_context(|| {
                                format!("Failed to create resource {}", resource_path)
                            })?;
                        let current_resource = crate::state::ResourceState {
                            type_: provider_info.resource_type.clone(),
                            input_properties: inputs.clone(),
                            output_properties: outputs.clone(),
                        };
                        state_handle
                            .resource_event(&resource_path, "create", None, &current_resource)
                            .await?;
                        outputs
                    }
                }
            }
        };

        drop(span);

        // Close the provider properly
        // We might want to reuse them in the future, but for now we launch one
        // per resource, except when it's a state provider (elsewhere).
        provider.close_wait().await?;

        self.publish_resource_outputs(&resource_path, inputs, outputs, provider_info.resource_type)
            .await
    }

    /// Open the state resource at `path`, so that the state it holds can be
    /// read and written.
    ///
    /// With a `MutationCapability` the resource is applied first, because a
    /// state resource is a resource like any other and may not exist yet.
    /// Without one it stays unapplied, so its declaration is the only
    /// account of it available here.
    async fn perform_run_state(
        &self,
        context: TaskContext<Self>,
        id: Id<ResourceType>,
        path: ComponentPath,
        cap: Option<MutationCapability>,
    ) -> Result<Outcome> {
        let (resource, provider_info) = match cap {
            Some(cap) => {
                // Mutating path: apply the resource first
                let resource = apply_resource(&context, id, path.clone(), cap)
                    .await
                    .with_context(|| format!("Failed to apply resource {}", path))?
                    .await?;

                let provider_info =
                    get_resource_provider_info(&context, id, path.clone(), Some(cap))
                        .await
                        .with_context(|| {
                            format!("Failed to get provider info for resource {}", path)
                        })?
                        .await?;

                (resource, provider_info)
            }
            None => {
                // Read-only path: resolve inputs without applying
                let (provider_info, inputs_thunk) = self
                    .resolve_resource_inputs(&context, id, &path, None)
                    .await?;
                let inputs = clone_result(inputs_thunk.force().await)?;
                let resource = nixops4_resource::schema::v0::ExtantResource {
                    input_properties: nixops4_resource::schema::v0::InputProperties(inputs),
                    output_properties: None,
                    type_: nixops4_resource::schema::v0::ResourceType(
                        provider_info.resource_type.clone(),
                    ),
                };

                (resource, provider_info)
            }
        };

        let handle = crate::state::StateHandle::open(&provider_info, &resource).await?;

        // Register cleanup task for this state handle
        {
            let handle_for_cleanup = handle.clone();
            let cleanup_task = Box::new(
                move || -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
                    Box::pin(async move { handle_for_cleanup.close().await })
                },
            );
            self.state.lock().await.cleanup_tasks.push(cleanup_task);
        }

        Ok(Outcome::RunState(handle))
    }
}

impl TaskWork for WorkContext {
    type Output = Arc<std::result::Result<Outcome, Arc<anyhow::Error>>>;

    type Key = Goal;

    type CycleError = anyhow::Error;

    fn cycle_error(&self, cycle: Cycle<Self::Key>) -> Self::CycleError {
        if self.options.verbose {
            anyhow::anyhow!("Cycle detected: {:?}", cycle)
        } else {
            anyhow::anyhow!("Cycle detected: {}", cycle)
        }
    }

    async fn work(&self, context: TaskContext<Self>, key: Self::Key) -> Self::Output {
        let r = match key {
            Goal::ResolveCompositePath(path) => {
                self.perform_resolve_composite_path(context, path)
                    .instrument(info_span!("Resolving composite path"))
                    .await
            }
            Goal::Apply(composite_id, composite_path, mutation_cap) => {
                self.perform_apply(context, composite_id, composite_path, mutation_cap)
                    .instrument(info_span!("Applying composite"))
                    .await
            }
            Goal::ListMembers(composite_id, composite_path, mutation_cap) => {
                self.perform_list_members(context, composite_id, composite_path, mutation_cap)
                    .instrument(info_span!("Listing members"))
                    .await
            }
            Goal::AssignMemberId(parent_id, name) => {
                self.perform_assign_member_id(context, parent_id, name)
                    .instrument(info_span!("Assigning member ID"))
                    .await
            }
            Goal::LoadMember(parent_id, name, mutation_cap) => {
                self.perform_load_member(context, parent_id, name, mutation_cap)
                    .instrument(info_span!("Loading member"))
                    .await
            }
            Goal::Preview(composite_id, composite_path) => {
                self.perform_preview(context, composite_id, composite_path)
                    .instrument(info_span!("Previewing composite"))
                    .await
            }
            Goal::GetResourceProviderInfo(id, resource_path, mutation_cap) => {
                self.perform_get_resource_provider_info(
                    context,
                    id,
                    resource_path.clone(),
                    mutation_cap,
                )
                .instrument(info_span!(
                    "Getting resource provider info",
                    resource = ?resource_path
                ))
                .await
            }

            Goal::ListResourceInputs(id, path, mutation_cap) => {
                self.perform_list_resource_inputs(context, id, path.clone(), mutation_cap)
                    .instrument(info_span!("Listing resource inputs", resource = ?path))
                    .await
            }

            Goal::GetResourceInputValue(id, resource_path, input_name, cap) => {
                let resource_path_2 = resource_path.clone();
                let input_name_2 = input_name.clone();
                self.perform_get_resource_input_value(context, id, resource_path, input_name, cap)
                    .instrument(info_span!(
                        "Getting resource input value",
                        resource = ?resource_path_2,
                        input = input_name_2
                    ))
                    .await
            }

            Goal::GetResourceOutputValue(id, path, property, cap) => {
                let path_2 = path.clone();
                let property_2 = property.clone();
                self.perform_get_resource_output_value(context, id, path, property, cap)
                    .instrument(info_span!(
                        "Getting resource output value",
                        resource = ?path_2,
                        property = property_2
                    ))
                    .await
            }

            Goal::ApplyResource(id, path, cap) => {
                let path_2 = path.clone();
                let context = context.clone();
                self.perform_apply_resource(context, id, path, cap)
                    .instrument(info_span!("Applying resource", resource = ?path_2))
                    .await
            }

            Goal::RunState(id, path, cap) => {
                let path_2 = path.clone();
                self.perform_run_state(context, id, path, cap)
                    .instrument(info_span!(
                        "Running state provider resource",
                        resource = ?path_2
                    ))
                    .await
            }
        };
        Arc::new(r.map_err(Arc::new))
    }
}

fn clone_result<T: Clone>(r: &std::result::Result<T, Arc<anyhow::Error>>) -> Result<T> {
    match r {
        Ok(v) => Ok(v.clone()),
        Err(e) => Err(clone_anyhow_from_arc(e)),
    }
}

/// Reconstruct an [`anyhow::Error`] from an `Arc`, preserving the full error chain.
///
/// `anyhow::Error` doesn't implement `Clone`, but the task system wraps errors in
/// `Arc` for sharing across consumers. This reconstructs the chain by iterating
/// from root cause to outermost context, re-adding each layer via `.context()`.
pub(crate) fn clone_anyhow_from_arc(e: &Arc<anyhow::Error>) -> anyhow::Error {
    let chain: Vec<String> = e.chain().map(|c| c.to_string()).collect();
    let mut err = anyhow::anyhow!("{}", chain.last().unwrap());
    for msg in chain.iter().rev().skip(1) {
        err = err.context(msg.to_string());
    }
    err
}

/// Where an eval request can be sent from.
///
/// Inside a task the request is recorded as a dependency, so that cycles are
/// detected, whereas outside the graph there is no task to attribute it to.
/// This means `TaskContext` is always the preferred dispatcher.
///
/// In either case, `dispatch` starts the request before it returns.
/// It does not wait for the returned thunk to be forced, enabling concurrency.
#[allow(async_fn_in_trait)]
pub(crate) trait Dispatch {
    async fn dispatch(&self, goal: Goal) -> Result<Thunk<<WorkContext as TaskWork>::Output>>;
}

impl Dispatch for TaskContext<WorkContext> {
    async fn dispatch(&self, goal: Goal) -> Result<Thunk<<WorkContext as TaskWork>::Output>> {
        self.spawn(goal).await
    }
}

impl Dispatch for crate::control::task_tracker::TaskTracker<WorkContext> {
    async fn dispatch(&self, goal: Goal) -> Result<Thunk<<WorkContext as TaskWork>::Output>> {
        let thunk = self.create(goal).await;
        let driving = thunk.clone();
        tokio::spawn(async move {
            driving.force().await;
        });
        Ok(thunk)
    }
}

/// Send a request to the evaluator and hand back a future of its answer.
///
/// Since this method is itself async, using the goal's return value requires
/// two awaits:
/// 1. dispatch the goal
/// 2. await the outcome
///
/// The split allows more work to be done or dispatched before awaiting outcomes,
/// enabling concurrency.
///
// Functions of this shape also nicely hide the outcome variant matching invariant
// that needs runtime checking. A wrong outcome variant means this module is wrong
// about itself, so it panics rather than burdening callers with a case they cannot
// act on.
async fn request<T: 'static>(
    dispatch: &impl Dispatch,
    goal: Goal,
    named: &'static str,
    extract: fn(&Outcome) -> Option<T>,
) -> Result<impl Future<Output = Result<T>> + 'static> {
    let thunk = dispatch.dispatch(goal).await?;
    Ok(async move {
        match thunk.force().await.as_ref() {
            Ok(outcome) => match extract(outcome) {
                Some(value) => Ok(value),
                None => panic!("Unexpected outcome from {}: {:?}", named, outcome),
            },
            Err(e) => Err(clone_anyhow_from_arc(e)),
        }
    })
}

/// Ask the evaluator to preview a composite.
async fn preview(
    dispatch: &impl Dispatch,
    id: Id<CompositeType>,
    path: ComponentPath,
) -> Result<impl Future<Output = Result<Vec<PreviewItem>>> + 'static> {
    request(
        dispatch,
        Goal::Preview(id, path),
        "Preview",
        |outcome| match outcome {
            Outcome::Preview(items) => Some(items.clone()),
            _ => None,
        },
    )
    .await
}

/// Apply a composite. Returned future: wait for completion.
async fn apply(
    dispatch: &impl Dispatch,
    id: Id<CompositeType>,
    path: ComponentPath,
    mutation_cap: MutationCapability,
) -> Result<impl Future<Output = Result<()>> + 'static> {
    request(
        dispatch,
        Goal::Apply(id, path, mutation_cap),
        "Apply",
        |outcome| match outcome {
            Outcome::Done() => Some(()),
            _ => None,
        },
    )
    .await
}

/// Ask the evaluator which members a composite has.
///
/// The set of members can itself depend on a resource output, so this has the
/// same two answers as [`load_member`]: with a `mutation_cap` the resource is
/// applied until the names are known, and without one the structural
/// dependency returned in `Err` says which output stands in the way.
pub(crate) async fn list_members(
    dispatch: &impl Dispatch,
    id: Id<CompositeType>,
    path: ComponentPath,
    mutation_cap: Option<MutationCapability>,
) -> Result<
    impl Future<Output = Result<std::result::Result<Vec<String>, StructuralDependency>>> + 'static,
> {
    request(
        dispatch,
        Goal::ListMembers(id, path, mutation_cap),
        "ListMembers",
        |outcome| match outcome {
            Outcome::MembersListed(names) => Some(names.clone()),
            _ => None,
        },
    )
    .await
}

/// Ask the evaluator to assign an ID to a member of a composite.
async fn assign_member_id(
    dispatch: &impl Dispatch,
    id: Id<CompositeType>,
    name: String,
) -> Result<impl Future<Output = Result<Id<AnyType>>> + 'static> {
    request(
        dispatch,
        Goal::AssignMemberId(id, name),
        "AssignMemberId",
        |outcome| match outcome {
            Outcome::MemberIdAssigned(id) => Some(*id),
            _ => None,
        },
    )
    .await
}

/// Ask the evaluator to load a member of a composite.
///
/// Which members exist can itself depend on a resource output. Given a
/// `mutation_cap`, that resource is applied until the member is known, so the
/// answer is always `Ok`. Without one, that resource stays unapplied, so the
/// answer is the `Err` structural dependency, which each caller reports in its
/// own terms.
pub(crate) async fn load_member(
    dispatch: &impl Dispatch,
    id: Id<CompositeType>,
    name: String,
    mutation_cap: Option<MutationCapability>,
) -> Result<
    impl Future<Output = Result<std::result::Result<ComponentHandle, StructuralDependency>>> + 'static,
> {
    request(
        dispatch,
        Goal::LoadMember(id, name, mutation_cap),
        "LoadMember",
        |outcome| match outcome {
            Outcome::MemberLoaded(handle) => Some(handle.clone()),
            _ => None,
        },
    )
    .await
}

/// Ask the evaluator for a resource's provider info.
async fn get_resource_provider_info(
    dispatch: &impl Dispatch,
    id: Id<ResourceType>,
    path: ComponentPath,
    mutation_cap: Option<MutationCapability>,
) -> Result<impl Future<Output = Result<ResourceProviderInfo>> + 'static> {
    request(
        dispatch,
        Goal::GetResourceProviderInfo(id, path, mutation_cap),
        "GetResourceProviderInfo",
        |outcome| match outcome {
            Outcome::ResourceProviderInfo(info) => Some(info.clone()),
            _ => None,
        },
    )
    .await
}

/// Ask the evaluator which inputs a resource has.
async fn list_resource_inputs(
    dispatch: &impl Dispatch,
    id: Id<ResourceType>,
    path: ComponentPath,
    mutation_cap: Option<MutationCapability>,
) -> Result<impl Future<Output = Result<Vec<String>>> + 'static> {
    request(
        dispatch,
        Goal::ListResourceInputs(id, path, mutation_cap),
        "ListResourceInputs",
        |outcome| match outcome {
            Outcome::ResourceInputsListed(names) => Some(names.clone()),
            _ => None,
        },
    )
    .await
}

/// Ask the evaluator for the value of one resource input.
async fn get_resource_input_value(
    dispatch: &impl Dispatch,
    id: Id<ResourceType>,
    path: ComponentPath,
    input_name: String,
    mutation_cap: Option<MutationCapability>,
) -> Result<impl Future<Output = Result<Value>> + 'static> {
    request(
        dispatch,
        Goal::GetResourceInputValue(id, path, input_name, mutation_cap),
        "GetResourceInputValue",
        |outcome| match outcome {
            Outcome::ResourceInputValue(value) => Some(value.clone()),
            _ => None,
        },
    )
    .await
}

/// Apply a resource, and ask for it as the provider left it.
async fn apply_resource(
    dispatch: &impl Dispatch,
    id: Id<ResourceType>,
    path: ComponentPath,
    mutation_cap: MutationCapability,
) -> Result<impl Future<Output = Result<nixops4_resource::schema::v0::ExtantResource>> + 'static> {
    request(
        dispatch,
        Goal::ApplyResource(id, path, mutation_cap),
        "ApplyResource",
        |outcome| match outcome {
            Outcome::ResourceOutputs(resource) => Some(resource.clone()),
            _ => None,
        },
    )
    .await
}

/// Run a state resource, so that the state it holds can be read and written.
pub(crate) async fn run_state(
    dispatch: &impl Dispatch,
    id: Id<ResourceType>,
    path: ComponentPath,
    mutation_cap: Option<MutationCapability>,
) -> Result<impl Future<Output = Result<Arc<crate::state::StateHandle>>> + 'static> {
    request(
        dispatch,
        Goal::RunState(id, path, mutation_cap),
        "RunState",
        |outcome| match outcome {
            Outcome::RunState(handle) => Some(handle.clone()),
            _ => None,
        },
    )
    .await
}

/// The names have to arrive before the values can be asked for, but the
/// values are then all requested before any of them is awaited.
async fn request_resource_input_values(
    dispatch: &impl Dispatch,
    id: Id<ResourceType>,
    path: ComponentPath,
    mutation_cap: Option<MutationCapability>,
) -> Result<BTreeMap<String, impl Future<Output = Result<Value>> + 'static>> {
    let names = list_resource_inputs(dispatch, id, path.clone(), mutation_cap)
        .await?
        .await?;

    let mut value_futs = BTreeMap::new();
    for name in names {
        let value_fut =
            get_resource_input_value(dispatch, id, path.clone(), name.clone(), mutation_cap)
                .await?;
        value_futs.insert(name, value_fut);
    }
    Ok(value_futs)
}

/// Ask the evaluator to resolve a component path to a composite ID.
pub(crate) async fn resolve_composite_path(
    dispatch: &impl Dispatch,
    path: ComponentPath,
) -> Result<impl Future<Output = Result<Id<CompositeType>>> + 'static> {
    request(
        dispatch,
        Goal::ResolveCompositePath(path),
        "ResolveCompositePath",
        |outcome| match outcome {
            Outcome::CompositeResolved(id) => Some(*id),
            _ => None,
        },
    )
    .await
}

fn indented_json(v: &Value) -> String {
    let s = serde_json::to_string_pretty(v).unwrap();
    s.replace("\n", "\n            ")
}
