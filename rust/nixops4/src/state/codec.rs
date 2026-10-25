use anyhow::{bail, Context as _, Result};
use serde::{Deserialize, Deserializer, Serialize};

use super::{DeploymentState, ResourceState, State};
use nixops4_core::eval_api::ComponentPath;

/// The persisted state document. The `_type` marker identifies the format.
#[derive(Debug, Deserialize, Serialize)]
struct StateRecord {
    #[serde(flatten)]
    deployment: DeploymentStateRecord,

    #[serde(deserialize_with = "type_is_nixops_state")]
    _type: String,
}

fn type_is_nixops_state<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    // i hate doing manual lifetimes but no way around it for now
    D: Deserializer<'de>,
{
    let type_: String = Deserialize::deserialize(deserializer)?;
    if type_ == "nixopsState" {
        Ok(type_)
    } else {
        Err(serde::de::Error::custom(format!(
            "unexpected _type in nixops state: expected 'nixopsState', got '{}'",
            type_
        )))
    }
}

/// The persisted state of a set of resources
#[derive(Debug, Deserialize, Serialize)]
struct DeploymentStateRecord {
    #[serde(default)]
    resources: std::collections::BTreeMap<String, ResourceStateRecord>,
    /// State of resources in nested deployments
    #[serde(default)]
    deployments: std::collections::BTreeMap<String, DeploymentStateRecord>,
}

/// The persisted state of one resource
#[derive(Debug, Deserialize, Serialize)]
struct ResourceStateRecord {
    /// The type of the resource
    #[serde(rename = "type")]
    type_: String,
    /// The input properties of the resource
    input_properties: serde_json::Map<String, serde_json::Value>,
    /// The output properties of the resource
    output_properties: serde_json::Map<String, serde_json::Value>,
}

/// Decode a persisted state document.
pub(super) fn decode_state(value: serde_json::Value) -> Result<State> {
    let record: StateRecord = serde_json::from_value(value)
        .map_err(|e| anyhow::anyhow!("Failed to parse state: {}", e))?;
    Ok(State {
        deployment: record.deployment.into(),
    })
}

/// Decode the persisted form of a deployment.
pub(super) fn decode_deployment(value: serde_json::Value) -> Result<DeploymentState> {
    let record: DeploymentStateRecord = serde_json::from_value(value)
        .with_context(|| "Failed to deserialize updated deployment state")?;
    Ok(record.into())
}

/// Encode a state document, including its `_type` marker.
pub(super) fn encode_state(state: &State) -> Result<serde_json::Value> {
    let record = StateRecord {
        deployment: DeploymentStateRecord::from(&state.deployment),
        _type: "nixopsState".to_string(),
    };
    serde_json::to_value(&record).with_context(|| "Failed to serialize state")
}

/// Encode a deployment as its persisted form.
pub(super) fn encode_deployment(deployment: &DeploymentState) -> Result<serde_json::Value> {
    serde_json::to_value(DeploymentStateRecord::from(deployment))
        .with_context(|| "Failed to serialize deployment state")
}

/// Encode one resource as its persisted form.
pub(super) fn encode_resource(resource: &ResourceState) -> Result<serde_json::Value> {
    serde_json::to_value(ResourceStateRecord::from(resource))
        .with_context(|| "Failed to serialize resource state")
}

/// Set a resource in a persisted deployment state, in place, at the given
/// path. All but the last path element are nested deployment names; the last
/// is the resource name.
pub(super) fn set_resource_in_deployment_state(
    deployment_state: &mut serde_json::Value,
    resource_path: &ComponentPath,
    resource_state: &ResourceState,
) -> Result<()> {
    let path_parts = &resource_path.0;
    if path_parts.is_empty() {
        bail!("Empty resource path");
    }
    let (deployment_names, resource_name) = path_parts.split_at(path_parts.len() - 1);
    let resource_name = &resource_name[0];

    let deployment = deployment_at_path(deployment_state, deployment_names);
    if deployment.get("resources").is_none() {
        deployment["resources"] = serde_json::json!({});
    }
    deployment["resources"][resource_name] = encode_resource(resource_state)?;

    Ok(())
}

