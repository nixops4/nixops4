use std::fmt::Formatter;

use async_trait::async_trait;
use futures_util::{SinkExt as _, StreamExt as _};
use jsonrpsee::{
    core::{
        client::{ReceivedMessage, TransportReceiverT, TransportSenderT},
        RpcResult,
    },
    proc_macros::rpc,
    types::{ErrorCode, ErrorObject, ErrorObjectOwned},
};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{FramedRead, FramedWrite};

use crate::{
    framework::{ContentLengthCodec, ResourceProvider},
    schema::v0,
};

#[rpc(client, server, namespace = "resource")]
pub trait ResourceProviderRpc {
    #[method(name = "create")]
    async fn create(
        &self,
        #[argument(rename = "type")] type_: String,
        input_properties: serde_json::Map<String, Value>,
        is_stateful: bool,
    ) -> RpcResult<v0::CreateResourceResponse>;

    #[method(name = "update")]
    async fn update(
        &self,
        resource: v0::ExtantResource,
        input_properties: serde_json::Map<String, Value>,
    ) -> RpcResult<v0::UpdateResourceResponse>;

    #[method(name = "state_read")]
    async fn state_read(
        &self,
        resource: v0::ExtantResource,
    ) -> RpcResult<v0::StateResourceReadResponse>;

    #[method(name = "state_event")]
    async fn state_event(
        &self,
        request: v0::StateResourceEvent,
    ) -> RpcResult<v0::StateResourceEventResponse>;
}

#[async_trait]
impl<T> ResourceProviderRpcServer for T
where
    T: ResourceProvider,
{
    async fn create(
        &self,
        type_: String,
        inputs: serde_json::Map<String, Value>,
        is_stateful: bool,
    ) -> RpcResult<v0::CreateResourceResponse> {
        let req = v0::CreateResourceRequest {
            input_properties: v0::InputProperties(inputs.clone()),
            type_: v0::ResourceType(type_.to_string()),
            is_stateful,
        };

        self.create(req).await.map_err(handle_error)
    }

    async fn update(
        &self,
        resource: v0::ExtantResource,
        inputs: serde_json::Map<String, Value>,
    ) -> RpcResult<v0::UpdateResourceResponse> {
        let req = v0::UpdateResourceRequest {
            resource,
            input_properties: v0::InputProperties(inputs.clone()),
        };

        self.update(req).await.map_err(handle_error)
    }

    async fn state_read(
        &self,
        resource: v0::ExtantResource,
    ) -> RpcResult<v0::StateResourceReadResponse> {
        let req = v0::StateResourceReadRequest { resource };

        self.state_read(req).await.map_err(handle_error)
    }

    async fn state_event(
        &self,
        request: v0::StateResourceEvent,
    ) -> RpcResult<v0::StateResourceEventResponse> {
        ResourceProvider::state_event(self, request)
            .await
            .map_err(handle_error)
    }
}

fn handle_error(error: anyhow::Error) -> ErrorObjectOwned {
    eprintln!("Error: {:?}", error);
    ErrorObject::owned(
        ErrorCode::InternalError.code(),
        "Resource provider encountered an error",
        Some(error.to_string()),
    )
}

pub struct ContentLengthSender<T> {
    inner: FramedWrite<T, ContentLengthCodec>,
}

impl<T: AsyncWrite> ContentLengthSender<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner: FramedWrite::new(inner, ContentLengthCodec::default()),
        }
    }
}

impl<T> TransportSenderT for ContentLengthSender<T>
where
    T: AsyncWrite + Unpin + Send + 'static,
{
    type Error = AnyhowError;

    async fn send(&mut self, msg: String) -> std::result::Result<(), Self::Error> {
        self.inner.send(&msg).await?;
        Ok(())
    }
}

pub struct ContentLengthReceiver<T> {
    inner: FramedRead<T, ContentLengthCodec>,
}

impl<T: AsyncRead> ContentLengthReceiver<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner: FramedRead::new(inner, ContentLengthCodec::default()),
        }
    }
}

impl<T> TransportReceiverT for ContentLengthReceiver<T>
where
    T: AsyncRead + Unpin + Send + 'static,
{
    type Error = AnyhowError;

    async fn receive(&mut self) -> Result<ReceivedMessage, Self::Error> {
        match self.inner.next().await {
            Some(Ok(msg)) => Ok(ReceivedMessage::Text(msg)),
            Some(Err(e)) => Err(e.into()),
            None => Err(anyhow::anyhow!("EOF reached while reading response").into()),
        }
    }
}

// Annoying hack because anyhow:Error doesn't implement StdError and
// the TransportSender/Receiver traits require an StdError
pub struct AnyhowError(anyhow::Error);

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
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::fmt::Debug for AnyhowError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
