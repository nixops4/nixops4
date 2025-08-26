use std::collections::HashMap;

use anyhow::{bail, Result};
use base64::engine::Engine;
use cstr::cstr;
use nix_bindings_expr::{
    eval_state::EvalState,
    primop::{PrimOp, PrimOpMeta},
    value::{Value, ValueType},
};
use nixops4_core::eval_api::{
    AssignRequest, EvalRequest, EvalResponse, FlakeType, Id, IdNum, NamedProperty, QueryRequest,
    QueryResponseValue, RequestIdType, ResourceInputDependency, ResourceInputState,
    ResourceProviderInfo, ResourceType,
};
use std::cell::RefCell;
use std::rc::Rc;

pub trait Respond {
    fn call(
        &mut self,
        response: EvalResponse,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}

pub struct EvaluationDriver<R: Respond> {
    eval_state: EvalState,
    fetch_settings: nix_bindings_fetchers::FetchersSettings,
    flake_settings: nix_bindings_flake::FlakeSettings,
    values: HashMap<IdNum, Result<Value, String>>,
    respond: R,
    known_outputs: Rc<RefCell<HashMap<NamedProperty, Value>>>,
    resource_names: HashMap<Id<ResourceType>, String>,
}
impl<R: Respond> EvaluationDriver<R> {
    pub fn new(
        eval_state: EvalState,
        fetch_settings: nix_bindings_fetchers::FetchersSettings,
        flake_settings: nix_bindings_flake::FlakeSettings,
        respond: R,
    ) -> EvaluationDriver<R> {
        EvaluationDriver {
            values: HashMap::new(),
            eval_state,
            fetch_settings,
            flake_settings,
            respond,
            known_outputs: Rc::new(RefCell::new(HashMap::new())),
            resource_names: HashMap::new(),
        }
    }

    async fn respond(&mut self, response: EvalResponse) -> Result<()> {
        self.respond.call(response).await
    }

    fn get_flake(
        &mut self,
        flakeref_str: &str,
        input_overrides: &Vec<(String, String)>,
    ) -> Result<Value> {
        let mut parse_flags =
            nix_bindings_flake::FlakeReferenceParseFlags::new(&self.flake_settings)?;

        let cwd = std::env::current_dir()
            .map_err(|e| anyhow::anyhow!("failed to get current directory: {}", e))?;
        let cwd = cwd
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("failed to convert current directory to string"))?;
        parse_flags.set_base_directory(cwd)?;

        let parse_flags = parse_flags;

        let mut lock_flags = nix_bindings_flake::FlakeLockFlags::new(&self.flake_settings)?;
        lock_flags.set_mode_write_as_needed()?;
        for (override_path, override_ref_str) in input_overrides {
            let (override_ref, fragment) = nix_bindings_flake::FlakeReference::parse_with_fragment(
                &self.fetch_settings,
                &self.flake_settings,
                &parse_flags,
                override_ref_str,
            )?;
            if !fragment.is_empty() {
                bail!(
                    "input override {} has unexpected fragment: {}",
                    override_path,
                    fragment
                );
            }
            lock_flags.add_input_override(override_path, &override_ref)?;
        }
        let lock_flags = lock_flags;

        let (flakeref, fragment) = nix_bindings_flake::FlakeReference::parse_with_fragment(
            &self.fetch_settings,
            &self.flake_settings,
            &parse_flags,
            flakeref_str,
        )?;
        if !fragment.is_empty() {
            bail!(
                "flake reference {} has unexpected fragment: {}",
                flakeref_str,
                fragment
            );
        }
        let flake = nix_bindings_flake::LockedFlake::lock(
            &self.fetch_settings,
            &self.flake_settings,
            &self.eval_state,
            &lock_flags,
            &flakeref,
        )?;

        flake.outputs(&self.flake_settings, &mut self.eval_state)
    }

