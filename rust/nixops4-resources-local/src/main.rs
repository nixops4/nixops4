mod state;

use std::fs::OpenOptions;
use std::io::{self, Write};

use anyhow::{bail, Context, Result};
use nixops4_resource::framework::run_main;
use nixops4_resource::{schema::v0::CreateResourceRequest, schema::v0::CreateResourceResponse};
use serde::Deserialize;
use serde_json::Value;
use state::StateHandle;

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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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
}

fn do_create<In: for<'de> Deserialize<'de>, Out: serde::Serialize>(
    request: CreateResourceRequest,
    f: impl Fn(In) -> Result<Out>,
) -> std::prelude::v1::Result<CreateResourceResponse, anyhow::Error> {
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

    Ok(CreateResourceResponse {
        output_properties: out_properties,
    })
}

fn main() {
    run_main(LocalResourceProvider {})
}
