use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use jsonrpsee::{
    async_client::ClientBuilder,
    core::client::{MaybeSend, ReceivedMessage, TransportReceiverT, TransportSenderT},
};
use nixops4_resource::{framework::ContentLengthCodec, schema::v0};
use serde_json::Value;
use std::{future::Future, process::ExitStatus};
use tokio::{
    io::{BufReader, BufWriter},
    process,
};
use tokio_util::codec::{FramedRead, FramedWrite};

use nixops4_resource::rpc::ResourceProviderRpcClient;

pub struct ResourceProviderConfig {
    pub provider_executable: String,
    pub provider_args: Vec<String>,
}

// Annoying hack because anyhow:Error doesn't implement StdError and
// the TransportSender/Receiver traits require an StdError
struct AnyhowError(anyhow::Error);

impl From<anyhow::Error> for AnyhowError {
    fn from(value: anyhow::Error) -> Self {
        Self(value)
    }
}

impl std::error::Error for AnyhowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl std::fmt::Display for AnyhowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::fmt::Debug for AnyhowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

struct StdIoSender {
    writer: FramedWrite<BufWriter<process::ChildStdin>, ContentLengthCodec>,
}

impl TransportSenderT for StdIoSender {
    type Error = AnyhowError;

    fn send(
        &mut self,
        msg: String,
    ) -> impl Future<Output = std::result::Result<(), Self::Error>> + MaybeSend {
        async move {
            tracing::info!("Sending message: {}", &msg);
            self.writer.send(&msg).await?;
            Ok(())
        }
    }
}

struct StdIoReceiver {
    reader: FramedRead<BufReader<process::ChildStdout>, ContentLengthCodec>,
}

impl TransportReceiverT for StdIoReceiver {
    type Error = AnyhowError;

    fn receive(
        &mut self,
    ) -> impl Future<Output = std::result::Result<ReceivedMessage, Self::Error>> + MaybeSend {
        async move {
            match self.reader.next().await {
                Some(Ok(msg)) => {
                    tracing::info!("Received message {}", &msg);
                    Ok(ReceivedMessage::Text(msg))
                }
                Some(Err(e)) => Err(e.into()),
                None => Err(anyhow::anyhow!("EOF reached while reading response").into()),
            }
        }
    }
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
        let child_reader = FramedRead::new(
            BufReader::new(process.stdout.take().unwrap()),
            ContentLengthCodec::default(),
        );
        let child_writer = FramedWrite::new(
            BufWriter::new(process.stdin.take().unwrap()),
            ContentLengthCodec::default(),
        );

        let rpc_client = ClientBuilder::new().build_with_tokio(
            StdIoSender {
                writer: child_writer,
            },
            StdIoReceiver {
                reader: child_reader,
            },
        );

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