    /// Helper function for assignment requests that handles error propagation.
    ///
    /// # Parameters:
    /// - handler: Performs the work and returns a Value. Errors are stored and reported to the client.
    ///
    // We may need more of these helper functions for different types of requests.
    async fn handle_assign_request<T: RequestIdType>(
        &mut self,
        request: &AssignRequest<T>,
        handler: impl FnOnce(&mut Self, &T) -> Result<Value>,
    ) -> Result<()> {
        // Check for duplicate ID assignment
        if let Some(_value) = self.values.get(&request.assign_to.num()) {
            let error_msg = format!("id already used: {}", request.assign_to.num());
            self.respond(EvalResponse::Error(request.assign_to.any(), error_msg))
                .await?;
            return Ok(());
        }

        let r = handler(self, &request.payload);
        match r {
            Ok(value) => {
                self.values.insert(request.assign_to.num(), Ok(value));
                Ok(())
            }
            Err(e) => {
                let error_msg = e.to_string();
                self.values
                    .insert(request.assign_to.num(), Err(error_msg.clone()));
                self.respond(EvalResponse::Error(request.assign_to.any(), error_msg))
                    .await
            }
        }
    }

    /// Helper function that helps with error handling and responding with the result.
    ///
    // We may need more of these helper functions for different types of requests.
    async fn handle_simple_request<Req, Resp>(
        &mut self,
        request: &QueryRequest<Req, Resp>,
        make_response: impl FnOnce(Resp) -> QueryResponseValue,
        handler: impl FnOnce(&mut Self, &Req) -> Result<Resp>,
    ) -> Result<()> {
        let rid = request.message_id;
        let r = handler(self, &request.payload);
        match r {
            Ok(resp) => {
                self.respond(EvalResponse::QueryResponse(rid, make_response(resp)))
                    .await
            }
            Err(e) => {
                self.respond(EvalResponse::Error(request.message_id.any(), e.to_string()))
                    .await
            }
        }
    }

    fn get_value<T>(&self, id: Id<T>) -> Result<&Value> {
        match self.values.get(&id.num()) {
            Some(Ok(value)) => Ok(value),
            Some(Err(error)) => Err(anyhow::anyhow!("{}", error)),
            None => Err(anyhow::anyhow!("id not found: {}", id.num().to_string())),
        }
    }

    fn get_flake_deployments_value(&mut self, flake: Id<FlakeType>) -> Result<Value> {
        let flake = self.get_value(flake)?.clone();
        let outputs = self.eval_state.require_attrs_select(&flake, "outputs")?;
        let deployments = self
            .eval_state
            .require_attrs_select(&outputs, "nixops4Deployments")?;
        Ok(deployments.clone())
    }

