use std::hash::{Hash, Hasher};

use anyhow::Result;
use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnyType;
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
    LoadFlake(AssignRequest<FlakeType, FlakeRequest>),
    ListDeployments(Id<FlakeType>),
    // ListResources(id),
    // LoadResource(id, String),
}

// TODO: probably better to use identifiers for _all_ requests; simplifies error handling which. I've got it wrong because the data structure doesn't match the code very well - the code should be more general and handle errors near the main loop instead of in each individual request handler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignRequest<T, R> {
    /// Unique id provided by the client.
    pub assign_to: Id<T>,
    pub payload: R,
}

/// This interface is internal to NixOps4. It is used to communicate between the CLI and the evaluator.
/// Only matching CLI and evaluator versions are compatible.
/// No promises are made about this interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvalResponse {
    Error(Id<AnyType>, String),
    ListDeployments(Id<FlakeType>, Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlakeRequest {
    /// The path to the flake to load.
    pub abspath: String,
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
        let req = EvalRequest::ListDeployments(Id::new(1));
        let s = eval_request_to_json(&req).unwrap();
        eprintln!("{}", s);
        let req2 = eval_request_from_json(&s).unwrap();
        assert_eq!(req, req2);
    }
}