/// The deployment at `path` in a persisted deployment state, creating the
/// nested deployments along the way.
fn deployment_at_path<'a>(
    deployment_state: &'a mut serde_json::Value,
    path: &[String],
) -> &'a mut serde_json::Value {
    let mut current = deployment_state;
    for deployment_name in path {
        if current.get("deployments").is_none() {
            current["deployments"] = serde_json::json!({});
        }
        current = &mut current["deployments"];
        if current.get(deployment_name).is_none() {
            current[deployment_name] = serde_json::json!({
                "resources": {},
                "deployments": {}
            });
        }
        current = &mut current[deployment_name];
    }
    current
}

impl From<&DeploymentState> for DeploymentStateRecord {
    fn from(model: &DeploymentState) -> Self {
        DeploymentStateRecord {
            resources: model
                .resources
                .iter()
                .map(|(k, v)| (k.clone(), ResourceStateRecord::from(v)))
                .collect(),
            deployments: model
                .deployments
                .iter()
                .map(|(k, v)| (k.clone(), DeploymentStateRecord::from(v)))
                .collect(),
        }
    }
}

impl From<DeploymentStateRecord> for DeploymentState {
    fn from(record: DeploymentStateRecord) -> Self {
        DeploymentState {
            resources: record
                .resources
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
            deployments: record
                .deployments
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
        }
    }
}

impl From<&ResourceState> for ResourceStateRecord {
    fn from(model: &ResourceState) -> Self {
        ResourceStateRecord {
            type_: model.type_.clone(),
            input_properties: model.input_properties.clone(),
            output_properties: model.output_properties.clone(),
        }
    }
}

impl From<ResourceStateRecord> for ResourceState {
    fn from(record: ResourceStateRecord) -> Self {
        ResourceState {
            type_: record.type_,
            input_properties: record.input_properties,
            output_properties: record.output_properties,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_rejects_wrong_type_marker() {
        let e = decode_state(serde_json::json!({
            "resources": {},
            "_type": "somethingElse"
        }))
        .unwrap_err();
        assert!(e.to_string().contains("unexpected _type"));
    }

    #[test]
    fn test_decode_requires_type_marker() {
        let e = decode_state(serde_json::json!({ "resources": {} })).unwrap_err();
        assert!(e.to_string().contains("Failed to parse state"));
    }

    #[test]
    fn test_state_round_trip() {
        let model = State {
            deployment: DeploymentState {
                resources: [(
                    "myresource".to_string(),
                    ResourceState {
                        type_: "test".to_string(),
                        input_properties: serde_json::Map::from_iter([(
                            "path".to_string(),
                            serde_json::json!("pubkey.txt"),
                        )]),
                        output_properties: serde_json::Map::new(),
                    },
                )]
                .into_iter()
                .collect(),
                deployments: [(
                    "nested".to_string(),
                    DeploymentState {
                        resources: Default::default(),
                        deployments: Default::default(),
                    },
                )]
                .into_iter()
                .collect(),
            },
        };

        let document = encode_state(&model).unwrap();
        assert_eq!(
            document,
            serde_json::json!({
                "resources": {
                    "myresource": {
                        "type": "test",
                        "input_properties": {"path": "pubkey.txt"},
                        "output_properties": {}
                    }
                },
                "deployments": {
                    "nested": {"resources": {}, "deployments": {}}
                },
                "_type": "nixopsState"
            })
        );

        let decoded = decode_state(document).unwrap();
        assert_eq!(decoded.deployment, model.deployment);
    }

    #[test]
    fn test_encode_empty_deployment() {
        let deployment = DeploymentState {
            resources: Default::default(),
            deployments: Default::default(),
        };
        let encoded = encode_deployment(&deployment).unwrap();
        assert_eq!(
            encoded,
            serde_json::json!({"resources": {}, "deployments": {}})
        );
    }
}
