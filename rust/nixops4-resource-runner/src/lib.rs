mod rpc;

use anyhow::{Context, Result};
use nixops4_resource::schema::v0;
use serde_json::Value;
use std::process::ExitStatus;
use tokio::process;

use nixops4_resource::rpc::ResourceProviderRpcClient;

use crate::rpc::build_rpc_client_from_child;

pub struct ResourceProviderConfig {
    pub provider_executable: String,
    pub provider_args: Vec<String>,
}

pub struct ResourceProviderClient {
    process: process::Child,
    rpc_client: Option<jsonrpsee::async_client::Client>,
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

        let rpc_client = build_rpc_client_from_child(&mut process);

        Ok(ResourceProviderClient {
            process,
            rpc_client: Some(rpc_client),
        })
    }
    pub async fn close_wait(&mut self) -> Result<ExitStatus> {
        // Close stdin to let the provider shut down
        let _ = self.rpc_client.take();
        // Wait for the process to finish
        self.process
            .wait()
            .await
            .context("waiting for provider process to finish")
    }
    fn get_client(&self) -> Result<&jsonrpsee::async_client::Client> {
        self.rpc_client.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Can not write to provider while provider is shutting down.")
        })
    }
    pub async fn create(
        &self,
        type_: &str,
        inputs: &serde_json::Map<String, Value>,
        is_stateful: bool,
    ) -> Result<serde_json::Map<String, Value>> {
        let response = self
            .get_client()?
            .create(type_.to_string(), inputs.clone(), is_stateful)
            .await?;

        Ok(response.output_properties.0)
    }

    pub async fn update(
        &self,
        type_: &str,
        inputs: &serde_json::Map<String, Value>,
        previous_inputs: &serde_json::Map<String, Value>,
        previous_outputs: &serde_json::Map<String, Value>,
    ) -> Result<serde_json::Map<String, Value>> {
        let resource = v0::ExtantResource {
            type_: v0::ResourceType(type_.to_string()),
            input_properties: v0::InputProperties(previous_inputs.clone()),
            output_properties: Some(v0::OutputProperties(previous_outputs.clone())),
        };
        let response = self.get_client()?.update(resource, inputs.clone()).await?;

        Ok(response.output_properties.0)
    }

    pub async fn state_read(
        &self,
        resource: v0::ExtantResource,
    ) -> Result<serde_json::Map<String, Value>> {
        let response = self.get_client()?.state_read(resource).await?;
        Ok(response.state)
    }

    pub async fn state_event(&self, request: v0::StateResourceEvent) -> Result<()> {
        let _ = self.get_client()?.state_event(request).await?;
        Ok(())
    }
}
