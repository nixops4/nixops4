/// This module supplements the `nixops4-resource-runner` library with
/// evaluation-layer logic.
use anyhow::{bail, Result};
use nixops4_core::eval_api::ResourceProviderInfo;
use nixops4_resource_runner::{ResourceProviderClient, ResourceProviderConfig};
use serde_json::Value;

/// This type implements the parsing of `type: "stdio"` providers.
#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
pub(crate) struct ProviderStdio {
    pub(crate) executable: String,
    pub(crate) args: Vec<String>,
}

/// Start provider process based on resource declaration
/// Not memoized. Where the provider can be reached through a memoized goal
/// instead, as a state resource is through `Goal::RunState`, prefer that: it
/// shares one process where this starts another.
pub(crate) async fn start_provider(
    provider_info: &ResourceProviderInfo,
) -> Result<ResourceProviderClient> {
    let argv = parse_provider(&provider_info.provider)?;
    ResourceProviderClient::new(ResourceProviderConfig {
        provider_executable: argv.executable,
        provider_args: argv.args,
    })
    .await
}

pub(crate) fn parse_provider(provider_value: &Value) -> Result<ProviderStdio> {
    let provider = provider_value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Provider must be an object"))?;
    let type_ = provider
        .get("type")
        .ok_or_else(|| anyhow::anyhow!("Provider must have a type"))?;
    let type_ = type_
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Provider type must be a string"))?;
    match type_ {
        "stdio" => serde_json::from_value(provider_value.clone()).map_err(|e| e.into()),
        _ => {
            bail!("Unknown provider type: {}", type_);
        }
    }
}
