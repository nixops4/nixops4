use std::collections::HashMap;

use anyhow::{bail, Result};
use base64::engine::Engine;
use cstr::cstr;
use nix_expr::{
    eval_state::EvalState,
    primop::{PrimOp, PrimOpMeta},
    value::Value,
};
use nixops4_core::eval_api::{
    AssignRequest, EvalRequest, EvalResponse, FlakeType, Id, IdNum, NamedProperty, RequestIdType,
    ResourceInputDependency, ResourceProviderInfo, SimpleRequest,
};
use std::sync::{Arc, Mutex};

pub trait Respond {
    fn call(&mut self, response: EvalResponse) -> Result<()>;
}

pub struct EvaluationDriver {
    eval_state: EvalState,
    values: HashMap<IdNum, Value>,
    respond: Box<dyn Respond>,
    known_outputs: Arc<Mutex<HashMap<NamedProperty, Value>>>,
}
impl EvaluationDriver {
    pub fn new(eval_state: EvalState, respond: Box<dyn Respond>) -> EvaluationDriver {
        EvaluationDriver {
            values: HashMap::new(),
            eval_state,
            respond,
            known_outputs: Arc::new(Mutex::new(HashMap::new())),
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
        // TODO: replace with native functionality through C API, see issue #10435, linked above

        // Avoid copying everything, including target/ and .git/ directories.
        // Check for a .git directory in the path.
        let flakeref_str = if std::path::Path::new(flakeref_str).join(".git").exists() {
            format!("git+file://{}", flakeref_str)
        } else {
            flakeref_str.to_string()
        };

        let flakeref = self.eval_state.new_value_str(flakeref_str.as_str())?;
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

    fn get_flake_deployments_value(&mut self, flake: Id<FlakeType>) -> Result<Value> {
        let flake = self.get_value(flake)?.clone();
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
                let flake = this.get_value(req.to_owned())?.clone();
                let outputs = this.eval_state.require_attrs_select(&flake, "outputs")?;
                let deployments_opt = this
                    .eval_state
                    .require_attrs_select_opt(&outputs, "nixops4Deployments")?;
                let deployments = deployments_opt
                    .map_or(Ok(Vec::new()), |v| this.eval_state.require_attrs_names(&v))?;
                Ok(EvalResponse::ListDeployments(*req, deployments))
            }),
            EvalRequest::LoadDeployment(req) => {
                let known_outputs = Arc::clone(&self.known_outputs);
                self.handle_assign_request(
                    req,
                    |this, req| {
                        // basic lookup
                        let deployments = { this.get_flake_deployments_value(req.flake)? }.clone();
                        let es = &mut this.eval_state;
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
                                  value.provider.types.${value.type}.outputs
                              )
                              (builtins.trace (builtins.attrNames fixpoint)
                              fixpoint.resources);
                          fixpoint = deploymentFunction arg;
                        in
                          fixpoint
                    "#;

                        let deployment_function =
                            es.require_attrs_select(&deployment, "deploymentFunction")?;

                        let prim_load_resource_attr = PrimOp::new(
                            es,
                            PrimOpMeta {
                                name: cstr!("nixopsLoadResourceAttr"),
                                doc: cstr!("Internal function that loads a resource attribute."),
                                args: [cstr!("resourceName"), cstr!("attrName"), cstr!("ignored")],
                            },
                            Box::new(move |es, [resource_name, attr_name, _]| {
                                let resource_name = es.require_string(resource_name)?;
                                let attr_name = es.require_string(attr_name)?;
                                let property = NamedProperty {
                                    resource: resource_name.to_string(),
                                    name: attr_name.to_string(),
                                };
                                let val = {
                                    let known_outputs = known_outputs.lock().unwrap();
                                    known_outputs.get(&property).map(Clone::clone)
                                };
                                match val {
                                    Some(val) => Ok(val),
                                    None =>
                                    // FIXME: add custom errors to the Nix C API, or at least don't put arbitrary length data here
                                    //        perhaps a number that refers to a hashmap?
                                    // FIXME: this will probably leak memory when accessing outputs before all providers are loaded, etc
                                    {
                                        Err(anyhow::anyhow!(
                                            "__internal_exception_load_resource_property_#{}#",
                                            base64::engine::general_purpose::STANDARD.encode(
                                                serde_json::to_string(&NamedProperty {
                                                    resource: resource_name,
                                                    name: attr_name
                                                })
                                                .unwrap()
                                            ),
                                        ))
                                    }
                                }
                            }),
                        )?;

                        let load_resource_attr = es.new_value_primop(prim_load_resource_attr)?;

                        // invoke it
                        let fixpoint = {
                            let v = es.eval_from_string(eval_expr, "<nixops4 internals>")?;
                            let v = es.call(v, load_resource_attr)?;
                            es.call(v, deployment_function)
                        }?;

                        Ok(fixpoint.clone())
                    },
                    EvaluationDriver::assign_value,
                )
            }
            EvalRequest::ListResources(req) => self.handle_simple_request(req, |this, req| {
                let deployment = this.get_value(req.to_owned())?.clone();
                let resources_attrset = this
                    .eval_state
                    .require_attrs_select(&deployment, "resources")?;
                let resources = this.eval_state.require_attrs_names(&resources_attrset)?;
                Ok(EvalResponse::ListResources(*req, resources))
            }),
            EvalRequest::LoadResource(req) => self.handle_assign_request(
                req,
                |this, req| {
                    let deployment = this.get_value(req.deployment)?.clone();
                    let resources_attrset = this
                        .eval_state
                        .require_attrs_select(&deployment, "resources")?;
                    let resource = this
                        .eval_state
                        .require_attrs_select(&resources_attrset, &req.name)?;
                    Ok(resource.clone())
                },
                EvaluationDriver::assign_value,
            ),
            EvalRequest::GetResource(req) => self.handle_simple_request(req, |this, req| {
                let resource = this.get_value(req.to_owned())?.clone();
                // let resource_api = this.eval_state.require_attrs_select(&resource, "_type")?;
                let provider_value = this
                    .eval_state
                    .require_attrs_select(&resource, "provider")?;
                let provider_json = value_to_json(&mut this.eval_state, &provider_value)?;
                let resource_type_value =
                    this.eval_state.require_attrs_select(&resource, "type")?;
                let resource_type_str = this.eval_state.require_string(&resource_type_value)?;
                Ok(EvalResponse::ResourceProviderInfo(ResourceProviderInfo {
                    id: req.to_owned(),
                    provider: provider_json,
                    resource_type: resource_type_str,
                }))
            }),
            EvalRequest::ListResourceInputs(req) => self.handle_simple_request(req, |this, req| {
                let resource = this.get_value(req.to_owned())?.clone();
                let inputs = this.eval_state.require_attrs_select(&resource, "inputs")?;
                let inputs = this.eval_state.require_attrs_names(&inputs)?;
                Ok(EvalResponse::ResourceInputs(*req, inputs))
            }),
            EvalRequest::GetResourceInput(req) => self.handle_simple_request(req, |this, req| {
                let attempt: Result<serde_json::Value, anyhow::Error> = (|| {
                    let resource = this.get_value(req.resource.to_owned())?.clone();
                    let inputs = this.eval_state.require_attrs_select(&resource, "inputs")?;
                    let input = this.eval_state.require_attrs_select(&inputs, &req.name)?;
                    let json = value_to_json(&mut this.eval_state, &input)?;
                    Ok(json)
                })();
                match attempt {
                    Ok(json) => Ok(EvalResponse::ResourceInputValue(req.to_owned(), json)),
                    Err(e) => {
                        let s = e.to_string();
                        if s.contains("__internal_exception_load_resource_property_#") {
                            let base64_str = s
                                .split("__internal_exception_load_resource_property_#")
                                .collect::<Vec<&str>>()[1]
                                .split("#")
                                .collect::<Vec<&str>>()[0];
                            let json_str =
                                base64::engine::general_purpose::STANDARD.decode(base64_str)?;
                            let named_property: NamedProperty = serde_json::from_slice(&json_str)?;
                            Ok(EvalResponse::ResourceInputDependency(
                                ResourceInputDependency {
                                    dependent: req.to_owned(),
                                    dependency: named_property,
                                },
                            ))
                        } else {
                            Err(e)
                        }
                    }
                }

                // Ok(EvalResponse::ResourceInputValue(req.to_owned(), json))
            }),
            EvalRequest::PutResourceOutput(named_prop, value) => {
                let value = json_to_value(&mut self.eval_state, value)?;
                {
                    self.known_outputs
                        .lock()
                        .unwrap()
                        .insert(named_prop.clone(), value);
                }
                Ok(())
            } // _ => unimplemented!(),
        }
    }
}

fn value_to_json(eval_state: &mut EvalState, value: &Value) -> Result<serde_json::Value> {
    let to_json = eval_state.eval_from_string("builtins.toJSON", "<nixops4-eval GetResource>")?;
    let json_str_value = eval_state.call(to_json, value.clone())?;
    let json_str = eval_state.realise_string(&json_str_value, false)?;
    let json = serde_json::from_str(&json_str.s)?;
    Ok(json)
}

fn json_to_value(eval_state: &mut EvalState, json: &serde_json::Value) -> Result<Value> {
    let from_json =
        eval_state.eval_from_string("builtins.fromJSON", "<nixops4-eval GetResource>")?;
    let json_str = serde_json::to_string(json)?;
    let json_str_value = eval_state.new_value_str(json_str.as_str())?;
    let value = eval_state.call(from_json, json_str_value)?;
    Ok(value)
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
