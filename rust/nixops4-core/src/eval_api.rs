use std::hash::{Hash, Hasher};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub struct Ids {
    counter: u64,
}
impl Ids {
    pub fn new() -> Self {
        Ids { counter: 0 }
    }
    pub fn next<T>(&mut self) -> Id<T> {
        let id = self.counter;
        self.counter += 1;
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
        self.id.partial_cmp(&other.id)
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

/// This interface is internal to NixOps4. It is used to communicate between the CLI and the evaluator.
/// Only matching CLI and evaluator versions are compatible.
/// No promises are made about this interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvalRequest {
    LoadFlake(AssignRequest<FlakeRequest>),
    ListDeployments(QueryRequest<Id<FlakeType>, (Id<FlakeType>, Vec<String>)>),
    LoadDeployment(AssignRequest<DeploymentRequest>),
    ListResources(QueryRequest<Id<DeploymentType>, (Id<DeploymentType>, Vec<String>)>),
    LoadResource(AssignRequest<ResourceRequest>),
    GetResource(QueryRequest<Id<ResourceType>, ResourceProviderInfo>),
    ListResourceInputs(QueryRequest<Id<ResourceType>, (Id<ResourceType>, Vec<String>)>),
    GetResourceInput(QueryRequest<Property, ResourceInputState>),
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryResponseValue {
    ListDeployments((Id<FlakeType>, Vec<String>)),
    ListResources((Id<DeploymentType>, Vec<String>)),
    ResourceProviderInfo(ResourceProviderInfo),
    ListResourceInputs((Id<ResourceType>, Vec<String>)),
    ResourceInputState((Property, ResourceInputState)),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceInputState {
    ResourceInputValue((Property, Value)),
    ResourceInputDependency(ResourceInputDependency),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceProviderInfo {
    pub id: Id<ResourceType>,
    pub provider: Value,
    pub resource_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceInputDependency {
    pub dependent: Property,
    pub dependency: NamedProperty,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NamedProperty {
    pub resource: String,
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
        let mut ids = Ids::new();
        let id1: Id<FlakeType> = ids.next();
        let id2 = ids.next();
        assert_ne!(id1, id2);
        assert_eq!(id1.num() + 1, id2.num());
    }

    #[test]
    fn test_id_any() {
        let mut ids = Ids::new();
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
}
