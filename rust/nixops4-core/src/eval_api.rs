use std::{
    hash::{Hash, Hasher},
    sync::{atomic::AtomicU64, Arc},
};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct Ids {
    counter: Arc<AtomicU64>,
}
impl Ids {
    // new_without_default: Not a value type. Equal instances are not interchangeable, so default() is not an appropriate constructor method.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Ids {
            counter: Arc::new(AtomicU64::new(0)),
        }
    }
    pub fn next<T>(&self) -> Id<T> {
        let id = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Id::new(id)
    }
}

pub type IdNum = u64;

/// A unique identifier for a value.
/// The type parameter T is used to check that the id is only used for the type it was created for.
/// This is a compile-time check only, and only serves to help the programmer.
///
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Id<T> {
    id: IdNum,
    // nothing, just to accept the compile-type only T
    #[serde(skip)]
    phantom: std::marker::PhantomData<T>,
}
impl<T> Id<T> {
    fn new(id: u64) -> Self {
        Id {
            id,
            phantom: std::marker::PhantomData,
        }
    }
    /// Create an Id with a specific type from an IdNum.
    /// Used by the evaluator to create typed IDs in responses.
    pub fn from_num(id: IdNum) -> Self {
        Id::new(id)
    }
    pub fn num(&self) -> IdNum {
        self.id
    }
    /// Erase the type (compile-time only)
    pub fn any(&self) -> Id<AnyType> {
        Id::new(self.id)
    }
}
impl<T> Hash for Id<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}
impl<T: Clone> Copy for Id<T> {}
impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<T> Eq for Id<T> {}
impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}
unsafe impl<T> Send for Id<T> {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnyType;

/// `QueryRequest`-based requests use message ids to match responses to requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageType;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlakeType;
/// Type marker for composite component IDs (components with nested members)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompositeType;
/// Type marker for resource component IDs (components wrapping provider resources)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceType;

/// Handle returned by LoadComponent - determines component kind
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComponentHandle {
    Resource(Id<ResourceType>),
    Composite(Id<CompositeType>),
}

/// A path to a component within nested components.
/// An empty path represents the root component.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, valuable::Valuable,
)]
pub struct ComponentPath(pub Vec<String>);

impl ComponentPath {
    /// Create a new root component path
    pub fn root() -> Self {
        Self(Vec::new())
    }

    /// Check if this is the root component
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Create a path to a child component
    pub fn child(&self, name: String) -> Self {
        let mut path = self.0.clone();
        path.push(name);
        Self(path)
    }

    /// Get the parent path and name of this component, if not root
    pub fn parent(&self) -> Option<(ComponentPath, &str)> {
        if self.0.is_empty() {
            None
        } else {
            let name = self.0.last().unwrap();
            let parent = ComponentPath(self.0[..self.0.len() - 1].to_vec());
            Some((parent, name))
        }
    }
}

impl std::fmt::Display for ComponentPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_empty() {
            write!(f, "(root)")
        } else {
            write!(f, "{}", self.0.join("."))
        }
    }
}

impl std::str::FromStr for ComponentPath {
    type Err = std::convert::Infallible;

    // TODO: parse quoted attributes (e.g., foo."example.com".qux)
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            Ok(Self::root())
        } else {
            Ok(Self(s.split('.').map(String::from).collect()))
        }
    }
}

/// This interface is internal to NixOps4. It is used to communicate between the CLI and the evaluator.
/// Only matching CLI and evaluator versions are compatible.
/// No promises are made about this interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvalRequest {
    LoadFlake(AssignRequest<FlakeRequest>),
    /// Load the root component from a flake (returns a composite component)
    LoadRoot(AssignRequest<RootRequest>),
    /// List members in a composite component (unified: replaces ListResources + ListNestedDeployments)
    ListMembers(QueryRequest<Id<CompositeType>, (Id<CompositeType>, ListMembersState)>),
    /// Load a component by name from a parent composite (returns ComponentHandle indicating kind)
    LoadComponent(AssignRequest<ComponentRequest>),
    /// Get resource provider info for a loaded resource component
    GetResource(QueryRequest<Id<ResourceType>, ResourceProviderInfoState>),
    /// List input names for a resource
    ListResourceInputs(QueryRequest<Id<ResourceType>, (Id<ResourceType>, ListResourceInputsState)>),
    /// Get a specific resource input value
    GetResourceInput(QueryRequest<Property, ResourceInputState>),
    /// Provide a resource output value for the fixpoint
    PutResourceOutput(NamedProperty, Value),
}

