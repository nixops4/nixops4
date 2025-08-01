mod state;

use std::fs::OpenOptions;
use std::io::{self, Write};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use nixops4_resource::framework::run_main;
use nixops4_resource::schema::v0;
use nixops4_resource::{schema::v0::CreateResourceRequest, schema::v0::CreateResourceResponse};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use state::{StateEvent, StateEventMeta, StateHandle};

use crate::state::StateEventStream;

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

/// A state provider resource generally doesn't have outputs. Any access of the
/// stored state would be out of sync with the current evaluation of the deployment.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct StateFileOutProperties {}

impl nixops4_resource::framework::ResourceProvider for LocalResourceProvider {
    fn create(&self, request: CreateResourceRequest) -> Result<CreateResourceResponse> {
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
            "memo" => do_create(request, |p: MemoInProperties| {
                // A stateful resource that is initialized upon creation and
                // not modified afterwards, except perhaps through a manual
                // operation or a migration of sorts
                Ok(MemoOutProperties {
                    value: p.initialize_with,
                })
            }),
            t => bail!(
                "LocalResourceProvider::create: unknown resource type: {}",
                t
            ),
        }
    }

    fn read(&self, request: v0::ReadResourceRequest) -> Result<v0::ReadResourceResponse> {
        match request.resource.type_.as_str() {
            "file" => {
                // Note that it's not a terraform-style data source
                todo!();
            }
            "exec" => {
                // ??
                todo!();
            }
            "state_file" => do_read(&request, |_p: StateFileInProperties| {
                Ok(StateFileOutProperties {})
            }),
            "memo" => {
                // TODO test
                do_read(&request, |_p: MemoInProperties| {
                    // let previous_input_properties = serde_json::from_value(Value::Object(
                    //     request.resource.input_properties.into_iter().collect(),
                    // ))
                    let previous_output_properties = request.resource.output_properties.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("The read operation on a memo resource requires that the output properties are set")
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
                })
            }
            t => bail!("LocalResourceProvider::read: unknown resource type: {}", t),
        }
    }

    fn destroy(&self, request: v0::DestroyResourceRequest) -> Result<v0::DestroyResourceResponse> {
        match request.resource.type_.as_str() {
            "file" => {
                todo!();
            }
            "exec" => {
                todo!();
            }
            "state_file" => {
                todo!();
            }
            "memo" => {
                todo!();
            }
            t => bail!(
                "LocalResourceProvider::destroy: unknown resource type: {}",
                t
            ),
        }
    }

    fn update(&self, request: v0::UpdateResourceRequest) -> Result<v0::UpdateResourceResponse> {
        match request.resource.type_.as_str() {
            "file" => {
                todo!();
            }
            "exec" => {
                todo!();
            }
            "state_file" => {
                todo!();
            }
            "memo" => {
                do_update(&request, |_p: MemoInProperties| {
                    // let previous_input_properties = serde_json::from_value(Value::Object(
                    //     request.resource.input_properties.into_iter().collect(),
                    // ))
                    let previous_output_properties = request.resource.output_properties.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("The read operation on a memo resource requires that the output properties are set")
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
                })
            }
            t => bail!(
                "LocalResourceProvider::update: unknown resource type: {}",
                t
            ),
        }
    }

    fn state_read(
        &self,
        request: v0::StateResourceReadRequest,
    ) -> Result<v0::StateResourceReadResponse> {
        match request.resource.type_.as_str() {
            "state_file" => {
                let inputs = parse_input_properties::<StateFileInProperties>(&request.resource.input_properties, &request.resource.type_)?;

                let file_contents = std::fs::read_to_string(inputs.name.as_str())?;
                let stream = StateEventStream::open_from_reader(
                    io::BufReader::new(file_contents.as_bytes()),
                )?;
                let mut state = serde_json::json!({});
                state::apply_state_events(&mut state, stream).unwrap();
                Ok(v0::StateResourceReadResponse {
                    state: serde_json::from_value(state)?,
                })
            },
            t => bail!(
                "LocalResourceProvider::state_read: not a state resource, or unknown resource type: {}",
                t
            ),
        }
    }

    fn state_event(
        &self,
        request: v0::StateResourceEvent,
    ) -> Result<v0::StateResourceEventResponse> {
        match request.resource.type_.as_str() {
            "state_file" => {
                let inputs = parse_input_properties::<StateFileInProperties>(&request.resource.input_properties, &request.resource.type_)?;
                let mut handle = StateHandle::open(inputs.name.as_str(), false)?;

                handle.append(&[&StateEvent {
                    index: 0,
                    meta: StateEventMeta {
                        time: Utc::now().to_rfc3339(),
                        other_fields: serde_json::json!({
                            "event": request.event,
                        }),
                    },
                    patch: serde_json::from_value(serde_json::Value::Array(request.patch))?,
                }])?;
                Ok(v0::StateResourceEventResponse { })
            },
            t => bail!(
                "LocalResourceProvider::state_read: not a state resource, or unknown resource type: {}",
                t
            ),
        }
    }
}

fn do_create<In: for<'de> Deserialize<'de>, Out: serde::Serialize>(
    request: v0::CreateResourceRequest,
    f: impl Fn(In) -> Result<Out>,
) -> std::prelude::v1::Result<CreateResourceResponse, anyhow::Error> {
    let parsed_properties = parse_input_properties(&request.input_properties, &request.type_)?;

    let out = f(parsed_properties)?;

    let out_value = serde_json::to_value(out)?;

    let out_object = match out_value {
        Value::Object(o) => o,
        _ => bail!("Expected object as output"),
    };

    let out_properties = out_object.into_iter().collect();

    Ok(CreateResourceResponse {
        output_properties: out_properties,
    })
}

fn parse_input_properties<In: for<'de> Deserialize<'de>>(
    input_properties: &v0::InputProperties,
    type_: &String,
) -> Result<In, anyhow::Error> {
    let parsed_properties: In = serde_json::from_value(Value::Object(
        input_properties
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    ))
    .with_context(|| {
        format!(
            "Could not deserialize input properties for {} resource",
            type_
        )
    })?;
    Ok(parsed_properties)
}

fn do_read<In: for<'de> Deserialize<'de>, Out: Serialize>(
    request: &v0::ReadResourceRequest,
    f: impl Fn(In) -> Result<Out>,
) -> std::prelude::v1::Result<v0::ReadResourceResponse, anyhow::Error> {
    let parsed_properties: In = serde_json::from_value(Value::Object(
        (&request.resource.input_properties)
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

    Ok(v0::ReadResourceResponse {
        output_properties: v0::OutputProperties(out_object),
    })
}

fn do_update<In: for<'de> Deserialize<'de>, Out: serde::Serialize>(
    request: &v0::UpdateResourceRequest,
    f: impl Fn(In) -> Result<Out>,
) -> std::prelude::v1::Result<v0::UpdateResourceResponse, anyhow::Error> {
    let parsed_properties: In = serde_json::from_value(Value::Object(
        (&request.input_properties)
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

    Ok(v0::UpdateResourceResponse {
        output_properties: v0::OutputProperties(out_object),
    })
}

fn do_destroy<In: for<'de> Deserialize<'de>>(
    request: &v0::DestroyResourceRequest,
    f: impl Fn(In) -> Result<()>,
) -> std::prelude::v1::Result<v0::DestroyResourceResponse, anyhow::Error> {
    let parsed_properties: In = serde_json::from_value(Value::Object(
        request
            .resource
            .input_properties
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

    f(parsed_properties)?;

    Ok(v0::DestroyResourceResponse {})
}

fn main() {
    run_main(LocalResourceProvider {})
}
