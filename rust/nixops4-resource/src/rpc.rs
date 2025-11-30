use async_trait::async_trait;
use jsonrpsee::{
    core::RpcResult,
    proc_macros::rpc,
    types::{ErrorCode, ErrorObject, ErrorObjectOwned},
};
use serde_json::Value;

use crate::{framework::ResourceProvider, schema::v0};

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
