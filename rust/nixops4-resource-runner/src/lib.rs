use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, BufWriter, Write},
    process,
};

use anyhow::{Context, Result};
use nixops4_resource::schema::v0::{
    self, CreateResourceRequest, Request, RequestOutputPropertiesCreateResourceRequestEnvelope,
    Response,
};
use serde_json::Value;
use tracing::warn;

pub struct ResourceProviderConfig {
    pub provider_executable: String,
    pub provider_args: Vec<String>,
}

pub struct ResourceProviderClient {
    provider_config: ResourceProviderConfig,
    process: process::Child,
    child_reader: BufReader<process::ChildStdout>,
    /// None: close stdin to let the provider shut down
    child_writer: Option<BufWriter<process::ChildStdin>>,
}

impl ResourceProviderClient {
    pub fn new(provider_config: ResourceProviderConfig) -> Result<Self> {
        let mut process = std::process::Command::new(provider_config.provider_executable.clone())
            .args(provider_config.provider_args.clone())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .with_context(|| {
                format!(
                    "Could not spawn provider process {}",
                    provider_config.provider_executable
                )
            })?;
        let child_reader = std::io::BufReader::new(process.stdout.take().unwrap());
        let child_writer = std::io::BufWriter::new(process.stdin.take().unwrap());
        Ok(ResourceProviderClient {
            provider_config,
            process,
            child_reader,
            child_writer: Some(child_writer),
        })
    }

    fn get_writer(&mut self) -> Result<&mut BufWriter<process::ChildStdin>> {
        self.child_writer.as_mut().ok_or_else(|| {
            anyhow::anyhow!("Can not write to provider while provider is shutting down.")
        })
    }

    fn write_request(&mut self, req: Request) -> Result<()> {
        let stdin_str = serde_json::to_string(&req).unwrap();
        let writer = self.get_writer()?;
        writer.write_all(stdin_str.as_bytes()).unwrap();
        writer.write_all(b"\n").unwrap();
        writer.flush().unwrap();
        Ok(())
    }

    fn read_response(&mut self) -> Result<Response> {
        let mut response = String::new();
        let n = self.child_reader.read_line(&mut response);
        match n {
            Err(e) => {
                anyhow::bail!("Error reading from provider process: {}", e);
            }
            // EOF
            Ok(0) => {
                // Log it
                warn!("Provider process did not return any output");

                // Wait for the process to finish
                let r = self.process.wait()?;

                if r.success() {
                    anyhow::bail!("Provider process did not return any output");
                } else {
                    bail_provider_exit_code(r)?
                }
            }
            Ok(_) => Ok(serde_json::from_str(&response)?),
        }
    }

    pub fn create(
        &mut self,
        type_: &str,
        inputs: &BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Value>> {
        let req = CreateResourceRequest {
            input_properties: inputs.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            type_: type_.to_string(),
            is_stateful: false,
        };

        // Write the request
        self.write_request(Request::CreateResourceRequestEnvelope(
            RequestOutputPropertiesCreateResourceRequestEnvelope {
                create_resource_request: req,
            },
        ))?;

        let response = self.read_response()?;
        match response {
            Response::CreateResourceResponseEnvelope(r) => Ok(r
                .create_resource_response
                .output_properties
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()),
            _ => anyhow::bail!(
                "Expected CreateResourceResponse from provider but got: {:?}",
                response
            ),
        }
    }

    pub fn update(
        &mut self,
        type_: &str,
        inputs: &BTreeMap<String, Value>,
        previous_inputs: &BTreeMap<String, Value>,
        previous_outputs: &BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Value>> {
        let res = v0::ExtantResource {
            input_properties: previous_inputs
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            output_properties: Some(
                previous_outputs
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
            type_: type_.to_string(),
        };
        let req = v0::UpdateResourceRequest {
            input_properties: inputs.clone(),
            resource: res,
        };
        // Write the request
        self.write_request(Request::UpdateResourceRequestEnvelope(
            v0::RequestOutputPropertiesUpdateResourceRequestEnvelope {
                update_resource_request: req,
            },
        ))?;

        let response = self.read_response()?;
        match response {
            Response::UpdateResourceResponseEnvelope(r) => Ok(r
                .update_resource_response
                .output_properties
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()),
            _ => anyhow::bail!(
                "Expected UpdateResourceResponse from provider but got: {:?}",
                response
            ),
        }
    }
}
impl Drop for ResourceProviderClient {
    fn drop(&mut self) {
        // Close stdin to let the provider shut down
        drop(self.child_writer.take());
        // Wait for the process to finish
        let r = self.process.wait().unwrap();
        if !r.success() {
            warn!("Provider process failed with exit code: {}", r);
        }
    }
}

fn bail_provider_exit_code<Absurd>(r: std::process::ExitStatus) -> Result<Absurd> {
    anyhow::bail!("Provider process failed with exit code: {}", r);
}
