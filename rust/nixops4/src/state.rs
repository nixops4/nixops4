use std::collections::BTreeMap;

use serde::Deserialize;
use serde::Deserializer;

/// The root of a state file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct State {
    #[serde(flatten)]
    deployment: DeploymentState,

    #[serde(deserialize_with = "type_is_nixops_state")]
    _type: String,
}

fn type_is_nixops_state<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    if s == "nixopsState" {
        Ok(s)
    } else {
        Err(serde::de::Error::custom(format!(
            "unexpected _type in nixops state: expected 'nixopsState', got '{}'",
            s
        )))
    }
}

/// The state of a set of resources
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct DeploymentState {
    resources: BTreeMap<String, ResourceState>,
    /// State of resources in nested deployments
    deployments: BTreeMap<String, DeploymentState>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct ResourceState {
    /// The type of the resource
    #[serde(rename = "type")]
    type_: String,
    /// The properties of the resource
    properties: serde_json::Value,
}
