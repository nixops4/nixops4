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
    AnyType, AssignRequest, ComponentHandle, ComponentRequest, CompositeType, EvalRequest,
    EvalResponse, FlakeType, Id, IdNum, NamedProperty, QueryRequest, QueryResponseValue,
    RequestIdType, ResourceProviderInfo, ResourceType, StepResult,
};
use std::cell::RefCell;
use std::rc::Rc;

/// Convert a Result to StepResult, catching dependency exceptions.
///
/// When evaluating Nix expressions, a dependency on a resource output that
/// doesn't exist yet is signaled via a special exception. This function
/// catches that exception and converts it to `StepResult::Needs`.
fn catch_dependency<T>(result: Result<T>) -> Result<StepResult<T>> {
    match result {
        Ok(val) => Ok(StepResult::Done(val)),
        Err(e) => {
            if let Some(dep) = parse_dependency_error(&e) {
                Ok(StepResult::Needs(dep))
            } else {
                Err(e)
            }
        }
    }
}

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
    /// Maps resource IDs to resource names for GetResource lookups
    resource_names: HashMap<IdNum, String>,
    /// Stores ComponentRequests by ID so GetComponentKind can load/retry.
    /// Populated by AssignMember.
    member_requests: HashMap<IdNum, ComponentRequest>,
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
            member_requests: HashMap::new(),
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

    /// Helper for assignment requests. Caches value on success, sends error on failure.
    async fn handle_assign_request<T: RequestIdType>(
        &mut self,
        request: &AssignRequest<T>,
        handler: impl FnOnce(&mut Self, &T) -> Result<Value>,
    ) -> Result<()> {
        // Check for duplicate ID assignment
        if self.values.contains_key(&request.assign_to.num()) {
            let error_msg = format!("id already used: {}", request.assign_to.num());
            self.respond(EvalResponse::Error(request.assign_to.any(), error_msg))
                .await?;
            return Ok(());
        }

        match handler(self, &request.payload) {
            Ok(value) => {
                self.values.insert(request.assign_to.num(), Ok(value));
                Ok(())
            }
            Err(e) => {
                // Dependency errors are not cached or reported (caller will retry)
                if parse_dependency_error(&e).is_some() {
                    return Ok(());
                }
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

    fn get_flake_root_value(&mut self, flake: Id<FlakeType>) -> Result<Value> {
        let flake = self.get_value(flake)?.clone();
        let outputs = self.eval_state.require_attrs_select(&flake, "outputs")?;
        let root = self.eval_state.require_attrs_select(&outputs, "nixops4")?;
        Ok(root.clone())
    }

    /// Load a member attrset from a parent composite.
    fn load_member(&mut self, req: &ComponentRequest) -> Result<Value> {
        let parent = self.get_value(req.parent)?.clone();
        let members_attrset = self.eval_state.require_attrs_select(&parent, "members")?;
        self.eval_state
            .require_attrs_select(&members_attrset, &req.name)
    }

    /// Get the component kind for an assigned member ID.
    fn get_component_kind(&mut self, id: Id<AnyType>) -> Result<StepResult<ComponentHandle>> {
        let id_num = id.num();

        let req = self
            .member_requests
            .get(&id_num)
            .ok_or_else(|| anyhow::anyhow!("no AssignMember request found for id {}", id_num))?
            .clone();

        catch_dependency((|| {
            let member = self.load_member(&req)?;
            let resource_opt = self
                .eval_state
                .require_attrs_select_opt(&member, "resource")?;

            let handle = match resource_opt {
                Some(resource_data) => {
                    self.values.insert(id_num, Ok(resource_data));
                    self.resource_names.insert(id_num, req.name.clone());
                    ComponentHandle::Resource(Id::<ResourceType>::from_num(id_num))
                }
                None => {
                    self.values.insert(id_num, Ok(member));
                    ComponentHandle::Composite(Id::<CompositeType>::from_num(id_num))
                }
            };
            Ok(handle)
        })())
    }

    pub async fn perform_request(&mut self, request: &EvalRequest) -> Result<()> {
        match request {
            EvalRequest::LoadFlake(req) => {
                self.handle_assign_request(req, |this, req| {
                    this.get_flake(req.abspath.as_str(), &req.input_overrides)
                })
                .await
            }
            EvalRequest::LoadRoot(req) => {
                let known_outputs = Rc::clone(&self.known_outputs);
                self.handle_assign_request(req, |this, req| {
                    perform_load_root(this, req, known_outputs)
                })
                .await
            }
            // List member names in a composite (kind determined later via GetComponentKind).
            // See StepResult::Needs for retry semantics.
            EvalRequest::ListMembers(req) => {
                self.handle_simple_request(req, QueryResponseValue::ListMembers, |this, req| {
                    let composite = this.get_value(req.to_owned())?.clone();
                    catch_dependency((|| {
                        let members_attrset = this
                            .eval_state
                            .require_attrs_select(&composite, "members")?;
                        this.eval_state.require_attrs_names(&members_attrset)
                    })())
                })
                .await
            }
            EvalRequest::AssignMember(request) => {
                self.member_requests
                    .insert(request.assign_to.num(), request.payload.clone());
                self.handle_assign_request(request, |this, req| this.load_member(req))
                    .await
            }
            // Query the component kind for a previously assigned member ID.
            // Returns ComponentHandle or dependency, retrying load if needed.
            EvalRequest::GetComponentKind(req) => {
                self.handle_simple_request(req, QueryResponseValue::ComponentKind, |this, id| {
                    this.get_component_kind(*id)
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
                        catch_dependency((|| {
                            let resource = this.get_value(req.to_owned())?.clone();
                            let inputs =
                                this.eval_state.require_attrs_select(&resource, "inputs")?;
                            this.eval_state.require_attrs_names(&inputs)
                        })())
                    },
                )
                .await
            }
            EvalRequest::GetResourceInput(req) => {
                self.handle_simple_request(
                    req,
                    QueryResponseValue::ResourceInputValue,
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
            }
        }
    }
}

fn perform_load_root<R: Respond>(
    driver: &mut EvaluationDriver<R>,
    req: &nixops4_core::eval_api::RootRequest,
    known_outputs: Rc<RefCell<HashMap<NamedProperty, Value>>>,
) -> Result<Value, anyhow::Error> {
    let root = driver.get_flake_root_value(req.flake)?;
    let es = &mut driver.eval_state;
    {
        let tag = es.require_attrs_select(&root, "_type")?;
        let str = es.require_string(&tag)?;
        if str != "nixops4Component" {
            bail!("expected _type to be 'nixops4Component', got: {}", str);
        }
    }
    // Unified component model fixpoint evaluation.
    // Walks the members tree, providing output values for resources and recursing into composites.
    let eval_expr = r#"
                        # primops
                        loadMemberOutput:
                        # user expr
                        rootFunction:
                        # other args, such as resourceProviderSystem
                        extraArgs:
                        let
                          inherit (builtins) mapAttrs;

                          # Build member output values for a composite
                          # path: component path (list of names)
                          # export: the _export value of the composite component
                          # Returns an attrset mapping member names to their outputs
                          makeMemberOutputs = path: export:
                            mapAttrs
                              (name: memberExport:
                                if memberExport ? resource
                                then
                                  # Resource component: provide output values from fixpoint
                                  mapAttrs
                                    (loadMemberOutput path name)
                                    (memberExport.resource.outputsSkeleton
                                      or (throw "Resource ${name} does not declare its outputs via outputsSkeleton. This is an implementation error in the resource provider."))
                                else
                                  # Composite component: recurse into members
                                  # Returns nested output values directly (no extraArgs wrapper)
                                  makeMemberOutputs (path ++ [name]) memberExport
                              )
                              (export.members or {});

                          # Build arguments for the root function at root level
                          # This includes extraArgs plus the output values
                          makeArguments = export:
                            extraArgs // {
                              outputValues = makeMemberOutputs [] export;
                            };
                          fixpoint = rootFunction (makeArguments fixpoint);
                        in
                          fixpoint
                    "#;
    let root_function = es.require_attrs_select(&root, "rootFunction")?;
    let prim_load_member_output = PrimOp::new(
        es,
        PrimOpMeta {
            name: cstr!("nixopsLoadMemberOutput"),
            doc: cstr!("Internal function that loads a member component output attribute."),
            args: [
                cstr!("componentPath"),
                cstr!("memberName"),
                cstr!("attrName"),
                cstr!("ignored"),
            ],
        },
        Box::new(move |es, [component_path, member_name, attr_name, _]| {
            // Build the full component path by appending the member name
            let component_path = {
                let value_list: Vec<_> = es.require_list_strict(component_path)?;
                let mut path = Vec::new();
                for value in value_list {
                    let name = es.require_string(&value)?;
                    path.push(name);
                }
                path
            };
            let member_name = es.require_string(member_name)?;
            let attr_name = es.require_string(attr_name)?;

            // Build full path to the resource (component_path + member_name)
            let mut resource_path = component_path.clone();
            resource_path.push(member_name.to_string());
            let property = NamedProperty {
                resource: nixops4_core::eval_api::ComponentPath(resource_path),
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
                        base64::engine::general_purpose::STANDARD
                            .encode(serde_json::to_string(&property).unwrap()),
                    ))
                }
            }
        }),
    )?;
    let load_member_output = es.new_value_primop(prim_load_member_output)?;
    // let extra_args = es.new_value_attrs(HashMap::new())?;
    let resource_provider_system = nix_bindings_util::settings::get("system")?;
    let resource_provider_system_value = es.new_value_str(resource_provider_system.as_str())?;
    let extra_args = es.new_value_attrs([(
        "resourceProviderSystem".to_string(),
        resource_provider_system_value,
    )])?;

    let fixpoint = {
        let v = es.eval_from_string(eval_expr, "<nixops4 internals>")?;
        es.call_multi(&v, &[load_member_output, root_function, extra_args])
    }?;
    Ok(fixpoint)
}

