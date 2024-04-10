use std::collections::HashMap;

use anyhow::{bail, Result};
use nix_expr::{eval_state::EvalState, value::Value};
use nixops4_core::eval_api::{
    AssignRequest, EvalRequest, EvalResponse, Id, IdNum, RequestIdType, SimpleRequest,
};

pub trait Respond {
    fn call(&mut self, response: EvalResponse) -> Result<()>;
}

pub struct EvaluationDriver {
    eval_state: EvalState,
    values: HashMap<IdNum, Value>,
    respond: Box<dyn Respond>,
}
impl EvaluationDriver {
    pub fn new(eval_state: EvalState, respond: Box<dyn Respond>) -> EvaluationDriver {
        EvaluationDriver {
            values: HashMap::new(),
            eval_state,
            respond,
        }
    }

    fn respond(&mut self, response: EvalResponse) -> Result<()> {
        self.respond.call(response)
    }

    fn assign_value<T>(&mut self, id: Id<T>, value: Value) -> Result<()> {
        if let Some(_value) = self.values.get(&id.num()) {
            return self.respond(EvalResponse::Error(
                id.any(),
                "id already used: ".to_string() + &id.num().to_string(),
            ));
        }
        self.values.insert(id.num(), value);
        Ok(())
    }

    pub fn handle_assign_value<T>(
        &mut self,
        id: Id<T>,
        f: impl FnOnce(&mut Self) -> Result<Value>,
    ) -> Result<()> {
        if let Some(_value) = self.values.get(&id.num()) {
            return self.respond(EvalResponse::Error(
                id.any(),
                "id already used: ".to_string() + &id.num().to_string(),
            ));
        }
        let value = f(self);
        match value {
            Ok(value) => {
                self.values.insert(id.num(), value);
                Ok(())
            }
            Err(e) => self.respond(EvalResponse::Error(id.any(), e.to_string())),
        }
    }

    // https://github.com/NixOS/nix/issues/10435
    fn get_flake(&mut self, flakeref_str: &str) -> Result<Value> {
        let get_flake = self
            .eval_state
            .eval_from_string("builtins.getFlake", "<nixops4-eval setup>")?;
        let flakeref = self.eval_state.new_value_str(flakeref_str)?;
        self.eval_state.call(get_flake, flakeref)
    }

    /// Helper function that helps with error handling and saving the result.
    ///
    /// # Parameters:
    /// - handler: do the work. Errors are reported to the client.
    /// - save: save the result. Errors terminate the evaluator process.
    ///
    // We may need more of these helper functions for different types of requests.
    fn handle_assign_request<R: RequestIdType, A>(
        &mut self,
        request: &AssignRequest<R>,
        handler: impl FnOnce(&mut Self, &R) -> Result<A>,
        save: impl FnOnce(&mut Self, Id<R::IdType>, A) -> Result<()>,
    ) -> Result<()> {
        let r = handler(self, &request.payload);
        match r {
            Ok(a) => save(self, request.assign_to, a),
            Err(e) => self.respond(EvalResponse::Error(request.assign_to.any(), e.to_string())),
        }
    }

    /// Helper function that helps with error handling and responding with the result.
    ///
    // We may need more of these helper functions for different types of requests.
    fn handle_simple_request<Req>(
        &mut self,
        request: &SimpleRequest<Req>,
        handler: impl FnOnce(&mut Self, &Req) -> Result<EvalResponse>,
    ) -> Result<()> {
        let r = handler(self, &request.payload);
        match r {
            Ok(resp) => self.respond(resp),
            Err(e) => self.respond(EvalResponse::Error(request.assign_to.any(), e.to_string())),
        }
    }

