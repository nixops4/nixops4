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
use nixops4_resource_runner::{ResourceProviderClient, ResourceProviderConfig};
use pubsub_rs::Pubsub;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fmt::Display,
    sync::Arc,
};
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

/// An item in the deployment preview, representing either a known resource
/// or an unknown structure blocked by a structural dependency.
#[derive(Clone, Debug)]
pub enum PreviewItem {
    /// A resource that will be applied (full path from root)
    Resource(ComponentPath),
    /// A structural dependency that must be resolved before the full structure is known.
    StructuralDependency {
        /// The composite path where the unknown structure exists (if known)
        path: Option<ComponentPath>,
        /// The resource output this structure depends on
        depends_on: NamedProperty,
    },
}

impl Display for PreviewItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreviewItem::Resource(path) => write!(f, "{}", path),
            PreviewItem::StructuralDependency { path, depends_on } => match path {
                Some(p) if p.is_root() => {
                    write!(
                        f,
                        "*  (structure depends on {}.{})",
                        depends_on.resource, depends_on.name
                    )
                }
                Some(p) => {
                    write!(
                        f,
                        "{}.*  (depends on {}.{})",
                        p, depends_on.resource, depends_on.name
                    )
                }
                None => {
                    write!(
                        f,
                        "*  (depends on {}.{})",
                        depends_on.resource, depends_on.name
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
    GetResourceProviderInfo(Id<ResourceType>, ComponentPath, MutationCapability),
    /// List resource inputs.
    /// Note: path is only used for logging; id is the source of truth.
    ListResourceInputs(Id<ResourceType>, ComponentPath, MutationCapability),
    /// Get a specific resource input value.
    /// Note: path is only used for logging; id is the source of truth.
    GetResourceInputValue(Id<ResourceType>, ComponentPath, String, MutationCapability),
    /// Get a resource output value (goes to ApplyResource).
    /// Note: path is only used for logging; id is the source of truth.
    GetResourceOutputValue(Id<ResourceType>, ComponentPath, String, MutationCapability),
    /// Apply a single resource.
    /// Note: path is only used for logging; id is the source of truth.
    ApplyResource(Id<ResourceType>, ComponentPath, MutationCapability),
    /// Run a state provider resource.
    /// Note: path is only used for logging; id is the source of truth.
    RunState(Id<ResourceType>, ComponentPath, MutationCapability),
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
    MembersListed(Result<Vec<String>, PreviewItem>),
    /// An ID assigned to a member (from AssignMemberId)
    MemberIdAssigned(Id<AnyType>),
    /// A single member loaded, or blocked by structural dependency (for preview)
    MemberLoaded(Result<ComponentHandle, PreviewItem>),
    ResourceProviderInfo(eval_api::ResourceProviderInfo),
    ResourceInputsListed(Vec<String>),
    ResourceInputValue(Value),
    ResourceOutputValue, /*Value ignored because passed eagerly before ResourceOutputs, a dependency of this.*/
    ResourceOutputs(nixops4_resource::schema::v0::ExtantResource),
    RunState(Arc<crate::state::StateHandle>),
}

pub struct WorkContext {
    pub options: crate::Options,
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
    ) -> Result<Result<Vec<String>, PreviewItem>> {
        let r = context
            .require(Goal::ListMembers(
                composite_id,
                composite_path.clone(),
                mutation_cap,
            ))
            .await;
        let outcome = match r {
            Err(e) => bail!("Error while listing members: {}", e),
            Ok(v) => clone_result(v.as_ref()),
        }?;
        match outcome {
            Outcome::MembersListed(result) => Ok(result),
            outcome => bail!("Unexpected outcome from ListMembers, {:?}", outcome),
        }
    }

    /// Load a single member via LoadMember goal.
    async fn load_member(
        &self,
        context: &TaskContext<Self>,
        composite_id: Id<CompositeType>,
        name: &str,
        mutation_cap: Option<MutationCapability>,
    ) -> Result<ComponentHandle> {
        let r = context
            .require(Goal::LoadMember(
                composite_id,
                name.to_string(),
                mutation_cap,
            ))
            .await;
        let outcome = match r {
            Err(e) => bail!("Error while loading member '{}': {}", name, e),
            Ok(v) => clone_result(v.as_ref()),
        }?;
        match outcome {
            Outcome::MemberLoaded(Ok(handle)) => Ok(handle),
            Outcome::MemberLoaded(Err(dep)) => {
                bail!(
                    "Structural dependency while loading member '{}': {}",
                    name,
                    dep
                )
            }
            outcome => bail!("Unexpected outcome from LoadMember, {:?}", outcome),
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
            PreviewItem,
        >,
    > {
        // Spawn all LoadMember goals in parallel
        let mut thunks = BTreeMap::new();
        for name in names {
            let thunk = context
                .spawn(Goal::LoadMember(composite_id, name.clone(), mutation_cap))
                .await?;
            thunks.insert(name, thunk);
        }

        // Await all and partition by kind, keeping typed IDs
        let mut resources = BTreeMap::new();
        let mut composites = BTreeMap::new();
        for (name, outcome) in Thunk::force_into_map(thunks).await {
            let handle = match clone_result(&outcome)? {
                Outcome::MemberLoaded(Ok(h)) => h,
                Outcome::MemberLoaded(Err(dep)) => {
                    // Structural dependency - return it
                    return Ok(Err(dep));
                }
                outcome => bail!("Unexpected outcome from LoadMember: {:?}", outcome),
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
            PreviewItem,
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

    /// Resolve a dependency by requiring the resource output value.
    async fn resolve_dependency(
        &self,
        context: &TaskContext<Self>,
        dep: &NamedProperty,
        mutation_cap: MutationCapability,
    ) -> Result<()> {
        let dep_id = self.get_resource_id(context, &dep.resource).await?;
        let r = context
            .require(Goal::GetResourceOutputValue(
                dep_id,
                dep.resource.clone(),
                dep.name.clone(),
                mutation_cap,
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
                items.push(structural_dep);
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
            let thunk = context
                .spawn(Goal::Preview(*nested_id, nested_path))
                .await?;
            nested_thunks.push(thunk);
        }

        // Collect results from nested composites
        for thunk in nested_thunks {
            match clone_result(thunk.force().await.as_ref())? {
                Outcome::Preview(nested_items) => {
                    items.extend(nested_items);
                }
                outcome => panic!("Unexpected outcome from Preview: {:?}", outcome),
            }
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
        let preview_thunk = context
            .require(Goal::Preview(composite_id, composite_path.clone()))
            .await?;
        let preview_items = match clone_result(&preview_thunk)? {
            Outcome::Preview(items) => items,
            outcome => panic!("Unexpected outcome from Preview: {:?}", outcome),
        };

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
            let thunk = context
                .spawn(Goal::Apply(*nested_id, nested_path, mutation_cap))
                .await?;
            nested_thunks.push(thunk);
        }

        // Apply resources (IDs already available from partitioning)
        let mut resource_thunk_map = BTreeMap::new();
        for (resource_path, id) in &resources {
            let r = context
                .spawn(Goal::ApplyResource(
                    *id,
                    resource_path.clone(),
                    mutation_cap,
                ))
                .await?;
            resource_thunk_map.insert(*id, r);
        }

        let mut resource_map = BTreeMap::new();
        for (id, outcome) in Thunk::force_into_map(resource_thunk_map).await {
            match clone_result(&outcome)? {
                Outcome::ResourceOutputs(i) => {
                    resource_map.insert(id, i);
                }
                _ => panic!("Unexpected outcome from ApplyResource: {:?}", outcome),
            }
        }

        // Wait for nested composites to complete
        for thunk in nested_thunks {
            match clone_result(thunk.force().await.as_ref())? {
                Outcome::Done() => {}
                outcome => panic!("Unexpected outcome from Apply: {:?}", outcome),
            }
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
                    Some(cap) => self.resolve_dependency(&context, &dep, cap).await?,
                    None => {
                        return Ok(Outcome::MembersListed(Err(
                            PreviewItem::StructuralDependency {
                                path: Some(composite_path),
                                depends_on: dep,
                            },
                        )));
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
                EvalResponse::Error(_id, e) => bail!("Evaluation error: {}", e),
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
        let r = context
            .require(Goal::AssignMemberId(parent_id, name.clone()))
            .await?;
        let id = match clone_result(&r)? {
            Outcome::MemberIdAssigned(id) => id,
            _ => bail!("Unexpected outcome from AssignMemberId"),
        };

        loop {
            let step = self.eval_get_component_kind(id).await?;
            match step {
                StepResult::Done(handle) => {
                    return Ok(Outcome::MemberLoaded(Ok(handle)));
                }
                StepResult::Needs(dep) => match mutation_cap {
                    Some(cap) => self.resolve_dependency(&context, &dep, cap).await?,
                    None => {
                        return Ok(Outcome::MemberLoaded(Err(
                            PreviewItem::StructuralDependency {
                                path: None,
                                depends_on: dep,
                            },
                        )));
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
                EvalResponse::Error(_id, e) => bail!("Evaluation error: {}", e),
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
    async fn perform_get_resource_provider_info(
        &self,
        context: TaskContext<Self>,
        id: Id<ResourceType>,
        _resource_path: ComponentPath,
        mutation_cap: MutationCapability,
    ) -> Result<Outcome> {
        loop {
            match self.eval_get_resource_provider_info(id).await? {
                StepResult::Done(info) => return Ok(Outcome::ResourceProviderInfo(info)),
                StepResult::Needs(dep) => {
                    self.resolve_dependency(&context, &dep, mutation_cap)
                        .await?
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
                EvalResponse::Error(_id, e) => bail!("Evaluation error: {}", e),
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
    async fn perform_list_resource_inputs(
        &self,
        context: TaskContext<Self>,
        id: Id<ResourceType>,
        _resource_path: ComponentPath,
        mutation_cap: MutationCapability,
    ) -> Result<Outcome> {
        loop {
            match self.eval_list_resource_inputs(id).await? {
                StepResult::Done(input_names) => {
                    return Ok(Outcome::ResourceInputsListed(input_names))
                }
                StepResult::Needs(dep) => {
                    self.resolve_dependency(&context, &dep, mutation_cap)
                        .await?
                }
            }
        }
    }

    async fn perform_get_resource_input_value(
        &self,
        context: TaskContext<Self>,
        id: Id<ResourceType>,
        _resource_path: ComponentPath,
        input_name: String,
        mutation_cap: MutationCapability,
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
                EvalResponse::Error(_id, e) => bail!("Evaluation error: {}", e),
                EvalResponse::QueryResponse(_id, query_response_value) => {
                    match query_response_value {
                        QueryResponseValue::ResourceInputValue(step_result) => match step_result {
                            StepResult::Done(value) => {
                                break Ok(Outcome::ResourceInputValue(value))
                            }
                            StepResult::Needs(dep) => {
                                self.resolve_dependency(&context, &dep, mutation_cap)
                                    .await?
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
        let r = context
            .require(Goal::ApplyResource(id, resource_path.clone(), mutation_cap))
            .await?;
        let outcome = clone_result(&r)?;
        match outcome {
            Outcome::ResourceOutputs(extant_resource) => {
                if let Some(output_properties) = &extant_resource.output_properties {
                    if output_properties.contains_key(&property) {
                        Ok(Outcome::ResourceOutputValue)
                    } else {
                        bail!(
                            "Resource {} does not have output {}",
                            resource_path,
                            property
                        )
                    }
                } else {
                    bail!("Resource {} has no output properties", resource_path)
                }
            }
            _ => panic!("Unexpected outcome from ApplyResource: {:?}", outcome),
        }
    }

    async fn perform_apply_resource(
        &self,
        context: TaskContext<Self>,
        id: Id<ResourceType>,
        resource_path: ComponentPath,
        mutation_cap: MutationCapability,
    ) -> Result<Outcome> {
        let provider_info_thunk = context
            .spawn(Goal::GetResourceProviderInfo(
                id,
                resource_path.clone(),
                mutation_cap,
            ))
            .await?;

        let resource_inputs_list_id = context
            .spawn(Goal::ListResourceInputs(
                id,
                resource_path.clone(),
                mutation_cap,
            ))
            .await?;

        let inputs_list = {
            let outcome = clone_result(resource_inputs_list_id.force().await.as_ref())?;
            match outcome {
                Outcome::ResourceInputsListed(i) => i,
                _ => {
                    panic!("Unexpected outcome from ListResourceInputs: {:?}", outcome)
                }
            }
        };

        let mut inputs_thunks = HashMap::new();
        for input_name in inputs_list {
            let input_id = context
                .spawn(Goal::GetResourceInputValue(
                    id,
                    resource_path.clone(),
                    input_name.clone(),
                    mutation_cap,
                ))
                .await?;
            inputs_thunks.insert(input_name, input_id);
        }

        let provider_info = {
            let outcome = clone_result(provider_info_thunk.force().await.as_ref())?;
            match outcome {
                Outcome::ResourceProviderInfo(i) => i,
                _ => panic!(
                    "Unexpected outcome from GetResourceProviderInfo: {:?}",
                    outcome
                ),
            }
        };

        let state_provider = match &provider_info.state {
            Some(state_resource_path) => {
                // Get the state resource ID
                let state_resource_id = self
                    .get_resource_id(&context, state_resource_path)
                    .await
                    .with_context(|| {
                    format!(
                        "Invalid state reference '{}' for resource",
                        state_resource_path
                    )
                })?;

                let thunk = context
                    .spawn(Goal::RunState(
                        state_resource_id,
                        state_resource_path.clone(),
                        mutation_cap,
                    ))
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to start state provider '{}' for resource",
                            state_resource_path
                        )
                    })?;
                Some((state_resource_path, state_resource_id, thunk))
            }
            None => None,
        };

        let state = match state_provider {
            Some((_state_resource_id, _state_resource_name, thunk)) => {
                let outcome = clone_result(thunk.force().await.as_ref())?;
                match outcome {
                    Outcome::RunState(i) => Some(i),
                    _ => panic!("Unexpected outcome from RunState: {:?}", outcome),
                }
            }
            None => None,
        };

        let inputs = {
            let mut inputs = serde_json::Map::new();
            for (input_name, input_thunk) in inputs_thunks {
                let outcome = clone_result(input_thunk.force().await.as_ref())?;
                match outcome {
                    Outcome::ResourceInputValue(i) => {
                        inputs.insert(input_name, i);
                    }
                    _ => panic!(
                        "Unexpected outcome from GetResourceInputValue: {:?}",
                        outcome
                    ),
                }
            }
            inputs
        };

        let span = info_span!(
            "creating resource",
            name = resource_path.to_string().as_str()
        );

        if self.options.verbose {
            eprintln!(
                "Provider details for {}: {:?}",
                resource_path, &provider_info
            );
            eprintln!("Resource inputs for {}: {:?}", resource_path, inputs);
        }

        let provider_argv = provider::parse_provider(&provider_info.provider)?;
        // Run the provider
        let mut provider = ResourceProviderClient::new(ResourceProviderConfig {
            provider_executable: provider_argv.executable,
            provider_args: provider_argv.args,
        })
        .await?;

        let outputs = match state {
            None => provider
                .create(provider_info.resource_type.as_str(), &inputs, false)
                .await
                .with_context(|| {
                    format!("Failed to create stateless resource {}", resource_path)
                })?,
            Some(ref state_handle) => {
                let past_resource_opt = state_handle
                    .current
                    .lock()
                    .await
                    .deployment
                    .get_resource(&resource_path)
                    .cloned();
                match past_resource_opt {
                    Some(past_resource) => {
                        if past_resource.type_ != provider_info.resource_type {
                            bail!(
                                "Resource type change is not supported: {} != {}",
                                past_resource.type_,
                                provider_info.resource_type
                            );
                        }

                        // Skip update if inputs haven't changed
                        let outputs = if inputs == past_resource.input_properties {
                            tracing::info!(
                                "Skipping update for resource {}: inputs unchanged",
                                resource_path
                            );
                            past_resource.output_properties.clone()
                        } else {
                            tracing::info!("Updating resource {}: inputs changed", resource_path);
                            provider
                                .update(
                                    provider_info.resource_type.as_str(),
                                    &inputs,
                                    &past_resource.input_properties,
                                    &past_resource.output_properties,
                                )
                                .await
                                .with_context(|| {
                                    format!("Failed to update resource {}", resource_path)
                                })?
                        };
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

        // Close the provider properly
        // We might want to reuse them in the future, but for now we launch one
        // per resource, except when it's a state provider (elsewhere).
        provider.close_wait().await?;

        let resource = nixops4_resource::schema::v0::ExtantResource {
            input_properties: nixops4_resource::schema::v0::InputProperties(inputs),
            output_properties: Some(nixops4_resource::schema::v0::OutputProperties(outputs)),
            type_: nixops4_resource::schema::v0::ResourceType(provider_info.resource_type),
        };

        Ok(Outcome::ResourceOutputs(resource))
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
                (async {
                    // Apply the resource
                    let r = context
                        .require(Goal::ApplyResource(id, path.clone(), cap))
                        .await
                        .with_context(|| format!("Failed to apply resource {}", path))?;
                    let resource = {
                        let outcome = clone_result(r.as_ref())?;
                        match outcome {
                            Outcome::ResourceOutputs(i) => i,
                            _ => panic!("Unexpected outcome from ApplyResource: {:?}", outcome),
                        }
                    };

                    // Get the provider info
                    let provider_info_thunk = context
                        .spawn(Goal::GetResourceProviderInfo(id, path.clone(), cap))
                        .await
                        .with_context(|| {
                            format!("Failed to get provider info for resource {}", path)
                        })?;
                    let provider_info = {
                        let outcome = clone_result(provider_info_thunk.force().await.as_ref())?;
                        match outcome {
                            Outcome::ResourceProviderInfo(i) => i,
                            _ => panic!(
                                "Unexpected outcome from GetResourceProviderInfo: {:?}",
                                outcome
                            ),
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
                })
                .instrument(info_span!(
                    "Running state provider resource",
                    resource = ?path
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
        Err(e) => bail!("{}", e),
    }
}

fn indented_json(v: &Value) -> String {
    let s = serde_json::to_string_pretty(v).unwrap();
    s.replace("\n", "\n            ")
}
