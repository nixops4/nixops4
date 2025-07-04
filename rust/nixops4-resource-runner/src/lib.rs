use anyhow::{Context, Result};
use nixops4_resource::schema::v0;
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
    async fn write_request(&mut self, req: v0::CreateResourceRequest) -> Result<()> {
        let req_str = serde_json::to_string(&req).unwrap();
        let writer = self.get_writer()?;
        writer.write_all(req_str.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();
        Ok(())
    }
    async fn read_response(&mut self) -> Result<v0::CreateResourceResponse> {
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
        inputs: &serde_json::Map<String, Value>,
    ) -> Result<serde_json::Map<String, Value>> {
        let req = v0::CreateResourceRequest {
            input_properties: inputs.clone(),
            type_: type_.to_string(),
        };

        // Write the request
        self.write_request(req).await?;

        Ok(self.read_response().await?.output_properties)
    }
}

fn bail_provider_exit_code<Absurd>(r: std::process::ExitStatus) -> Result<Absurd> {
    anyhow::bail!("Provider process failed with exit code: {}", r);
}
