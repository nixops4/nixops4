use anyhow::{Context, Result};

use serde_json::Value;
use tracing::warn;

use std::{collections::BTreeMap, fmt::Debug, process::ExitStatus};

use nixops4_resource::schema::v0;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, BufWriter},
    process,
};

pub struct ResourceProviderConfig {
    pub provider_executable: String,
    pub provider_args: Vec<String>,
}

pub struct ResourceProviderClient {
    process: process::Child,
    child_reader: BufReader<process::ChildStdout>,
    /// None: close stdin to let the provider shut down
    child_writer: Option<BufWriter<process::ChildStdin>>,
}
impl ResourceProviderClient {
    pub async fn new(provider_config: ResourceProviderConfig) -> Result<Self> {
        let mut process = process::Command::new(provider_config.provider_executable.clone())
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
        let child_reader = BufReader::new(process.stdout.take().unwrap());
        let child_writer = BufWriter::new(process.stdin.take().unwrap());
        Ok(ResourceProviderClient {
            process,
            child_reader,
            child_writer: Some(child_writer),
        })
    }
    pub async fn close_wait(&mut self) -> Result<ExitStatus> {
        // Close stdin to let the provider shut down
        let _ = self.child_writer.take();
        // Wait for the process to finish
        self.process
            .wait()
            .await
            .context("waiting for provider process to finish")
    }
    fn get_writer(&mut self) -> Result<&mut BufWriter<process::ChildStdin>> {
        self.child_writer.as_mut().ok_or_else(|| {
            anyhow::anyhow!("Can not write to provider while provider is shutting down.")
        })
    }
    async fn write_request(&mut self, req: v0::Request) -> Result<()> {
        let req_str = serde_json::to_string(&req).unwrap();
        let writer = self.get_writer()?;
        writer.write_all(req_str.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();
        Ok(())
    }
    async fn read_response(&mut self) -> Result<v0::Response> {
        let mut response = String::new();
        let n = self.child_reader.read_line(&mut response).await;
        match n {
            Err(e) => {
                anyhow::bail!("Error reading from provider process: {}", e);
            }
            // EOF
            Ok(0) => {
                // Log it
                warn!("Provider process did not return any output");

                // Wait for the process to finish
                let r = self.process.wait().await?;

                if r.success() {
                    anyhow::bail!("Provider process did not return any output");
                } else {
                    bail_provider_exit_code(r)?
                }
            }
            Ok(_) => Ok(serde_json::from_str(&response)?),
        }
    }
    pub async fn create(
        &mut self,
        type_: &str,
        inputs: &BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Value>> {
        let req = v0::CreateResourceRequest {
            input_properties: inputs.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            type_: type_.to_string(),
            is_stateful: false,
        };

        // Write the request
        self.write_request(v0::Request::CreateResourceRequestEnvelope(
            v0::RequestOutputPropertiesCreateResourceRequestEnvelope {
                create_resource_request: req,
            },
        ))
        .await?;

        let response = self.read_response().await?;
        match response {
            v0::Response::CreateResourceResponseEnvelope(r) => Ok(r
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

    pub async fn update(
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
        self.write_request(v0::Request::UpdateResourceRequestEnvelope(
            v0::RequestOutputPropertiesUpdateResourceRequestEnvelope {
                update_resource_request: req,
            },
        ))
        .await?;

        let response = self.read_response().await?;
        match response {
            v0::Response::UpdateResourceResponseEnvelope(r) => Ok(r
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

    pub async fn state_read(
        &mut self,
        resource: v0::ExtantResource,
    ) -> Result<BTreeMap<String, Value>> {
        let req = v0::StateResourceReadRequest { resource: resource };
        // Write the request
        self.write_request(v0::Request::StateResourceReadRequestEnvelope(
            v0::RequestOutputPropertiesStateResourceReadRequestEnvelope {
                state_resource_read_request: req,
            },
        ))
        .await?;
        eprintln!("State read request sent");
        let response = self.read_response().await?;
        eprintln!("State read response received");
        match response {
            v0::Response::StateResourceReadResponseEnvelope(r) => {
                Ok(r.state_resource_read_response.state)
            }
            _ => anyhow::bail!(
                "Expected StateResourceReadResponse from provider but got: {:?}",
                response
            ),
        }
    }

    pub async fn state_event(&mut self, event: v0::StateResourceEvent) -> Result<()> {
        // Write the request
        self.write_request(v0::Request::StateResourceEventEnvelope(
            v0::RequestOutputPropertiesStateResourceEventEnvelope {
                state_resource_event: event,
            },
        ))
        .await?;

        let response = self.read_response().await?;
        match response {
            v0::Response::StateResourceEventResponseEnvelope(_) => Ok(()),
            _ => anyhow::bail!(
                "Expected StateResourceEventResponse from provider but got: {:?}",
                response
            ),
        }
    }
}
impl Debug for ResourceProviderClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceProviderClient")
            .field("process", &self.process)
            .finish()
    }
}

fn bail_provider_exit_code<Absurd>(r: std::process::ExitStatus) -> Result<Absurd> {
    anyhow::bail!("Provider process failed with exit code: {}", r);
}