pub trait RequestIdType {
    /// `Id` type associated with the result of the request
    type IdType: Clone;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignRequest<R: RequestIdType> {
    /// Unique id provided by the client.
    pub assign_to: Id<R::IdType>,
    pub payload: R,
}
impl<Req: RequestIdType> RequestIdType for AssignRequest<Req> {
    type IdType = Req::IdType;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryRequest<P, R> {
    pub message_id: Id<MessageType>,
    pub payload: P,
    #[serde(skip)]
    phantom: std::marker::PhantomData<R>,
}
impl<P, R> QueryRequest<P, R> {
    pub fn new(message_id: Id<MessageType>, payload: P) -> Self {
        QueryRequest {
            message_id,
            payload,
            phantom: std::marker::PhantomData,
        }
    }
}
impl<P, R> RequestIdType for QueryRequest<P, R> {
    type IdType = AnyType;
}

/// This interface is internal to NixOps4. It is used to communicate between the CLI and the evaluator.
/// Only matching CLI and evaluator versions are compatible.
/// No promises are made about this interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvalResponse {
    Error(Id<AnyType>, String),
    QueryResponse(Id<MessageType>, QueryResponseValue),
    TracingEvent(
        /// This is a tracing_tunnel::TracingEvent, but that type (rightfully)
        /// does not implement Eq, while we would like to have that on our other
        /// EvalResponse variants for ease of testing.
        Value,
    ),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryResponseValue {
    ListMembers((Id<CompositeType>, ListMembersState)),
    /// Response from LoadComponent indicating the component kind or a dependency
    ComponentLoaded(ComponentLoadState),
    ResourceProviderInfo(ResourceProviderInfoState),
    ListResourceInputs((Id<ResourceType>, ListResourceInputsState)),
    ResourceInputState((Property, ResourceInputState)),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceInputState {
    ResourceInputValue((Property, Value)),
    ResourceInputDependency(ResourceInputDependency),
}

/// Response state for ListMembers request.
/// Returns only member names - kind is determined when loading via LoadComponent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListMembersState {
    Listed(Vec<String>),
    /// See [`ComponentLoadState::StructuralDependency`] for retry semantics.
    StructuralDependency(NamedProperty),
}

/// Response state for LoadComponent request.
/// Returns the component handle, or indicates a structural dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComponentLoadState {
    Loaded(ComponentHandle),
    /// Evaluation requires a resource output that doesn't exist yet.
    ///
    /// Unlike `Loaded` and errors, this response is intentionally NOT cached by
    /// the evaluator. This enables retry: after the work scheduler resolves the
    /// dependency (by applying the required resource), it re-sends `LoadComponent`
    /// with the same ID. Since the dependency wasn't cached, the evaluator
    /// re-evaluates with the now-available output value.
    StructuralDependency(NamedProperty),
}

/// Response state for GetResource request.
/// Returns provider info, or indicates a structural dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceProviderInfoState {
    Loaded(ResourceProviderInfo),
    /// See [`ComponentLoadState::StructuralDependency`] for retry semantics.
    StructuralDependency(NamedProperty),
}

