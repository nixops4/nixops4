/// This module supplements the `nixops4-resource-runner` library with
/// evaluation-layer logic.
use anyhow::{bail, Result};
use serde_json::Value;

/// This type implements the parsing of `type: "stdio"` providers.
#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
pub(crate) struct ProviderStdio {
    pub(crate) executable: String,
    pub(crate) args: Vec<String>,
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
