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
/// so NixOps has to track it in the state resource that it declares
/// in turn.
///
/// - `name`: path of the JSON document holding the stored state.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct StatefulStateFileInProperties {
    name: String,
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

impl ResourceProvider for FixtureResourceProvider {
    async fn create(
        &self,
        request: v0::CreateResourceRequest,
    ) -> Result<v0::CreateResourceResponse> {
        match request.type_.as_str() {
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
            "stateful_state_file" => {
                parse_inputs::<StatefulStateFileInProperties>(
                    &request.input_properties,
                    request.resource.type_.as_str(),
                )?;
                Ok(v0::UpdateResourceResponse {
                    output_properties: v0::OutputProperties(Default::default()),
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
                let mut state = read_state_document(p.name.as_str())?;
                json_patch::patch(&mut state, &request.patch.0).context("applying state patch")?;
                write_state_document(p.name.as_str(), &state)?;
                Ok(v0::StateResourceEventResponse {})
            }
            t => bail!("fixture provider: state_event on resource type {}", t),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    run_main(FixtureResourceProvider).await;
}
