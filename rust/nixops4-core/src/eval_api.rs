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
    panthom: std::marker::PhantomData<T>,
}
impl<T> Id<T> {
    fn new(id: u64) -> Self {
        Id {
            id,
            panthom: std::marker::PhantomData,
        }
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentType;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceType;

/// A path to a deployment within nested deployments.
/// An empty path represents the root deployment.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, valuable::Valuable,
)]
pub struct DeploymentPath(pub Vec<String>);

impl DeploymentPath {
    /// Create a new root deployment path
    pub fn root() -> Self {
        Self(Vec::new())
    }

    /// Check if this is the root deployment
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Create a path to a child deployment
    pub fn child(&self, name: String) -> Self {
        let mut path = self.0.clone();
        path.push(name);
        Self(path)
    }
}

impl std::fmt::Display for DeploymentPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_empty() {
            write!(f, "(root)")
        } else {
            write!(f, "{}", self.0.join("."))
        }
    }
}

/// A resource path within a deployment
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, valuable::Valuable,
)]
pub struct ResourcePath {
    /// Path to the deployment containing the resource
    pub deployment_path: DeploymentPath,
    /// Name of the resource within the deployment
    pub resource_name: String,
}

impl std::fmt::Display for ResourcePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.deployment_path.0.is_empty() {
            write!(f, "{}", self.resource_name)
        } else {
            write!(
                f,
                "{}.{}",
                self.deployment_path.0.join("."),
                self.resource_name
            )
        }
    }
}

/// This interface is internal to NixOps4. It is used to communicate between the CLI and the evaluator.
/// Only matching CLI and evaluator versions are compatible.
/// No promises are made about this interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvalRequest {
    LoadFlake(AssignRequest<FlakeRequest>),
    ListDeployments(QueryRequest<Id<FlakeType>, (Id<FlakeType>, Vec<String>)>),
    /// Load a deployment from a flake. The deployment is treated as a composite component.
    LoadDeployment(AssignRequest<DeploymentRequest>),
    /// Load a nested deployment from a parent deployment
    LoadNestedDeployment(AssignRequest<NestedDeploymentRequest>),
    /// List nested deployments in a deployment
    ListNestedDeployments(
        QueryRequest<Id<DeploymentType>, (Id<DeploymentType>, ListNestedDeploymentsState)>,
    ),
    /// List resources in a deployment
    ListResources(QueryRequest<Id<DeploymentType>, (Id<DeploymentType>, ListResourcesState)>),
    /// Load a resource by name from a deployment
    LoadResource(AssignRequest<ResourceRequest>),
    /// Get resource provider info for a loaded resource
    GetResource(QueryRequest<Id<ResourceType>, ResourceProviderInfo>),
    /// List input names for a resource
    ListResourceInputs(QueryRequest<Id<ResourceType>, (Id<ResourceType>, Vec<String>)>),
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
    panthom: std::marker::PhantomData<R>,
}
impl<P, R> QueryRequest<P, R> {
    pub fn new(message_id: Id<MessageType>, payload: P) -> Self {
        QueryRequest {
            message_id,
            payload,
            panthom: std::marker::PhantomData,
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
    ListDeployments((Id<FlakeType>, Vec<String>)),
    ListNestedDeployments((Id<DeploymentType>, ListNestedDeploymentsState)),
    ListResources((Id<DeploymentType>, ListResourcesState)),
    ResourceProviderInfo(ResourceProviderInfo),
    ListResourceInputs((Id<ResourceType>, Vec<String>)),
    ResourceInputState((Property, ResourceInputState)),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceInputState {
    ResourceInputValue((Property, Value)),
    ResourceInputDependency(ResourceInputDependency),
}

/// Response state for ListResources request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListResourcesState {
    Listed(Vec<String>),
    StructuralDependency(NamedProperty),
}

/// Response state for ListNestedDeployments request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListNestedDeploymentsState {
    Listed(Vec<String>),
    StructuralDependency(NamedProperty),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceProviderInfo {
    pub id: Id<ResourceType>,
    pub provider: Value,
    pub resource_type: String,
    pub state: Option<ResourcePath>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceInputDependency {
    pub dependent: Property,
    pub dependency: NamedProperty,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NamedProperty {
    pub resource: ResourcePath,
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
pub struct DeploymentRequest {
    /// The flake to load the deployment from.
    pub flake: Id<FlakeType>,
    /// The name of the deployment to load.
    pub name: String,
}
impl RequestIdType for DeploymentRequest {
    type IdType = DeploymentType;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NestedDeploymentRequest {
    /// The parent deployment to load the nested deployment from.
    pub parent_deployment: Id<DeploymentType>,
    /// The name of the nested deployment to load.
    pub name: String,
}
impl RequestIdType for NestedDeploymentRequest {
    type IdType = DeploymentType;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRequest {
    /// The deployment to load the resource from.
    pub deployment: Id<DeploymentType>,
    /// The name of the resource to load.
    pub name: String,
}
impl RequestIdType for ResourceRequest {
    type IdType = ResourceType;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceSpec {
    /// Deployment this resource is part of
    pub id: Id<DeploymentType>,
    /// Name of the resource in the deployment
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
    fn test_eval_request_list_deployments() {
        let req = EvalRequest::ListDeployments(QueryRequest {
            message_id: Id::new(2),
            payload: Id::new(1),
            panthom: std::marker::PhantomData,
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
            state: Some(ResourcePath {
                deployment_path: DeploymentPath::root(),
                resource_name: "myStateHandler".to_string(),
            }),
        };

        // Test serialization/deserialization
        let json = serde_json::to_string(&info).unwrap();
        let info2: ResourceProviderInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, info2);

        // Verify state field contains the expected value
        assert_eq!(
            info.state,
            Some(ResourcePath {
                deployment_path: DeploymentPath::root(),
                resource_name: "myStateHandler".to_string(),
            })
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

        // Test that JSON with state field works correctly
        let json_with_state = r#"{
            "id": {"id": 4},
            "provider": {"executable": "/bin/memo", "type": "stdio"},
            "resource_type": "memo",
            "state": {"deployment_path": [], "resource_name": "myState"}
        }"#;

        let info: ResourceProviderInfo = serde_json::from_str(json_with_state).unwrap();
        assert_eq!(
            info.state,
            Some(ResourcePath {
                deployment_path: DeploymentPath::root(),
                resource_name: "myState".to_string(),
            })
        );
        assert_eq!(info.resource_type, "memo");
    }
}