    pub async fn perform_request(&mut self, request: &EvalRequest) -> Result<()> {
        match request {
            EvalRequest::LoadFlake(req) => {
                self.handle_assign_request(req, |this, req| {
                    this.get_flake(req.abspath.as_str(), &req.input_overrides)
                })
                .await
            }
            EvalRequest::ListDeployments(req) => {
                self.handle_simple_request(req, QueryResponseValue::ListDeployments, |this, req| {
                    let flake = this.get_value(req.to_owned())?.clone();
                    let outputs = this.eval_state.require_attrs_select(&flake, "outputs")?;
                    let deployments_opt = this
                        .eval_state
                        .require_attrs_select_opt(&outputs, "nixops4Deployments")?;
                    let deployments = deployments_opt
                        .map_or(Ok(Vec::new()), |v| this.eval_state.require_attrs_names(&v))?;
                    Ok((*req, deployments))
                })
                .await
            }
            EvalRequest::LoadDeployment(req) => {
                let known_outputs = Rc::clone(&self.known_outputs);
                self.handle_assign_request(req, |this, req| {
                    perform_load_deployment(this, req, known_outputs)
                })
                .await
            }
            EvalRequest::ListResources(req) => {
                self.handle_simple_request(req, QueryResponseValue::ListResources, |this, req| {
                    let deployment = this.get_value(req.to_owned())?.clone();
                    let resources_attrset = this
                        .eval_state
                        .require_attrs_select(&deployment, "resources")?;
                    let resources = this.eval_state.require_attrs_names(&resources_attrset)?;
                    Ok((*req, resources))
                })
                .await
            }
            EvalRequest::LoadResource(areq) => {
                self.handle_assign_request(areq, |this, req| {
                    let deployment = this.get_value(req.deployment)?.clone();
                    let resources_attrset = this
                        .eval_state
                        .require_attrs_select(&deployment, "resources")?;
                    let resource = this
                        .eval_state
                        .require_attrs_select(&resources_attrset, &req.name)?;
                    this.resource_names.insert(areq.assign_to, req.name.clone());
                    Ok(resource)
                })
                .await
            }
            EvalRequest::GetResource(req) => {
                self.handle_simple_request(
                    req,
                    QueryResponseValue::ResourceProviderInfo,
                    perform_get_resource,
                )
                .await
            }
            EvalRequest::ListResourceInputs(req) => {
                self.handle_simple_request(
                    req,
                    QueryResponseValue::ListResourceInputs,
                    |this, req| {
                        let resource = this.get_value(req.to_owned())?.clone();
                        let inputs = this.eval_state.require_attrs_select(&resource, "inputs")?;
                        let inputs = this.eval_state.require_attrs_names(&inputs)?;
                        Ok((*req, inputs))
                    },
                )
                .await
            }
            EvalRequest::GetResourceInput(req) => {
                self.handle_simple_request(
                    req,
                    |x| QueryResponseValue::ResourceInputState((req.payload.clone(), x)),
                    perform_get_resource_input,
                )
                .await
            }
            EvalRequest::PutResourceOutput(named_prop, value) => {
                let value = json_to_value(&mut self.eval_state, value)?;
                {
                    self.known_outputs
                        .borrow_mut()
                        .insert(named_prop.clone(), value);
                }
                Ok(())
            } // _ => unimplemented!(),
        }
    }
}

fn perform_load_deployment<R: Respond>(
    driver: &mut EvaluationDriver<R>,
    req: &nixops4_core::eval_api::DeploymentRequest,
    known_outputs: Rc<RefCell<HashMap<NamedProperty, Value>>>,
) -> Result<Value, anyhow::Error> {
    let deployments = { driver.get_flake_deployments_value(req.flake)? }.clone();
    let es = &mut driver.eval_state;
    let deployment = es.require_attrs_select(&deployments, &req.name)?;
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
                        # other args, such as resourceProviderSystem
                        extraArgs:
                        let
                          arg = {
                            inherit resources;
                          } // extraArgs;
                          resources =
                            builtins.mapAttrs
                              (name: value:
                                builtins.mapAttrs
                                  (loadResourceAttr name)
                                  (value.outputsSkeleton
                                    or value.provider.types.${value.type}.outputs
                                    or (throw "Resource ${name} does not declare its outputs. It is currently required for resources to declare their outputs. This is an implementation error in the resource provider."))
                              )
                              fixpoint.resources;
                          fixpoint = deploymentFunction arg;
                        in
                          fixpoint
                    "#;
    let deployment_function = es.require_attrs_select(&deployment, "deploymentFunction")?;
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
                resource: nixops4_core::eval_api::ResourcePath(resource_name.to_string()),
                name: attr_name.to_string(),
            };
            let val = { known_outputs.borrow().get(&property).cloned() };
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
                                resource: nixops4_core::eval_api::ResourcePath(resource_name),
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
    // let extra_args = es.new_value_attrs(HashMap::new())?;
    let resource_provider_system = nix_bindings_util::settings::get("system")?;
    let resource_provider_system_value = es.new_value_str(resource_provider_system.as_str())?;
    let extra_args = es.new_value_attrs([(
        "resourceProviderSystem".to_string(),
        resource_provider_system_value,
    )])?;

    let fixpoint = {
        let v = es.eval_from_string(eval_expr, "<nixops4 internals>")?;
        es.call_multi(&v, &[load_resource_attr, deployment_function, extra_args])
    }?;
    Ok(fixpoint)
}