/// Response state for ListResourceInputs request.
/// Returns input names, or indicates a structural dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListResourceInputsState {
    Listed(Vec<String>),
    /// See [`ComponentLoadState::StructuralDependency`] for retry semantics.
    StructuralDependency(NamedProperty),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceProviderInfo {
    pub id: Id<ResourceType>,
    pub provider: Value,
    pub resource_type: String,
    /// Path to state handler component, if this resource is stateful
    pub state: Option<ComponentPath>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceInputDependency {
    pub dependent: Property,
    pub dependency: NamedProperty,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NamedProperty {
    /// Path to the resource component
    pub resource: ComponentPath,
    pub name: String,
}

/// Can be input property or output property
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Property {
    pub resource: Id<ResourceType>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlakeRequest {
    /// The path to the flake to load.
    pub abspath: String,
    pub input_overrides: Vec<(String, String)>,
}
impl RequestIdType for FlakeRequest {
    type IdType = FlakeType;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootRequest {
    /// The flake to load the root component from.
    pub flake: Id<FlakeType>,
}
impl RequestIdType for RootRequest {
    type IdType = CompositeType;
}

/// Request to load a component (resource or composite) by name from a parent composite.
/// The response indicates the component kind via ComponentHandle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentRequest {
    /// The parent composite component to load from.
    pub parent: Id<CompositeType>,
    /// The name of the member component to load.
    pub name: String,
}
impl RequestIdType for ComponentRequest {
    /// ComponentRequest returns ComponentHandle, but AssignRequest needs a single type.
    /// We use AnyType here; the actual result is a ComponentHandle enum.
    type IdType = AnyType;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceSpec {
    /// Parent composite this resource component is part of
    pub parent: Id<CompositeType>,
    /// Name of the resource in the parent composite
    pub name: String,

    // Value of the resource
    /// Type of the resource, e.g. `"stdio-simple"`
    pub resource_api: String,
    /// Arbitrary JSON input for the resource
    pub inputs_json: String,
    /// Realised store paths
    // TODO: use unreleased derivable paths for better performance
    pub store_paths: Vec<String>,
}

/// Facade for nixops4-eval
pub fn eval_request_from_json(s: &str) -> Result<EvalRequest> {
    serde_json::from_str(s).map_err(|e| e.into())
}

/// Facade for nixops4-eval
pub fn eval_response_to_json(r: &EvalResponse) -> Result<String> {
    serde_json::to_string(r).map_err(|e| e.into())
}

/// Facade for nixops4-core
pub fn eval_request_to_json(s: &EvalRequest) -> Result<String> {
    serde_json::to_string(s).map_err(|e| e.into())
}

/// Facade for nixops4-core
pub fn eval_response_from_json(r: &str) -> Result<EvalResponse> {
    serde_json::from_str(r).map_err(|e| e.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ids() {
        let ids = Ids::new();
        let id1: Id<FlakeType> = ids.next();
        let id2 = ids.next();
        assert_ne!(id1, id2);
        assert_eq!(id1.num() + 1, id2.num());
    }

    #[test]
    fn test_id_any() {
        let ids = Ids::new();
        let id1: Id<FlakeType> = ids.next();
        let id2 = id1.any();
        assert_eq!(id1.num(), id2.num());
    }

    #[test]
    fn test_eval_request_load_flake() {
        let req = EvalRequest::LoadFlake(AssignRequest {
            assign_to: Id::new(1),
            payload: FlakeRequest {
                abspath: "/path/to/flake".to_string(),
                input_overrides: Vec::new(),
            },
        });
        let s = eval_request_to_json(&req).unwrap();
        eprintln!("{}", s);
        let req2 = eval_request_from_json(&s).unwrap();
        assert_eq!(req, req2);
    }

    #[test]
    fn test_eval_request_load_root() {
        let req = EvalRequest::LoadRoot(AssignRequest {
            assign_to: Id::new(2),
            payload: RootRequest { flake: Id::new(1) },
        });
        let s = eval_request_to_json(&req).unwrap();
        eprintln!("{}", s);
        let req2 = eval_request_from_json(&s).unwrap();
        assert_eq!(req, req2);
    }

    #[test]
    fn test_resource_provider_info_without_state() {
        let info = ResourceProviderInfo {
            id: Id::new(1),
            provider: serde_json::json!({"executable": "/bin/test", "type": "stdio"}),
            resource_type: "file".to_string(),
            state: None,
        };

        // Test serialization/deserialization
        let json = serde_json::to_string(&info).unwrap();
        let info2: ResourceProviderInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, info2);

        // Verify state field is None for stateless resources
        assert_eq!(info.state, None);
    }

    #[test]
    fn test_resource_provider_info_with_state() {
        let info = ResourceProviderInfo {
            id: Id::new(2),
            provider: serde_json::json!({"executable": "/bin/memo", "type": "stdio"}),
            resource_type: "memo".to_string(),
            state: Some(ComponentPath(vec!["myStateHandler".to_string()])),
        };

        // Test serialization/deserialization
        let json = serde_json::to_string(&info).unwrap();
        let info2: ResourceProviderInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, info2);

        // Verify state field contains the expected value
        assert_eq!(
            info.state,
            Some(ComponentPath(vec!["myStateHandler".to_string()]))
        );
    }

    #[test]
    fn test_resource_provider_info_json_compatibility() {
        // Test that JSON without state field can still be deserialized
        let json_without_state = r#"{
            "id": {"id": 3},
            "provider": {"executable": "/bin/test", "type": "stdio"},
            "resource_type": "file"
        }"#;

        let info: ResourceProviderInfo = serde_json::from_str(json_without_state).unwrap();
        assert_eq!(info.state, None);
        assert_eq!(info.resource_type, "file");

        // Test that JSON with state field works correctly (new ComponentPath format)
        let json_with_state = r#"{
            "id": {"id": 4},
            "provider": {"executable": "/bin/memo", "type": "stdio"},
            "resource_type": "memo",
            "state": ["myState"]
        }"#;

        let info: ResourceProviderInfo = serde_json::from_str(json_with_state).unwrap();
        assert_eq!(info.state, Some(ComponentPath(vec!["myState".to_string()])));
        assert_eq!(info.resource_type, "memo");
    }

    #[test]
    fn test_component_path() {
        let root = ComponentPath::root();
        assert!(root.is_root());
        assert_eq!(root.to_string(), "(root)");

        let child = root.child("foo".to_string());
        assert!(!child.is_root());
        assert_eq!(child.to_string(), "foo");

        let grandchild = child.child("bar".to_string());
        assert_eq!(grandchild.to_string(), "foo.bar");

        // Test parent()
        let (parent, name) = grandchild.parent().unwrap();
        assert_eq!(name, "bar");
        assert_eq!(parent.to_string(), "foo");
    }

    #[test]
    fn test_component_handle() {
        let ids = Ids::new();
        let resource_id: Id<ResourceType> = ids.next();
        let composite_id: Id<CompositeType> = ids.next();

        let handle_r = ComponentHandle::Resource(resource_id);
        let handle_c = ComponentHandle::Composite(composite_id);

        // Test serialization
        let json_r = serde_json::to_string(&handle_r).unwrap();
        let json_c = serde_json::to_string(&handle_c).unwrap();

        let handle_r2: ComponentHandle = serde_json::from_str(&json_r).unwrap();
        let handle_c2: ComponentHandle = serde_json::from_str(&json_c).unwrap();

        assert_eq!(handle_r, handle_r2);
        assert_eq!(handle_c, handle_c2);
    }
}