    pub fn perform_request(&mut self, request: &EvalRequest) -> Result<()> {
        match request {
            EvalRequest::LoadFlake(req) => self.handle_assign_request(
                req,
                |this, req| this.get_flake(req.abspath.as_str()),
                EvaluationDriver::assign_value,
            ),
            EvalRequest::ListDeployments(req) => self.handle_simple_request(req, |this, req| {
                let flake = match this.values.get(&req.num()) {
                    Some(flake) => flake,
                    None => {
                        bail!("flake id not found");
                    }
                };
                let outputs = this.eval_state.require_attrs_select(&flake, "outputs")?;
                let deployments_opt = this
                    .eval_state
                    .require_attrs_select_opt(&outputs, "nixops4Deployments")?;
                let deployments = deployments_opt
                    .map_or(Ok(Vec::new()), |v| this.eval_state.require_attrs_names(&v))?;
                Ok(EvalResponse::ListDeployments(*req, deployments))
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use ctor::ctor;
    use nix_expr::eval_state::{gc_registering_current_thread, EvalState};
    use nix_store::store::Store;
    use nixops4_core::eval_api::{AssignRequest, FlakeRequest, Ids, SimpleRequest};
    use tempdir::TempDir;

    struct TestRespond {
        responses: Arc<Mutex<Vec<EvalResponse>>>,
    }
    impl Respond for TestRespond {
        fn call(&mut self, response: EvalResponse) -> Result<()> {
            let mut responses = self.responses.lock().unwrap();
            responses.push(response);
            Ok(())
        }
    }

    #[ctor]
    fn setup() {
        nix_expr::eval_state::test_init();
        nix_util::settings::set("experimental-features", "flakes").unwrap();
    }

    #[test]
    fn test_eval_driver_invalid_flakeref() {
        gc_registering_current_thread(|| -> Result<()> {
            let store = Store::open("auto", [])?;
            let eval_state = EvalState::new(store, [])?;
            let responses: Arc<Mutex<Vec<EvalResponse>>> = Default::default();
            let respond = Box::new(TestRespond {
                responses: responses.clone(),
            });
            let mut driver = EvaluationDriver::new(eval_state, respond);

            let flake_request = FlakeRequest {
                abspath: "/non-existent/path/to/flake".to_string(),
            };
            let mut ids = Ids::new();
            let flake_id = ids.next();
            let assign_request = AssignRequest {
                assign_to: flake_id,
                payload: flake_request,
            };
            let request = EvalRequest::LoadFlake(assign_request);
            driver.perform_request(&request)?;
            {
                let r = responses.lock().unwrap();
                assert_eq!(r.len(), 1);
                match &r[0] {
                    EvalResponse::Error(id, msg) => {
                        assert_eq!(id, &flake_id.any());
                        if msg.contains("/non-existent/path/to/flake") {
                            return Ok(());
                        } else {
                            panic!("unexpected error message: {}", msg);
                        }
                    }
                    _ => panic!("expected EvalResponse::Error"),
                };
            }
        })
        .unwrap()
        .unwrap();
    }

    #[test]
    fn test_eval_driver_empty_flake() {
        generic_test_eval_driver_empty_flake(
            r#"
            {
                outputs = { ... }: {
                };
            }
        "#,
        );
    }

    #[test]
    fn test_eval_driver_empty_flake2() {
        generic_test_eval_driver_empty_flake(
            r#"
            {
                outputs = { ... }: {
                    nixops4Deployments = {
                    };
                };
            }
        "#,
        );
    }

    fn generic_test_eval_driver_empty_flake(flake_nix: &str) {
        let tmpdir = TempDir::new("test-nixops4-eval").unwrap();
        // write flake.nix
        let flake_path = tmpdir.path().join("flake.nix");
        std::fs::write(&flake_path, flake_nix).unwrap();

        gc_registering_current_thread(|| -> Result<()> {
            let store = Store::open("auto", [])?;
            let eval_state = EvalState::new(store, [])?;
            let responses: Arc<Mutex<Vec<EvalResponse>>> = Default::default();
            let respond = Box::new(TestRespond {
                responses: responses.clone(),
            });
            let mut driver = EvaluationDriver::new(eval_state, respond);

            let flake_request = FlakeRequest {
                abspath: tmpdir.path().to_str().unwrap().to_string(),
            };
            let mut ids = Ids::new();
            let flake_id = ids.next();
            let deployments_id = ids.next();
            let assign_request = AssignRequest {
                assign_to: flake_id,
                payload: flake_request,
            };
            driver.perform_request(&EvalRequest::LoadFlake(assign_request))?;
            {
                let r = responses.lock().unwrap();
                if r.len() != 0 {
                    panic!("expected 0 responses, got: {:?}", r);
                }
            }
            driver.perform_request(&EvalRequest::ListDeployments(SimpleRequest {
                assign_to: deployments_id,
                payload: flake_id,
            }))?;
            {
                let r = responses.lock().unwrap();
                if r.len() != 1 {
                    panic!("expected 1 response, got: {:?}", r);
                }
                match &r[0] {
                    EvalResponse::ListDeployments(id, names) => {
                        eprintln!("id: {:?}, names: {:?}", id, names);
                        assert_eq!(id, &flake_id);
                        assert_eq!(names.len(), 0);
                    }
                    _ => panic!("expected EvalResponse::ListResources"),
                }
            }

            Ok(())
        })
        .unwrap()
        .unwrap();
    }

    #[test]
    fn test_eval_driver_flake_deployments_throw() {
        let flake_nix = r#"
            {
                outputs = { ... }: {
                    nixops4Deployments = throw "so this is the error message from the nixops4Deployments attribute value";
                };
            }
        "#;

        let tmpdir = TempDir::new("test-nixops4-eval").unwrap();
        let flake_path = tmpdir.path().join("flake.nix");
        std::fs::write(&flake_path, flake_nix).unwrap();

        gc_registering_current_thread(|| {
            let store = Store::open("auto", []).unwrap();
            let eval_state = EvalState::new(store, []).unwrap();
            let responses: Arc<Mutex<Vec<EvalResponse>>> = Default::default();
            let respond = Box::new(TestRespond {
                responses: responses.clone(),
            });
            let mut driver = EvaluationDriver::new(eval_state, respond);

            let flake_request = FlakeRequest {
                abspath: tmpdir.path().to_str().unwrap().to_string(),
            };
            let mut ids = Ids::new();
            let flake_id = ids.next();
            let deployments_id = ids.next();
            let assign_request = AssignRequest {
                assign_to: flake_id,
                payload: flake_request,
            };
            driver
                .perform_request(&EvalRequest::LoadFlake(assign_request))
                .unwrap();
            driver
                .perform_request(&EvalRequest::ListDeployments(SimpleRequest{ assign_to : deployments_id, payload : flake_id }))
                .unwrap();
            {
                let r = responses.lock().unwrap();
                if r.len() != 1 {
                    panic!("expected 1 response, got: {:?}", r);
                }
                match &r[0] {
                    EvalResponse::Error(id, msg) => {
                        assert_eq!(id, &deployments_id.any());
                        if !msg.contains("so this is the error message from the nixops4Deployments attribute value") {
                            panic!("unexpected error message: {}", msg);
                        }
                    }
                    _ => panic!("expected EvalResponse::Error"),
                }
            }
        })
        .unwrap()
    }

    #[test]
    fn test_eval_driver_flake_skeleton_lazy() {
        let flake_nix = r#"
            {
                outputs = { ... }: {
                    nixops4Deployments = {
                        a = throw "do not evaluate a";
                        b = throw "do not evaluate b";
                    };
                };
            }
            "#;

        let tmpdir = TempDir::new("test-nixops4-eval").unwrap();
        let flake_path = tmpdir.path().join("flake.nix");
        std::fs::write(&flake_path, flake_nix).unwrap();

        gc_registering_current_thread(|| {
            let store = Store::open("auto", []).unwrap();
            let eval_state = EvalState::new(store, []).unwrap();
            let responses: Arc<Mutex<Vec<EvalResponse>>> = Default::default();
            let respond = Box::new(TestRespond {
                responses: responses.clone(),
            });
            let mut driver = EvaluationDriver::new(eval_state, respond);

            let flake_request = FlakeRequest {
                abspath: tmpdir.path().to_str().unwrap().to_string(),
            };
            let mut ids = Ids::new();
            let flake_id = ids.next();
            let deployments_id = ids.next();
            let assign_request = AssignRequest {
                assign_to: flake_id,
                payload: flake_request,
            };
            driver
                .perform_request(&EvalRequest::LoadFlake(assign_request))
                .unwrap();
            {
                let r = responses.lock().unwrap();
                if r.len() != 0 {
                    panic!("expected 0 responses, got: {:?}", r);
                }
            }
            driver
                .perform_request(&EvalRequest::ListDeployments(SimpleRequest {
                    assign_to: deployments_id,
                    payload: flake_id,
                }))
                .unwrap();
            {
                let r = responses.lock().unwrap();
                if r.len() != 1 {
                    panic!("expected 1 response, got: {:?}", r);
                }
                match &r[0] {
                    EvalResponse::ListDeployments(id, names) => {
                        eprintln!("id: {:?}, names: {:?}", id, names);
                        assert_eq!(id, &flake_id);
                        assert_eq!(names.len(), 2);
                        assert_eq!(names[0], "a");
                        assert_eq!(names[1], "b");
                    }
                    _ => panic!("expected EvalResponse::ListDeployments"),
                }
            }
        })
        .unwrap()
    }
}