fn perform_get_resource<R: Respond>(
    this: &mut EvaluationDriver<R>,
    req: &Id<nixops4_core::eval_api::ResourceType>,
) -> Result<ResourceProviderInfo> {
    let resource = this.get_value(req.to_owned())?.clone();
    let resource_name = this.resource_names.get(req).unwrap();
    // let resource_api = this.eval_state.require_attrs_select(&resource, "_type")?;
    parse_resource(req, &mut this.eval_state, resource_name, resource)
}

fn parse_resource(
    req: &Id<nixops4_core::eval_api::ResourceType>,
    eval_state: &mut EvalState,
    resource_name: &String,
    resource: Value,
) -> Result<ResourceProviderInfo> {
    let provider_value = eval_state.require_attrs_select(&resource, "provider")?;
    let provider_json = {
        let span = tracing::info_span!(
            "evaluating and realising provider",
            resource_name = resource_name
        );
        let r = value_to_json(eval_state, &provider_value)?;
        drop(span);
        r
    };
    let resource_type_value = eval_state.require_attrs_select(&resource, "type")?;
    let resource_type_str = eval_state.require_string(&resource_type_value)?;
    let resource_state_value = eval_state.require_attrs_select(&resource, "state")?;
    let resource_state_value_type = eval_state.value_type(&resource_state_value)?;
    let resource_state_opt_str = match resource_state_value_type {
        ValueType::Null => None,
        ValueType::String => Some(eval_state.require_string(&resource_state_value)?),
        _ => bail!("expected state to be a string or null"),
    };
    Ok(ResourceProviderInfo {
        id: req.to_owned(),
        provider: provider_json,
        resource_type: resource_type_str,
        state: resource_state_opt_str,
    })
}

