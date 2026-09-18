//! Test-only resource provider for NixOps4.
//!
//! Each resource type here exists to exercise one facet of the
//! NixOps4 resource protocol or NixOps4 coordinating behavior.

use anyhow::{bail, Context, Result};
use nixops4_resource::framework::{run_main, ResourceProvider};
use nixops4_resource::schema::v0;
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::path::Path;

struct FixtureResourceProvider;

/// `stateful_state_file`: a state resource that is itself stateful,
/// so its *own* `privateState` has to round-trip through the state
/// resource that it declares in turn.
///
/// - `name`: path of the JSON document holding the stored state.
/// - `new_private_state`: what the provider writes into its own
///   `privateState` on create/update.
/// - `expected_prior`: when set, this resource asserts on update that
///   the prior `privateState` handed back equals this value.
/// - `expect_own_private_state`: when set, `state_read` and
///   `state_event` assert that the private state NixOps4 hands back
///   with this resource equals this value. Catches NixOps4 opening
///   a stateful state resource as if it had never written private state.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct StatefulStateFileInProperties {
    name: String,
    #[serde(default)]
    new_private_state: Option<Value>,
    #[serde(default)]
    expected_prior: Option<Value>,
    #[serde(default)]
    expect_own_private_state: Option<Value>,
}

/// Deserialise a resource's input properties, the way
/// `nixops4-resources-local` does, so that each resource type here needs
/// only name the struct it wants.
fn parse_inputs<T: for<'de> Deserialize<'de>>(
    raw: &v0::InputProperties,
    resource_type: &str,
) -> Result<T> {
    serde_json::from_value(Value::Object(raw.0.clone().into_iter().collect()))
        .with_context(|| format!("parsing {} inputs", resource_type))
}

/// Refuse a create without state.
///
/// Every resource type here declares `requireState = true`, and the protocol
/// asks such a type to fail rather than quietly do without: see Private
/// State in doc/manual/src/resource-provider/interface.md.
fn require_stateful(is_stateful: bool, resource_type: &str) -> Result<()> {
    if !is_stateful {
        bail!(
            "fixture provider: {} requires state (isStateful must be true)",
            resource_type
        );
    }
    Ok(())
}

/// `undestroyable`: creates and updates without doing anything; always
/// fails to destroy. Lets a test watch a destroy fail and then see the
/// resource still in state.
///
/// - `reason`: text the provider puts in the error it fails with.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct UndestroyableInProperties {
    reason: String,
}

/// Read the stored state document, defaulting to the empty-state
/// skeleton when the file does not exist yet.
fn read_state_document(path: &str) -> Result<Value> {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents)
            .with_context(|| format!("parsing state document {}", path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(nixops4_resource::state::empty_state())
        }
        Err(e) => Err(e).with_context(|| format!("reading state document {}", path)),
    }
}

fn write_state_document(path: &str, state: &Value) -> Result<()> {
    let contents = serde_json::to_string_pretty(state).context("serialising state document")?;
    fs::write(path, contents).with_context(|| format!("writing state document {}", path))
}

/// Assert that the private state NixOps4 handed back with the resource is
/// what the deployment said to expect.
///
/// Nix serialises an undefined optional input as `null`, so `None` and
/// `null` both mean "no expectation"; only a concrete shape asserts.
fn assert_private_state(
    expected: Option<&Value>,
    resource: &v0::ExtantResource,
    op: &str,
) -> Result<()> {
    let expected = match expected {
        Some(v) if !v.is_null() => v,
        _ => return Ok(()),
    };
    let actual = resource.private_state.as_ref().map(|s| &s.0);
    if actual != Some(expected) {
        bail!(
            "{} ({}): expected private state {:?}, got {:?}",
            resource.type_.as_str(),
            op,
            expected,
            actual
        );
    }
    Ok(())
}

/// `private_state_echo`: round-trips arbitrary JSON through the
/// resource protocol's `privateState` slot.
///
/// - `new_private_state`: what the provider writes into
///   `privateState` on this operation. Absent means "clear it."
/// - `expected_prior`: on update and destroy, the provider asserts the
///   prior `privateState` handed back by NixOps4 equals this value.
///   Catches NixOps4 losing or corrupting the state.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct PrivateStateEchoInProperties {
    new_private_state: Option<Value>,
    #[serde(default)]
    expected_prior: Option<Value>,
}

impl ResourceProvider for FixtureResourceProvider {
    async fn create(
        &self,
        request: v0::CreateResourceRequest,
    ) -> Result<v0::CreateResourceResponse> {
        match request.type_.as_str() {
            "private_state_echo" => {
                require_stateful(request.is_stateful, request.type_.as_str())?;
                let p = parse_inputs::<PrivateStateEchoInProperties>(
                    &request.input_properties,
                    request.type_.as_str(),
                )?;
                Ok(v0::CreateResourceResponse {
                    output_properties: v0::OutputProperties(Default::default()),
                    private_state: p.new_private_state.map(v0::PrivateState),
                })
            }
            "stateful_state_file" => {
                require_stateful(request.is_stateful, request.type_.as_str())?;
                let p = parse_inputs::<StatefulStateFileInProperties>(
                    &request.input_properties,
                    request.type_.as_str(),
                )?;
                // Materialise the document, so that a later `state_read`
                // has something to read even before the first event.
                if !Path::new(p.name.as_str()).exists() {
                    write_state_document(p.name.as_str(), &nixops4_resource::state::empty_state())?;
                }
                Ok(v0::CreateResourceResponse {
                    output_properties: v0::OutputProperties(Default::default()),
                    private_state: p.new_private_state.map(v0::PrivateState),
                })
            }
            "undestroyable" => {
                require_stateful(request.is_stateful, request.type_.as_str())?;
                parse_inputs::<UndestroyableInProperties>(
                    &request.input_properties,
                    request.type_.as_str(),
                )?;
                Ok(v0::CreateResourceResponse {
                    output_properties: v0::OutputProperties(Default::default()),
                    private_state: None,
                })
            }
            t => bail!("fixture provider: unknown resource type {}", t),
        }
    }