fn perform_get_resource<R: Respond>(
    this: &mut EvaluationDriver<R>,
    req: &Id<nixops4_core::eval_api::ResourceType>,
) -> Result<StepResult<ResourceProviderInfo>> {
    let resource = this.get_value(req.to_owned())?.clone();
    let resource_name = this.resource_names.get(&req.num()).unwrap();
    catch_dependency(parse_resource(
        req,
        &mut this.eval_state,
        resource_name,
        resource,
    ))
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
    let resource_state_opt = match resource_state_value_type {
        ValueType::Null => None,
        ValueType::List => {
            let state_list: Vec<_> = eval_state.require_list_strict(&resource_state_value)?;
            if state_list.is_empty() {
                bail!("expected state list to be non-empty");
            }
            // State is a ComponentPath - a list of component names from root to the state resource
            let mut component_path = Vec::new();
            for value in &state_list {
                component_path.push(eval_state.require_string(value)?);
            }
            Some(nixops4_core::eval_api::ComponentPath(component_path))
        }
        _ => bail!("expected state to be a list of strings or null"),
    };
    Ok(ResourceProviderInfo {
        id: req.to_owned(),
        provider: provider_json,
        resource_type: resource_type_str,
        state: resource_state_opt,
    })
}

fn perform_get_resource_input<R: Respond>(
    this: &mut EvaluationDriver<R>,
    req: &nixops4_core::eval_api::Property,
) -> Result<StepResult<serde_json::Value>> {
    catch_dependency((|| {
        let resource = this.get_value(req.resource.to_owned())?.clone();
        let inputs = this.eval_state.require_attrs_select(&resource, "inputs")?;
        let input = this.eval_state.require_attrs_select(&inputs, &req.name)?;
        value_to_json(&mut this.eval_state, &input)
    })())
}