fn perform_get_resource_input<R: Respond>(
    this: &mut EvaluationDriver<R>,
    req: &nixops4_core::eval_api::Property,
) -> std::result::Result<ResourceInputState, anyhow::Error> {
    let attempt: Result<serde_json::Value, anyhow::Error> = (|| {
        let resource = this.get_value(req.resource.to_owned())?.clone();
        let inputs = this.eval_state.require_attrs_select(&resource, "inputs")?;
        let input = this.eval_state.require_attrs_select(&inputs, &req.name)?;
        let json = value_to_json(&mut this.eval_state, &input)?;
        Ok(json)
    })();
    match attempt {
        Ok(json) => Ok(ResourceInputState::ResourceInputValue((
            req.to_owned(),
            json,
        ))),
        Err(e) => {
            let s = e.to_string();
            if s.contains("__internal_exception_load_resource_property_#") {
                let base64_str = s
                    .split("__internal_exception_load_resource_property_#")
                    .collect::<Vec<&str>>()[1]
                    .split("#")
                    .collect::<Vec<&str>>()[0];
                let json_str = base64::engine::general_purpose::STANDARD.decode(base64_str)?;
                let named_property: NamedProperty = serde_json::from_slice(&json_str)?;
                Ok(ResourceInputState::ResourceInputDependency(
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
}

// TODO (roberth, nix): add API to add string context to a Worker, handling concurrent builds
//      and dynamic addition of more builds to the Worker
//      this worker should run on a separate thread in nixops4-eval
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
    use nix_bindings_expr::eval_state::{gc_register_my_thread, EvalState, EvalStateBuilder};
    use nix_bindings_fetchers::FetchersSettings;
    use nix_bindings_flake::EvalStateBuilderExt as _;
    use nix_bindings_flake::FlakeSettings;
    use nix_bindings_store::store::Store;
    use nixops4_core::eval_api::{
        AssignRequest, DeploymentRequest, FlakeRequest, Ids, QueryRequest,
    };
    use tempfile::TempDir;
    use tokio::runtime;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    struct TestRespond {
        responses: Arc<Mutex<Vec<EvalResponse>>>,
    }
    impl Respond for TestRespond {
        async fn call(&mut self, response: EvalResponse) -> Result<()> {
            let mut responses = self.responses.lock().unwrap();
            responses.push(response);
            Ok(())
        }
    }

    #[ctor]
    fn setup() {
        nix_bindings_util::settings::set("experimental-features", "flakes").unwrap();
        nix_bindings_expr::eval_state::test_init();
    }

    fn new_eval_state() -> Result<(EvalState, FetchersSettings, FlakeSettings)> {
        let fetch_settings = FetchersSettings::new()?;
        let flake_settings = FlakeSettings::new()?;
        let store = Store::open(None, [])?;
        let eval_state = EvalStateBuilder::new(store)?
            .flakes(&flake_settings)?
            .build()?;
        Ok((eval_state, fetch_settings, flake_settings))
    }

    #[test]
    fn test_eval_driver_invalid_flakeref() {
        (|| -> Result<()> {
            let guard = gc_register_my_thread().unwrap();
            let (eval_state, fetch_settings, flake_settings) = new_eval_state()?;
            let responses: Arc<Mutex<Vec<EvalResponse>>> = Default::default();
            let respond = TestRespond {
                responses: responses.clone(),
            };
            let mut driver =
                EvaluationDriver::new(eval_state, fetch_settings, flake_settings, respond);

            let flake_request = FlakeRequest {
                abspath: "/non-existent/path/to/flake".to_string(),
                input_overrides: Vec::new(),
            };
            let ids = Ids::new();
            let flake_id = ids.next();
            let assign_request = AssignRequest {
                assign_to: flake_id,
                payload: flake_request,
            };
            let request = EvalRequest::LoadFlake(assign_request);
            block_on(async { driver.perform_request(&request).await }).unwrap();
            {
                let r = responses.lock().unwrap();
                assert_eq!(r.len(), 1);
                match &r[0] {
                    EvalResponse::Error(id, msg) => {
                        assert_eq!(id, &flake_id.any());
                        if msg.contains("/non-existent/path/to/flake") {
                            drop(guard);
                            Ok(())
                        } else {
                            panic!("unexpected error message: {}", msg);
                        }
                    }
                    _ => panic!("expected EvalResponse::Error"),
                }
            }
        })()
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
        let tmpdir = TempDir::with_suffix("-test-nixops4-eval").unwrap();
        // write flake.nix
        let flake_path = tmpdir.path().join("flake.nix");
        std::fs::write(&flake_path, flake_nix).unwrap();

        (|| -> Result<()> {
            let guard = gc_register_my_thread().unwrap();
            let (eval_state, fetch_settings, flake_settings) = new_eval_state()?;
            let responses: Arc<Mutex<Vec<EvalResponse>>> = Default::default();
            let respond = TestRespond {
                responses: responses.clone(),
            };
            let mut driver =
                EvaluationDriver::new(eval_state, fetch_settings, flake_settings, respond);

            let flake_request = FlakeRequest {
                abspath: tmpdir.path().to_str().unwrap().to_string(),
                input_overrides: Vec::new(),
            };
            let ids = Ids::new();
            let flake_id = ids.next();
            let deployments_id = ids.next();
            let assign_request = AssignRequest {
                assign_to: flake_id,
                payload: flake_request,
            };
            block_on(async {
                driver
                    .perform_request(&EvalRequest::LoadFlake(assign_request))
                    .await
            })
            .unwrap();
            {
                let r = responses.lock().unwrap();
                if !r.is_empty() {
                    panic!("expected 0 responses, got: {:?}", r);
                }
            }
            block_on(async {
                driver
                    .perform_request(&EvalRequest::ListDeployments(QueryRequest::new(
                        deployments_id,
                        flake_id,
                    )))
                    .await
            })
            .unwrap();
            {
                let r = responses.lock().unwrap();
                if r.len() != 1 {
                    panic!("expected 1 response, got: {:?}", r);
                }
                match &r[0] {
                    EvalResponse::QueryResponse(
                        _id,
                        QueryResponseValue::ListDeployments((id, names)),
                    ) => {
                        // eprintln!("id: {:?}, names: {:?}", id, names);
                        assert_eq!(id, &flake_id);
                        assert_eq!(names.len(), 0);
                    }
                    _ => panic!("expected EvalResponse::ListResources"),
                }
            }

            drop(guard);
            Ok(())
        })()
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

        let tmpdir = TempDir::with_suffix("-test-nixops4-eval").unwrap();
        let flake_path = tmpdir.path().join("flake.nix");
        std::fs::write(&flake_path, flake_nix).unwrap();

        {
            let guard = gc_register_my_thread().unwrap();
            let (eval_state, fetch_settings, flake_settings) = new_eval_state().unwrap();
            let responses: Arc<Mutex<Vec<EvalResponse>>> = Default::default();
            let respond = TestRespond {
                responses: responses.clone(),
            };
            let mut driver =
                EvaluationDriver::new(eval_state, fetch_settings, flake_settings, respond);

            let flake_request = FlakeRequest {
                abspath: tmpdir.path().to_str().unwrap().to_string(),
                input_overrides: Vec::new(),
            };
            let ids = Ids::new();
            let flake_id = ids.next();
            let deployments_id = ids.next();
            let assign_request = AssignRequest {
                assign_to: flake_id,
                payload: flake_request,
            };
            block_on(driver.perform_request(&EvalRequest::LoadFlake(assign_request))).unwrap();
            block_on(
                driver.perform_request(&EvalRequest::ListDeployments(QueryRequest::new(
                    deployments_id,
                    flake_id,
                ))),
            )
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
            };
            drop(guard);
        }
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

        let tmpdir = TempDir::with_suffix("-test-nixops4-eval").unwrap();
        let flake_path = tmpdir.path().join("flake.nix");
        std::fs::write(&flake_path, flake_nix).unwrap();

        {
            let guard = gc_register_my_thread().unwrap();
            let (eval_state, fetch_settings, flake_settings) = new_eval_state().unwrap();
            let responses: Arc<Mutex<Vec<EvalResponse>>> = Default::default();
            let respond = TestRespond {
                responses: responses.clone(),
            };
            let mut driver =
                EvaluationDriver::new(eval_state, fetch_settings, flake_settings, respond);

            let flake_request = FlakeRequest {
                abspath: tmpdir.path().to_str().unwrap().to_string(),
                input_overrides: Vec::new(),
            };
            let ids = Ids::new();
            let flake_id = ids.next();
            let deployments_id = ids.next();
            let assign_request = AssignRequest {
                assign_to: flake_id,
                payload: flake_request,
            };
            block_on(driver.perform_request(&EvalRequest::LoadFlake(assign_request))).unwrap();
            {
                let r = responses.lock().unwrap();
                if !r.is_empty() {
                    panic!("expected 0 responses, got: {:?}", r);
                }
            }
            block_on(
                driver.perform_request(&EvalRequest::ListDeployments(QueryRequest::new(
                    deployments_id,
                    flake_id,
                ))),
            )
            .unwrap();
            {
                let r = responses.lock().unwrap();
                if r.len() != 1 {
                    panic!("expected 1 response, got: {:?}", r);
                }
                match &r[0] {
                    EvalResponse::QueryResponse(
                        _id,
                        QueryResponseValue::ListDeployments((id, names)),
                    ) => {
                        // eprintln!("id: {:?}, names: {:?}", id, names);
                        assert_eq!(id, &flake_id);
                        assert_eq!(names.len(), 2);
                        assert_eq!(names[0], "a");
                        assert_eq!(names[1], "b");
                    }
                    _ => panic!("expected EvalResponse::ListDeployments"),
                }
            }
            drop(guard);
        }
    }

    #[test]
    fn test_eval_driver_flake_example() {
        let flake_nix = r#"
            {
                outputs = { self, ... }: {
                    nixops4Deployments = {
                        example = {
                            _type = "nixops4Deployment";
                            deploymentFunction = { resources, resourceProviderSystem }:
                            assert resourceProviderSystem == builtins.currentSystem;
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

        let tmpdir = TempDir::with_suffix("-test-nixops4-eval").unwrap();
        let flake_path = tmpdir.path().join("flake.nix");
        std::fs::write(&flake_path, flake_nix).unwrap();

        {
            let guard = gc_register_my_thread().unwrap();
            let (eval_state, fetch_settings, flake_settings) = new_eval_state().unwrap();
            let responses: Arc<Mutex<Vec<EvalResponse>>> = Default::default();
            let respond = TestRespond {
                responses: responses.clone(),
            };
            let mut driver =
                EvaluationDriver::new(eval_state, fetch_settings, flake_settings, respond);

            let flake_request = FlakeRequest {
                abspath: tmpdir.path().to_str().unwrap().to_string(),
                input_overrides: Vec::new(),
            };
            let ids = Ids::new();
            let flake_id = ids.next();
            let deployment_id = ids.next();
            let assign_request = AssignRequest {
                assign_to: flake_id,
                payload: flake_request,
            };
            block_on(driver.perform_request(&EvalRequest::LoadFlake(assign_request))).unwrap();
            {
                let r = responses.lock().unwrap();
                if !r.is_empty() {
                    panic!("expected 0 responses, got: {:?}", r);
                }
            }
            block_on(
                driver.perform_request(&EvalRequest::LoadDeployment(AssignRequest {
                    assign_to: deployment_id,
                    payload: DeploymentRequest {
                        flake: flake_id,
                        name: "example".to_string(),
                    },
                })),
            )
            .unwrap();
            {
                let r = responses.lock().unwrap();
                if !r.is_empty() {
                    panic!("expected 0 responses, got: {:?}", r);
                }
            };
            drop(guard);
        }
    }

    #[test]
    fn test_parse_resource() {
        let ids = Ids::new();
        let guard = gc_register_my_thread().unwrap();
        let (mut eval_state, _fetch_settings, _flake_settings) = new_eval_state().unwrap();

        // Test parsing a resource with state
        let resource_with_state = eval_state
            .eval_from_string(
                r#"{
                provider = { example = "config"; };
                type = "example";
                state = "stateful";
            }"#,
                "<test-resource-with-state>",
            )
            .unwrap();

        let req_id = ids.next();
        let resource_name = "myResource".to_string();

        let result = parse_resource(
            &req_id,
            &mut eval_state,
            &resource_name,
            resource_with_state,
        )
        .unwrap();

        assert_eq!(result.id, req_id);
        assert_eq!(result.resource_type, "example");
        assert_eq!(result.state, Some("stateful".to_string()));
        assert_eq!(
            result.provider.get("example").unwrap().as_str().unwrap(),
            "config"
        );

        // Test parsing a resource without state (null)
        let resource_without_state = eval_state
            .eval_from_string(
                r#"{
                provider = { another = "value"; };
                type = "another-type";
                state = null;
            }"#,
                "<test-resource-without-state>",
            )
            .unwrap();

        let result2 = parse_resource(
            &req_id,
            &mut eval_state,
            &resource_name,
            resource_without_state,
        )
        .unwrap();

        assert_eq!(result2.id, req_id);
        assert_eq!(result2.resource_type, "another-type");
        assert_eq!(result2.state, None);
        assert_eq!(
            result2.provider.get("another").unwrap().as_str().unwrap(),
            "value"
        );

        drop(guard);
    }
}