    async fn update(
        &self,
        request: v0::UpdateResourceRequest,
    ) -> Result<v0::UpdateResourceResponse> {
        match request.resource.type_.as_str() {
            "private_state_echo" => {
                let p = parse_inputs::<PrivateStateEchoInProperties>(
                    &request.input_properties,
                    request.resource.type_.as_str(),
                )?;
                assert_private_state(p.expected_prior.as_ref(), &request.resource, "update")?;
                Ok(v0::UpdateResourceResponse {
                    output_properties: v0::OutputProperties(Default::default()),
                    private_state: p.new_private_state.map(v0::PrivateState),
                })
            }
            "stateful_state_file" => {
                let p = parse_inputs::<StatefulStateFileInProperties>(
                    &request.input_properties,
                    request.resource.type_.as_str(),
                )?;
                assert_private_state(p.expected_prior.as_ref(), &request.resource, "update")?;
                Ok(v0::UpdateResourceResponse {
                    output_properties: v0::OutputProperties(Default::default()),
                    private_state: p.new_private_state.map(v0::PrivateState),
                })
            }
            "undestroyable" => {
                parse_inputs::<UndestroyableInProperties>(
                    &request.input_properties,
                    request.resource.type_.as_str(),
                )?;
                Ok(v0::UpdateResourceResponse {
                    output_properties: v0::OutputProperties(Default::default()),
                    private_state: None,
                })
            }
            t => bail!("fixture provider: unknown resource type {}", t),
        }
    }

    async fn state_read(
        &self,
        request: v0::StateResourceReadRequest,
    ) -> Result<v0::StateResourceReadResponse> {
        match request.resource.type_.as_str() {
            "stateful_state_file" => {
                let p = parse_inputs::<StatefulStateFileInProperties>(
                    &request.resource.input_properties,
                    request.resource.type_.as_str(),
                )?;
                assert_private_state(
                    p.expect_own_private_state.as_ref(),
                    &request.resource,
                    "state_read",
                )?;
                let state = read_state_document(p.name.as_str())?;
                Ok(v0::StateResourceReadResponse {
                    state: serde_json::from_value(state)
                        .context("state document is not a JSON object")?,
                })
            }
            t => bail!("fixture provider: state_read on resource type {}", t),
        }
    }

    async fn state_event(
        &self,
        request: v0::StateResourceEvent,
    ) -> Result<v0::StateResourceEventResponse> {
        match request.resource.type_.as_str() {
            "stateful_state_file" => {
                let p = parse_inputs::<StatefulStateFileInProperties>(
                    &request.resource.input_properties,
                    request.resource.type_.as_str(),
                )?;
                assert_private_state(
                    p.expect_own_private_state.as_ref(),
                    &request.resource,
                    "state_event",
                )?;
                let mut state = read_state_document(p.name.as_str())?;
                json_patch::patch(&mut state, &request.patch.0).context("applying state patch")?;
                write_state_document(p.name.as_str(), &state)?;
                Ok(v0::StateResourceEventResponse {})
            }
            t => bail!("fixture provider: state_event on resource type {}", t),
        }
    }

    async fn destroy(
        &self,
        request: v0::DestroyResourceRequest,
    ) -> Result<v0::DestroyResourceResponse> {
        match request.resource.type_.as_str() {
            "private_state_echo" => {
                // Inputs come from the state record here, not from the
                // deployment, so `expected_prior` is still readable.
                let p = parse_inputs::<PrivateStateEchoInProperties>(
                    &request.resource.input_properties,
                    request.resource.type_.as_str(),
                )?;
                assert_private_state(p.expected_prior.as_ref(), &request.resource, "destroy")?;
                Ok(v0::DestroyResourceResponse {})
            }
            "stateful_state_file" => {
                // A real state resource should check that it's not tracking any live
                // resources anymore, but we don't need that behavior here.
                // That's tested with/for local's state_file just fine.
                let p = parse_inputs::<StatefulStateFileInProperties>(
                    &request.resource.input_properties,
                    request.resource.type_.as_str(),
                )?;
                assert_private_state(
                    p.expect_own_private_state.as_ref(),
                    &request.resource,
                    "destroy",
                )?;
                // Removing the document is what makes a destroy observable.
                match fs::remove_file(p.name.as_str()) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        return Err(e)
                            .with_context(|| format!("removing state document {}", p.name))
                    }
                }
                Ok(v0::DestroyResourceResponse {})
            }
            "undestroyable" => {
                // The point of the type: a destroy that fails, so a test
                // can watch the command fail and then see the resource
                // still in state.
                let p = parse_inputs::<UndestroyableInProperties>(
                    &request.resource.input_properties,
                    request.resource.type_.as_str(),
                )?;
                bail!("fixture provider: refusing to destroy: {}", p.reason)
            }
            t => bail!("fixture provider: unknown resource type {}", t),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    run_main(FixtureResourceProvider).await;
}
