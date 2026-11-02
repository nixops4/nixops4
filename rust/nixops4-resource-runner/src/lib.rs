use anyhow::{Context, Result};
use nixops4_provider_wire::schema::v0;
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

/// The type of a resource, as a provider knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceType(String);

impl ResourceType {
    pub fn new(type_: impl Into<String>) -> Self {
        ResourceType(type_.into())
    }
}

impl AsRef<str> for ResourceType {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A resource as addressed to its provider: its type and its input and output
/// properties.
#[derive(Debug, Clone)]
pub struct ProviderResource {
    pub type_: ResourceType,
    pub input_properties: serde_json::Map<String, Value>,
    pub output_properties: Option<serde_json::Map<String, Value>>,
}

impl ProviderResource {
    fn to_wire(&self) -> v0::ExtantResource {
        v0::ExtantResource {
            type_: v0::ResourceType(self.type_.as_ref().to_string()),
            input_properties: v0::InputProperties(self.input_properties.clone()),
            output_properties: self.output_properties.clone().map(v0::OutputProperties),
        }
    }
}

pub struct StateEvent {
    pub event: String,
    pub nixops_version: String,
    pub patch: json_patch::Patch,
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

    pub async fn create(
        &mut self,
        type_: &ResourceType,
        input_properties: &serde_json::Map<String, Value>,
        is_stateful: bool,
    ) -> Result<serde_json::Map<String, Value>> {
        let request = v0::CreateResourceRequest {
            input_properties: v0::InputProperties(input_properties.clone()),
            type_: v0::ResourceType(type_.as_ref().to_string()),
            is_stateful,
        };
        let response = self
            .call(v0::Request::CreateResourceRequest(request))
            .await?;
        match response {
            v0::Response::CreateResourceResponse(r) => Ok(r.output_properties.0),
            _ => Err(unexpected_response("CreateResourceResponse", &response)),
        }
    }

    pub async fn update(
        &mut self,
        previous: &ProviderResource,
        input_properties: &serde_json::Map<String, Value>,
    ) -> Result<serde_json::Map<String, Value>> {
        let request = v0::UpdateResourceRequest {
            resource: previous.to_wire(),
            input_properties: v0::InputProperties(input_properties.clone()),
        };
        let response = self
            .call(v0::Request::UpdateResourceRequest(request))
            .await?;
        match response {
            v0::Response::UpdateResourceResponse(r) => Ok(r.output_properties.0),
            _ => Err(unexpected_response("UpdateResourceResponse", &response)),
        }
    }

    pub async fn state_read(
        &mut self,
        resource: &ProviderResource,
    ) -> Result<serde_json::Map<String, Value>> {
        let request = v0::StateResourceReadRequest {
            resource: resource.to_wire(),
        };
        let response = self
            .call(v0::Request::StateResourceReadRequest(request))
            .await?;
        match response {
            v0::Response::StateResourceReadResponse(r) => Ok(r.state),
            _ => Err(unexpected_response("StateResourceReadResponse", &response)),
        }
    }

    pub async fn state_event(
        &mut self,
        resource: &ProviderResource,
        event: StateEvent,
    ) -> Result<()> {
        let request = v0::StateResourceEvent {
            resource: resource.to_wire(),
            event: event.event,
            nixops_version: event.nixops_version,
            patch: event.patch,
        };
        let response = self.call(v0::Request::StateResourceEvent(request)).await?;
        match response {
            v0::Response::StateResourceEventResponse(_) => Ok(()),
            _ => Err(unexpected_response("StateResourceEventResponse", &response)),
        }
    }

    async fn call(&mut self, request: v0::Request) -> Result<v0::Response> {
        self.write_request(&request).await?;
        self.read_response().await
    }

    async fn write_request(&mut self, request: &v0::Request) -> Result<()> {
        let request_str = serde_json::to_string(request).unwrap();
        let writer = self.get_writer()?;
        writer.write_all(request_str.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();
        Ok(())
    }

    async fn read_response(&mut self) -> Result<v0::Response> {
        match self.read_line().await? {
            Some(line) => parse_response(&line),
            None => Err(self.exit_error().await),
        }
    }

    /// Read one line from the provider; `None` on EOF.
    async fn read_line(&mut self) -> Result<Option<String>> {
        let mut line = String::new();
        let n = self.child_reader.read_line(&mut line).await;
        match n {
            Err(e) => anyhow::bail!("Error reading from provider process: {}", e),
            Ok(0) => Ok(None),
            Ok(_) => Ok(Some(line)),
        }
    }

    /// The error for a provider that exited without answering: wait for it
    /// and report how it ended.
    async fn exit_error(&mut self) -> anyhow::Error {
        warn!("Provider process did not return any output");
        let status = match self.process.wait().await {
            Ok(status) => status,
            Err(e) => return e.into(),
        };
        if status.success() {
            anyhow::anyhow!("Provider process did not return any output")
        } else {
            anyhow::anyhow!("Provider process failed with exit code: {}", status)
        }
    }
}

fn parse_response(line: &str) -> Result<v0::Response> {
    let response: v0::Response = serde_json::from_str(line)?;
    if let v0::Response::ErrorResponse(e) = &response {
        anyhow::bail!("{}", e.message);
    }
    Ok(response)
}

fn unexpected_response(expected: &str, got: &v0::Response) -> anyhow::Error {
    anyhow::anyhow!("Expected {} from provider but got: {:?}", expected, got)
}
