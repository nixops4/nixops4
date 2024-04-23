use std::collections::HashMap;

use anyhow::{bail, Result};
use nix_expr::{
    eval_state::{EvalState, FUNCTION_ANONYMOUS},
    value::Value,
};
use nixops4_core::eval_api::{
    AssignRequest, EvalRequest, EvalResponse, FlakeType, Id, IdNum, RequestIdType, SimpleRequest,
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

    fn get_value<T>(&self, id: Id<T>) -> Result<&Value> {
        self.values
            .get(&id.num())
            .ok_or_else(|| anyhow::anyhow!("id not found: {}", id.num().to_string()))
    }

    fn get_flake_deployments_value(&self, flake: Id<FlakeType>) -> Result<Value> {
        let flake = self.get_value(flake)?;
        let outputs = self.eval_state.require_attrs_select(&flake, "outputs")?;
        let deployments = self
            .eval_state
            .require_attrs_select(&outputs, "nixops4Deployments")?;
        Ok(deployments.clone())
    }

    pub fn perform_request(&mut self, request: &EvalRequest) -> Result<()> {
        match request {
            EvalRequest::LoadFlake(req) => self.handle_assign_request(
                req,
                |this, req| this.get_flake(req.abspath.as_str()),
                EvaluationDriver::assign_value,
            ),
            EvalRequest::ListDeployments(req) => self.handle_simple_request(req, |this, req| {
                let flake = this.get_value(req.to_owned())?;
                let outputs = this.eval_state.require_attrs_select(&flake, "outputs")?;
                let deployments_opt = this
                    .eval_state
                    .require_attrs_select_opt(&outputs, "nixops4Deployments")?;
                let deployments = deployments_opt
                    .map_or(Ok(Vec::new()), |v| this.eval_state.require_attrs_names(&v))?;
                Ok(EvalResponse::ListDeployments(*req, deployments))
            }),
            EvalRequest::LoadDeployment(req) => self.handle_assign_request(
                req,
                |this, req| {
                    // basic lookup
                    let es = &this.eval_state;
                    let deployments = this.get_flake_deployments_value(req.flake)?;
                    let deployment = es.require_attrs_select(&deployments, &req.name)?;

                    // check _type attr
                    {
                        let tag = es.require_attrs_select(&deployment, "_type")?;
                        let str = es.require_string(&tag)?;
                        if str != "nixops4Deployment" {
                            bail!("expected _type to be 'nixops4Deployment', got: {}", str);
                        }
                    }

                    let eval_expr = r#"
                        # primops
                        loadResourceAttr:
                        loadResourceSkeleton:
                        # user expr
                        deploymentFunction:
                        let
                          arg = {
                            inherit resources;
                          };
                          resources =
                            builtins.mapAttrs
                              (name: value:
                                builtins.mapAttrs
                                  (loadResourceAttr name)
                                  (loadResourceSkeleton name value)
                              )
                              fixpoint.deployment.resources;
                          fixpoint = deploymentFunction arg;
                        in
                          fixpoint
                    "#;

                    let deployment_function =
                        es.require_attrs_select(&deployment, "deploymentFunction")?;

                    let load_resource_attr = es.new_value_function(
                        FUNCTION_ANONYMOUS.as_ptr(),
                        Box::new(|es, [resource_name, attr_name, _]| {
                            let resource_name = es.require_string(resource_name)?;
                            let attr_name = es.require_string(attr_name)?;
                            es.new_value_str(format!("{}.{}", resource_name, attr_name).as_str())
                        }),
                    )?;

                    let load_resource_skeleton = es.new_value_function(
                        FUNCTION_ANONYMOUS.as_ptr(),
                        Box::new(|es, [resource_name, value, _]| {
                            let resource_name = es.require_string(resource_name)?;
                            es.new_value_str(format!("{}.skeleton", resource_name).as_str())
                        }),
                    )?;

                    // invoke it
                    let fixpoint = {
                        let v = es.eval_from_string(eval_expr, "<nixops4 internals>")?;
                        let v = es.call(v, load_resource_attr)?;
                        let v = es.call(v, load_resource_skeleton)?;
                        es.call(v, deployment_function)
                    }?;

                    Ok(fixpoint.clone())
                },
                EvaluationDriver::assign_value,
            ),
            _ => unimplemented!(),
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
    use nixops4_core::eval_api::{
        AssignRequest, DeploymentRequest, FlakeRequest, Ids, SimpleRequest,
    };
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

    #[test]
    fn test_eval_driver_flake_example() {
        let flake_nix = r#"
            {
                outputs = { self, ... }: {
                    nixops4Deployments = {
                        example = {
                            _type = "nixops4Deployment";
                            deploymentFunction = { resources }:
                            {
                                resources = {
                                    a = {
                                        _type = "nixops4SimpleResource";
                                        exe = "__test:dummy";
                                        inputs = {
                                            foo = "bar";
                                        };
                                    };
                                    b = {
                                        _type = "nixops4SimpleResource";
                                        exe = "__test:dummy";
                                        inputs = {
                                            qux = resources.a.foo2;
                                        };
                                    };
                                };
                            };
                        };
                    };
                };
            }
            "#;

        let tmpdir = TempDir::new("test-nixops4-eval").unwrap();
        let flake_path = tmpdir.path().join("flake.nix");
        std::fs::write(&flake_path, flake_nix).unwrap();

        gc_registering_current_thread(|| {
            let store = Store::open("auto").unwrap();
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
            let deployment_id = ids.next();
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
                .perform_request(&EvalRequest::LoadDeployment(AssignRequest {
                    assign_to: deployment_id,
                    payload: DeploymentRequest {
                        flake: flake_id,
                        name: "example".to_string(),
                    },
                }))
                .unwrap();
            {
                let r = responses.lock().unwrap();
                if r.len() != 0 {
                    panic!("expected 0 responses, got: {:?}", r);
                }
            }
        })
        .unwrap()
    }
}
