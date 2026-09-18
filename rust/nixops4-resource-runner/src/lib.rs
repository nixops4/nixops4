use anyhow::{Context, Result};
use nixops4_resource::schema::v0;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::process::ExitStatus;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, BufWriter},
    process,
};
use tracing::warn;

pub struct ResourceProviderConfig {
    pub provider_executable: String,
    pub provider_args: Vec<String>,
}

/// Outputs and provider-owned private state returned by operations that
/// produce a resource state, such as Create or Update, probably Read
/// when implemented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceOutcome {
    pub output_properties: serde_json::Map<String, Value>,
    /// Provider-owned opaque JSON. `None` when the provider did not give
    /// us private state, which on an Update also covers clearing what it
    /// had written before. NixOps4 stores it verbatim and hands it back
    /// operation, e.g. Update or Destroy.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub private_state: Option<Value>,
}

pub struct ResourceProviderClient {
    process: process::Child,
    child_reader: BufReader<process::ChildStdout>,
    /// `None` to close stdin and let the provider shut down
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
            Ok(_) => {
                let resp: v0::Response = serde_json::from_str(&response)?;
                if let v0::Response::ErrorResponse(e) = resp {
                    anyhow::bail!("{}", e.message);
                }
                Ok(resp)
            }
        }
    }
    pub async fn create(
        &mut self,
        type_: &str,
        inputs: &serde_json::Map<String, Value>,
        is_stateful: bool,
    ) -> Result<ResourceOutcome> {
        let req = v0::CreateResourceRequest {
            input_properties: v0::InputProperties(inputs.clone()),
            type_: v0::ResourceType(type_.to_string()),
            is_stateful,
        };

        // Write the request
        self.write_request(v0::Request::CreateResourceRequest(req))
            .await?;

        let response = self.read_response().await?;
        match response {
            v0::Response::CreateResourceResponse(r) => Ok(ResourceOutcome {
                output_properties: r.output_properties.0,
                private_state: r.private_state.map(|p| p.0),
            }),
            _ => anyhow::bail!(
                "Expected CreateResourceResponse from provider but got: {:?}",
                response
            ),
        }
    }

    pub async fn update(
        &mut self,
        type_: &str,
        inputs: &serde_json::Map<String, Value>,
        previous_inputs: &serde_json::Map<String, Value>,
        previous_outputs: &serde_json::Map<String, Value>,
        previous_private_state: Option<Value>,
    ) -> Result<ResourceOutcome> {
        let req = v0::UpdateResourceRequest {
            resource: v0::ExtantResource {
                type_: v0::ResourceType(type_.to_string()),
                input_properties: v0::InputProperties(previous_inputs.clone()),
                output_properties: Some(v0::OutputProperties(previous_outputs.clone())),
                private_state: previous_private_state.map(v0::PrivateState),
            },
            input_properties: v0::InputProperties(inputs.clone()),
        };

        // Write the request
        self.write_request(v0::Request::UpdateResourceRequest(req))
            .await?;

        let response = self.read_response().await?;
        match response {
            v0::Response::UpdateResourceResponse(r) => Ok(ResourceOutcome {
                output_properties: r.output_properties.0,
                private_state: r.private_state.map(|p| p.0),
            }),
            _ => anyhow::bail!(
                "Expected UpdateResourceResponse from provider but got: {:?}",
                response
            ),
        }
    }

    pub async fn state_read(
        &mut self,
        resource: v0::ExtantResource,
    ) -> Result<serde_json::Map<String, Value>> {
        let req = v0::StateResourceReadRequest { resource };

        // Write the request
        self.write_request(v0::Request::StateResourceReadRequest(req))
            .await?;

        let response = self.read_response().await?;
        match response {
            v0::Response::StateResourceReadResponse(r) => Ok(r.state),
            _ => anyhow::bail!(
                "Expected StateResourceReadResponse from provider but got: {:?}",
                response
            ),
        }
    }

    pub async fn destroy(&mut self, resource: v0::ExtantResource) -> Result<()> {
        let req = v0::DestroyResourceRequest { resource };

        self.write_request(v0::Request::DestroyResourceRequest(req))
            .await?;

        let response = self.read_response().await?;
        match response {
            v0::Response::DestroyResourceResponse(_) => Ok(()),
            _ => anyhow::bail!(
                "Expected DestroyResourceResponse from provider but got: {:?}",
                response
            ),
        }
    }

    pub async fn state_event(&mut self, request: v0::StateResourceEvent) -> Result<()> {
        // Write the request
        self.write_request(v0::Request::StateResourceEvent(request))
            .await?;

        let response = self.read_response().await?;
        match response {
            v0::Response::StateResourceEventResponse(_) => Ok(()),
            _ => anyhow::bail!(
                "Expected StateResourceEventResponse from provider but got: {:?}",
                response
            ),
        }
    }
}

fn bail_provider_exit_code<Absurd>(r: std::process::ExitStatus) -> Result<Absurd> {
    anyhow::bail!("Provider process failed with exit code: {}", r);
}
