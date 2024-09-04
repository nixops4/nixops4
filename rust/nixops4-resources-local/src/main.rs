use std::io::Write;

use anyhow::{bail, Context, Result};
use nixops4_resource::framework::run_main;
use nixops4_resource::{schema::v0::CreateResourceRequest, schema::v0::CreateResourceResponse};
use serde::Deserialize;
use serde_json::Value;

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
    command: String,
    args: Vec<String>,
    stdin: Option<String>,
    // TODO parseJSON: bool  (for convenience and presentation purposes)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct ExecOutProperties {
    stdout: String,
}

impl nixops4_resource::framework::ResourceProvider for LocalResourceProvider {
    fn create(&self, request: CreateResourceRequest) -> Result<CreateResourceResponse> {
        match request.type_.as_str() {
            "file" => do_create(request, |p: FileInProperties| {
                std::fs::write(&p.name, &p.contents)?;
                Ok(FileOutProperties {})
            }),
            "exec" => do_create(request, |p: ExecInProperties| {
                let mut command = std::process::Command::new(&p.command);
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
                    .with_context(|| format!("Could not spawn command: {}", p.command))?;

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

                Ok(ExecOutProperties { stdout })
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
        request.input_properties.into_iter().collect(),
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
