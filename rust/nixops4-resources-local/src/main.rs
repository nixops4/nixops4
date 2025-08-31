use std::fs::OpenOptions;
use std::io::{self, Write};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use nixops4_resource::framework::run_main;
use nixops4_resource::schema::v0;
use serde::Deserialize;
use serde_json::Value;

mod state;
use state::{StateEvent, StateEventMeta, StateEventStream, StateHandle};

struct LocalResourceProvider {}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct FileInProperties {
    name: String,
    contents: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct FileOutProperties {}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct ExecInProperties {
    executable: String,
    args: Vec<String>,
    stdin: Option<String>,
    // TODO parseJSON: bool  (for convenience and presentation purposes)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct ExecOutProperties {
    stdout: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct MemoInProperties {
    initialize_with: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct MemoOutProperties {
    value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct StateFileInProperties {
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct StateFileOutProperties {}

impl nixops4_resource::framework::ResourceProvider for LocalResourceProvider {
    async fn create(
        &self,
        request: v0::CreateResourceRequest,
    ) -> Result<v0::CreateResourceResponse> {
        match request.type_.as_str() {
            "file" => do_create(request, |p: FileInProperties| {
                std::fs::write(&p.name, &p.contents)?;
                Ok(FileOutProperties {})
            }),
            "exec" => do_create(request, |p: ExecInProperties| {
                let mut command = std::process::Command::new(&p.executable);
                command.args(&p.args);

                let in_stdio = if p.stdin.is_some() {
                    std::process::Stdio::piped()
                } else {
                    std::process::Stdio::null()
                };

                let mut child = command
                    .stdin(in_stdio)
                    .stdout(std::process::Stdio::piped())
                    .spawn()
                    .with_context(|| {
                        format!(
                            "Could not spawn resource provider process: {}",
                            p.executable
                        )
                    })?;

                match p.stdin {
                    Some(stdinstr) => {
                        child
                            .stdin
                            .as_mut()
                            .unwrap()
                            .write_all(stdinstr.as_bytes())?;
                    }
                    None => {}
                }

                // Read stdout
                let output = child.wait_with_output()?;
                let stdout = String::from_utf8(output.stdout)?;

                if output.status.success() {
                    Ok(ExecOutProperties { stdout })
                } else {
                    bail!(
                        "Local resource process failed with exit code: {}",
                        output.status
                    )
                }
            }),
            "memo" => {
                if !request.is_stateful {
                    bail!("memo resources require state (isStateful must be true)");
                }
                do_create(request, |p: MemoInProperties| {
                    // A stateful resource that is initialized upon creation and
                    // not modified afterwards, except perhaps through a manual
                    // operation or a migration of sorts
                    Ok(MemoOutProperties {
                        value: p.initialize_with,
                    })
                })
            }
            "state_file" => do_create(request, |p: StateFileInProperties| {
                // Validate the first entry
                let r = OpenOptions::new()
                    .read(true)
                    .write(false)
                    .create(false)
                    .open(&p.name);

                match r {
                    Ok(file) => {
                        // Check the first event
                        StateEventStream::open_from_reader(io::BufReader::new(file))?;
                    }
                    Err(e) => {
                        // If the file doesn't exist, we can create it
                        if e.kind() == io::ErrorKind::NotFound {
                            StateHandle::open(&p.name, true)?;
                        } else {
                            bail!("Could not open state file: {}", e);
                        }
                    }
                }
                Ok(StateFileOutProperties {})
            }),
            t => bail!(
                "LocalResourceProvider::create: unknown resource type: {}",
                t
            ),
        }
    }

    async fn update(
        &self,
        request: v0::UpdateResourceRequest,
    ) -> Result<v0::UpdateResourceResponse> {
        match request.resource.type_.as_str() {
            "file" => {
                bail!("Internal error: update called on stateless file resource");
            }
            "exec" => {
                bail!("Internal error: update called on stateless exec resource");
            }
            "state_file" => {
                bail!("Internal error: update called on stateless state_file resource");
            }
            "memo" => do_update(&request, |_p: MemoInProperties| {
                let previous_output_properties = request.resource.output_properties.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("The update operation on a memo resource requires that the output properties are set")
                    })?;
                let previous_output_properties: MemoOutProperties =
                    serde_json::from_value(Value::Object(
                        previous_output_properties
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                    ))
                    .with_context(|| {
                        format!(
                            "Could not deserialize output properties for {} resource",
                            request.resource.type_
                        )
                    })?;
                Ok(MemoOutProperties {
                    value: previous_output_properties.value,
                })
            }),
            t => bail!(
                "LocalResourceProvider::update: unknown resource type: {}",
                t
            ),
        }
    }

    async fn state_read(
        &self,
        request: v0::StateResourceReadRequest,
    ) -> Result<v0::StateResourceReadResponse> {
        match request.resource.type_.as_str() {
            "state_file" => {
                let inputs = parse_input_properties::<StateFileInProperties>(
                    &request.resource.input_properties,
                    &request.resource.type_,
                )?;

                let file_contents = std::fs::read_to_string(inputs.name.as_str())?;
                let stream = StateEventStream::open_from_reader(io::BufReader::new(
                    file_contents.as_bytes(),
                ))?;
                let mut state = serde_json::json!({});
                state::apply_state_events(&mut state, stream).unwrap();
                Ok(v0::StateResourceReadResponse {
                    state: serde_json::from_value(state)?,
                })
            }
            t => bail!(
                "LocalResourceProvider::state_read: unknown resource type: {}",
                t
            ),
        }
    }

    async fn state_event(
        &self,
        request: v0::StateResourceEvent,
    ) -> Result<v0::StateResourceEventResponse> {
        match request.resource.type_.as_str() {
            "state_file" => {
                let inputs = parse_input_properties::<StateFileInProperties>(
                    &request.resource.input_properties,
                    &request.resource.type_,
                )?;
                let mut handle = StateHandle::open(inputs.name.as_str(), false)?;

                handle.append(&[&StateEvent {
                    index: 0,
                    meta: StateEventMeta {
                        time: Utc::now().to_rfc3339(),
                        other_fields: serde_json::json!({
                            "event": request.event,
                        }),
                    },
                    patch: request.patch,
                }])?;
                Ok(v0::StateResourceEventResponse {})
            }
            t => bail!(
                "LocalResourceProvider::state_event: unknown resource type: {}",
                t
            ),
        }
    }
}

fn do_create<In: for<'de> Deserialize<'de>, Out: serde::Serialize>(
    request: v0::CreateResourceRequest,
    f: impl Fn(In) -> Result<Out>,
) -> std::prelude::v1::Result<v0::CreateResourceResponse, anyhow::Error> {
    let parsed_properties: In = serde_json::from_value(Value::Object(
        request.input_properties.0.into_iter().collect(),
    ))
    .with_context(|| {
        format!(
            "Could not deserialize input properties for {} resource",
            request.type_
        )
    })?;

    let out = f(parsed_properties)?;

    let out_value = serde_json::to_value(out)?;

    let out_object = match out_value {
        Value::Object(o) => o,
        _ => bail!("Expected object as output"),
    };

    let out_properties = out_object.into_iter().collect();

    Ok(v0::CreateResourceResponse {
        output_properties: v0::OutputProperties(out_properties),
    })
}

fn do_update<In: for<'de> Deserialize<'de>, Out: serde::Serialize>(
    request: &v0::UpdateResourceRequest,
    f: impl Fn(In) -> Result<Out>,
) -> std::prelude::v1::Result<v0::UpdateResourceResponse, anyhow::Error> {
    let parsed_properties: In = serde_json::from_value(Value::Object(
        request
            .input_properties
            .0
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    ))
    .with_context(|| {
        format!(
            "Could not deserialize input properties for {} resource",
            request.resource.type_
        )
    })?;

    let out = f(parsed_properties)?;

    let out_value = serde_json::to_value(out)?;

    let out_object = match out_value {
        Value::Object(o) => o,
        _ => bail!("Expected object as output"),
    };

    let out_properties = out_object.into_iter().collect();

    Ok(v0::UpdateResourceResponse {
        output_properties: v0::OutputProperties(out_properties),
    })
}

fn parse_input_properties<T: for<'de> Deserialize<'de>>(
    input_properties: &serde_json::Map<String, serde_json::Value>,
    resource_type: &str,
) -> Result<T> {
    serde_json::from_value(serde_json::Value::Object(input_properties.clone())).with_context(|| {
        format!(
            "Could not deserialize input properties for {} resource",
            resource_type
        )
    })
}

#[tokio::main]
async fn main() {
    run_main(LocalResourceProvider {}).await
}