/// Parse a structural dependency error from the evaluator.
///
/// When evaluating an expression requires a resource output that doesn't exist yet,
/// the evaluator throws an error containing the dependency information encoded as
/// base64 JSON. This function extracts that dependency.
fn parse_dependency_error(e: &anyhow::Error) -> Option<NamedProperty> {
    let s = e.to_string();
    if !s.contains("__internal_exception_load_resource_property_#") {
        return None;
    }
    let base64_str = s
        .split("__internal_exception_load_resource_property_#")
        .collect::<Vec<&str>>()
        .get(1)?
        .split('#')
        .next()?;
    let json_str = base64::engine::general_purpose::STANDARD
        .decode(base64_str)
        .ok()?;
    serde_json::from_slice(&json_str).ok()
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
        AnyType, AssignRequest, ComponentPath, ComponentRequest, CompositeType, FlakeRequest, Id,
        Ids, QueryRequest, QueryResponseValue, ResourceType, RootRequest, StepResult,
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
    fn test_eval_driver_flake_no_nixops4() {
        let flake_nix = r#"
            {
                outputs = { ... }: {
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
            let root_id = ids.next();
            let assign_request = AssignRequest {
                assign_to: flake_id,
                payload: flake_request,
            };
            block_on(driver.perform_request(&EvalRequest::LoadFlake(assign_request))).unwrap();
            block_on(
                driver.perform_request(&EvalRequest::LoadRoot(AssignRequest {
                    assign_to: root_id,
                    payload: RootRequest { flake: flake_id },
                })),
            )
            .unwrap();
            {
                let r = responses.lock().unwrap();
                assert_eq!(r.len(), 1);
                match &r[0] {
                    EvalResponse::Error(id, msg) => {
                        assert_eq!(id, &root_id.any());
                        assert!(
                            msg.contains("nixops4"),
                            "expected error about missing nixops4 attribute, got: {}",
                            msg
                        );
                    }
                    _ => panic!("expected EvalResponse::Error, got: {:?}", r[0]),
                }
            };
            drop(guard);
        }
    }

    #[test]
    fn test_eval_driver_flake_empty_root() {
        let flake_nix = r#"
            {
                outputs = { ... }: {
                    nixops4 = {
                        _type = "nixops4Component";
                        rootFunction = { outputValues, resourceProviderSystem, ... }: {
                            members = {};
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
            let root_id = ids.next();
            let list_members_id = ids.next();
            let assign_request = AssignRequest {
                assign_to: flake_id,
                payload: flake_request,
            };
            block_on(driver.perform_request(&EvalRequest::LoadFlake(assign_request))).unwrap();
            block_on(
                driver.perform_request(&EvalRequest::LoadRoot(AssignRequest {
                    assign_to: root_id,
                    payload: RootRequest { flake: flake_id },
                })),
            )
            .unwrap();
            {
                let r = responses.lock().unwrap();
                assert!(
                    r.is_empty(),
                    "expected no errors from LoadRoot, got: {:?}",
                    r
                );
            }
            // List members should return empty
            block_on(
                driver.perform_request(&EvalRequest::ListMembers(QueryRequest::new(
                    list_members_id,
                    root_id,
                ))),
            )
            .unwrap();
            {
                let r = responses.lock().unwrap();
                assert_eq!(r.len(), 1);
                match &r[0] {
                    EvalResponse::QueryResponse(
                        _,
                        QueryResponseValue::ListMembers(step_result),
                    ) => match step_result {
                        StepResult::Done(members) => {
                            assert!(members.is_empty(), "expected empty members");
                        }
                        _ => panic!("expected Done, got: {:?}", step_result),
                    },
                    _ => panic!("expected ListMembers response, got: {:?}", r[0]),
                }
            }
            drop(guard);
        }
    }

    #[test]
    fn test_eval_driver_flake_root_throw() {
        let flake_nix = r#"
            {
                outputs = { ... }: {
                    nixops4 = throw "so this is the error message from the nixops4 attribute value";
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
            let root_id = ids.next();
            let assign_request = AssignRequest {
                assign_to: flake_id,
                payload: flake_request,
            };
            block_on(driver.perform_request(&EvalRequest::LoadFlake(assign_request))).unwrap();
            block_on(
                driver.perform_request(&EvalRequest::LoadRoot(AssignRequest {
                    assign_to: root_id,
                    payload: RootRequest { flake: flake_id },
                })),
            )
            .unwrap();
            {
                let r = responses.lock().unwrap();
                if r.len() != 1 {
                    panic!("expected 1 response, got: {:?}", r);
                }
                match &r[0] {
                    EvalResponse::Error(id, msg) => {
                        assert_eq!(id, &root_id.any());
                        if !msg.contains(
                            "so this is the error message from the nixops4 attribute value",
                        ) {
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
    fn test_eval_driver_list_members_lazy() {
        // Verify that ListMembers can enumerate member names without evaluating member contents
        let flake_nix = r#"
            {
                outputs = { ... }: {
                    nixops4 = {
                        _type = "nixops4Component";
                        rootFunction = { outputValues, resourceProviderSystem, ... }: {
                            members = {
                                a = {
                                    resource = {
                                        type = "dummy";
                                        provider = { type = "stdio"; executable = "__test:dummy"; };
                                        inputs = {};
                                        outputsSkeleton = {};
                                        state = null;
                                    };
                                };
                                b = throw "do not evaluate b when listing members";
                                c = throw "do not evaluate c when listing members";
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
            let root_id = ids.next();
            let list_members_id = ids.next();
            let assign_request = AssignRequest {
                assign_to: flake_id,
                payload: flake_request,
            };
            block_on(driver.perform_request(&EvalRequest::LoadFlake(assign_request))).unwrap();
            block_on(
                driver.perform_request(&EvalRequest::LoadRoot(AssignRequest {
                    assign_to: root_id,
                    payload: RootRequest { flake: flake_id },
                })),
            )
            .unwrap();
            {
                let r = responses.lock().unwrap();
                assert!(
                    r.is_empty(),
                    "expected no errors from LoadRoot, got: {:?}",
                    r
                );
            }
            // ListMembers should return all three names without triggering the throws
            block_on(
                driver.perform_request(&EvalRequest::ListMembers(QueryRequest::new(
                    list_members_id,
                    root_id,
                ))),
            )
            .unwrap();
            {
                let r = responses.lock().unwrap();
                assert_eq!(r.len(), 1);
                match &r[0] {
                    EvalResponse::QueryResponse(
                        _,
                        QueryResponseValue::ListMembers(step_result),
                    ) => match step_result {
                        StepResult::Done(members) => {
                            assert_eq!(members.len(), 3, "expected 3 members");
                            assert!(members.contains(&"a".to_string()));
                            assert!(members.contains(&"b".to_string()));
                            assert!(members.contains(&"c".to_string()));
                        }
                        _ => panic!("expected Done, got: {:?}", step_result),
                    },
                    _ => panic!("expected ListMembers response, got: {:?}", r[0]),
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
                    nixops4 = {
                        _type = "nixops4Component";
                        rootFunction = { outputValues, resourceProviderSystem, ... }:
                        assert resourceProviderSystem == builtins.currentSystem;
                        {
                            members = {
                                a = {
                                    resource = {
                                        type = "dummy";
                                        provider = { type = "stdio"; executable = "__test:dummy"; };
                                        inputs = {
                                            foo = "bar";
                                        };
                                        outputsSkeleton = { foo2 = {}; };
                                        state = null;
                                    };
                                };
                                b = {
                                    resource = {
                                        type = "dummy";
                                        provider = { type = "stdio"; executable = "__test:dummy"; };
                                        inputs = {
                                            qux = outputValues.a.foo2;
                                        };
                                        outputsSkeleton = {};
                                        state = null;
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
            let root_id = ids.next();
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
                driver.perform_request(&EvalRequest::LoadRoot(AssignRequest {
                    assign_to: root_id,
                    payload: RootRequest { flake: flake_id },
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
                state = ["stateful"];
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
        assert_eq!(
            result.state,
            Some(nixops4_core::eval_api::ComponentPath(vec![
                "stateful".to_string()
            ]))
        );
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

    /// Test that GetResource returns a structural dependency when the resource type
    /// depends on another resource's output.
    #[test]
    fn test_get_resource_structural_dependency() {
        let flake_nix = r#"
            {
                outputs = { self, ... }: {
                    nixops4 = {
                        _type = "nixops4Component";
                        rootFunction = { outputValues, resourceProviderSystem, ... }: {
                            members = {
                                a = {
                                    resource = {
                                        type = "dummy";
                                        provider = { type = "stdio"; executable = "__test:dummy"; };
                                        inputs = {};
                                        outputsSkeleton = { resourceType = {}; };
                                        state = null;
                                    };
                                };
                                b = {
                                    resource = {
                                        # type depends on resource a's output
                                        type = outputValues.a.resourceType;
                                        provider = { type = "stdio"; executable = "__test:dummy"; };
                                        inputs = {};
                                        outputsSkeleton = {};
                                        state = null;
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
            let root_id: Id<CompositeType> = ids.next();
            let b_id: Id<AnyType> = ids.next();
            let assign_request = AssignRequest {
                assign_to: flake_id,
                payload: flake_request,
            };
            block_on(driver.perform_request(&EvalRequest::LoadFlake(assign_request))).unwrap();
            block_on(
                driver.perform_request(&EvalRequest::LoadRoot(AssignRequest {
                    assign_to: root_id,
                    payload: RootRequest { flake: flake_id },
                })),
            )
            .unwrap();

            // Load component b using AssignMember + GetComponentKind
            block_on(
                driver.perform_request(&EvalRequest::AssignMember(AssignRequest {
                    assign_to: b_id,
                    payload: ComponentRequest {
                        parent: root_id,
                        name: "b".to_string(),
                    },
                })),
            )
            .unwrap();
            block_on(
                driver.perform_request(&EvalRequest::GetComponentKind(QueryRequest::new(
                    ids.next(),
                    b_id,
                ))),
            )
            .unwrap();

            // Clear responses and try GetResource on b
            responses.lock().unwrap().clear();

            let b_resource_id = Id::<ResourceType>::from_num(b_id.num());
            block_on(
                driver.perform_request(&EvalRequest::GetResource(QueryRequest::new(
                    ids.next(),
                    b_resource_id,
                ))),
            )
            .unwrap();

            // Should get a structural dependency response
            let r = responses.lock().unwrap();
            assert_eq!(r.len(), 1, "expected 1 response, got: {:?}", r);
            match &r[0] {
                EvalResponse::QueryResponse(
                    _,
                    QueryResponseValue::ResourceProviderInfo(StepResult::Needs(dep)),
                ) => {
                    assert_eq!(dep.resource, ComponentPath(vec!["a".to_string()]));
                    assert_eq!(dep.name, "resourceType");
                }
                other => panic!("expected Needs, got: {:?}", other),
            }

            drop(guard);
        }
    }

    /// Test that ListResourceInputs returns a structural dependency when the inputs
    /// attrset structure depends on another resource's output.
    #[test]
    fn test_list_resource_inputs_structural_dependency() {
        let flake_nix = r#"
            {
                outputs = { self, ... }: {
                    nixops4 = {
                        _type = "nixops4Component";
                        rootFunction = { outputValues, resourceProviderSystem, ... }: {
                            members = {
                                a = {
                                    resource = {
                                        type = "dummy";
                                        provider = { type = "stdio"; executable = "__test:dummy"; };
                                        inputs = {};
                                        # extraInputs is an attrset that b will merge into its inputs
                                        outputsSkeleton = { extraInputs = {}; };
                                        state = null;
                                    };
                                };
                                b = {
                                    resource = {
                                        type = "dummy";
                                        provider = { type = "stdio"; executable = "__test:dummy"; };
                                        # inputs attrset structure depends on resource a's output
                                        # The // operator forces evaluation to determine attr names
                                        inputs = { static = "value"; } // outputValues.a.extraInputs;
                                        outputsSkeleton = {};
                                        state = null;
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
            let root_id: Id<CompositeType> = ids.next();
            let b_id: Id<AnyType> = ids.next();
            let assign_request = AssignRequest {
                assign_to: flake_id,
                payload: flake_request,
            };
            block_on(driver.perform_request(&EvalRequest::LoadFlake(assign_request))).unwrap();
            block_on(
                driver.perform_request(&EvalRequest::LoadRoot(AssignRequest {
                    assign_to: root_id,
                    payload: RootRequest { flake: flake_id },
                })),
            )
            .unwrap();

            // Load component b using AssignMember + GetComponentKind
            block_on(
                driver.perform_request(&EvalRequest::AssignMember(AssignRequest {
                    assign_to: b_id,
                    payload: ComponentRequest {
                        parent: root_id,
                        name: "b".to_string(),
                    },
                })),
            )
            .unwrap();
            block_on(
                driver.perform_request(&EvalRequest::GetComponentKind(QueryRequest::new(
                    ids.next(),
                    b_id,
                ))),
            )
            .unwrap();

            // Clear responses and try ListResourceInputs on b
            responses.lock().unwrap().clear();

            let b_resource_id = Id::<ResourceType>::from_num(b_id.num());
            block_on(
                driver.perform_request(&EvalRequest::ListResourceInputs(QueryRequest::new(
                    ids.next(),
                    b_resource_id,
                ))),
            )
            .unwrap();

            // Should get a dependency response
            let r = responses.lock().unwrap();
            assert_eq!(r.len(), 1, "expected 1 response, got: {:?}", r);
            match &r[0] {
                EvalResponse::QueryResponse(
                    _,
                    QueryResponseValue::ListResourceInputs(StepResult::Needs(dep)),
                ) => {
                    assert_eq!(dep.resource, ComponentPath(vec!["a".to_string()]));
                    assert_eq!(dep.name, "extraInputs");
                }
                other => panic!("expected Needs, got: {:?}", other),
            }

            drop(guard);
        }
    }
}
