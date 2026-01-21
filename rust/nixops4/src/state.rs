use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Context as _;
use anyhow::Result;
use nixops4_core::eval_api::{ResourcePath, ResourceProviderInfo};
use nixops4_resource::schema::v0;
use nixops4_resource::schema::v0::ExtantResource;
use nixops4_resource_runner::ResourceProviderClient;
use serde::Deserialize;
use serde::Deserializer;
use tokio::sync::Mutex;

use crate::provider;

/// The root of a state file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct State {
    #[serde(flatten)]
    pub deployment: DeploymentState,

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
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct DeploymentState {
    #[serde(default)]
    pub resources: BTreeMap<String, ResourceState>,
    /// State of resources in nested deployments
    #[serde(default)]
    pub deployments: BTreeMap<String, DeploymentState>,
}

impl DeploymentState {
    pub fn get_resource(&self, path: &ResourcePath) -> Option<&ResourceState> {
        self.resources.get(&path.resource_name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct ResourceState {
    /// The type of the resource
    #[serde(rename = "type")]
    pub type_: String,
    /// The input properties of the resource
    pub input_properties: serde_json::Map<String, serde_json::Value>,
    /// The output properties of the resource
    pub output_properties: serde_json::Map<String, serde_json::Value>,
}

pub struct StateHandle {
    pub past: State,
    pub state_provider_resource: v0::ExtantResource,
    pub state_provider: Arc<Mutex<ResourceProviderClient>>,
}

impl StateHandle {
    pub async fn open(
        provider_info: &ResourceProviderInfo,
        resource: &ExtantResource,
    ) -> Result<Arc<StateHandle>> {
        let provider_argv = provider::parse_provider(&provider_info.provider)?;
        // Run the provider
        let mut provider =
            ResourceProviderClient::new(nixops4_resource_runner::ResourceProviderConfig {
                provider_executable: provider_argv.executable,
                provider_args: provider_argv.args,
            })
            .await?;

        let state = provider.state_read(resource.clone()).await?;
        // Awkwardly construct a JSON object from the BTreeMap
        let state = serde_json::to_value(&state)
            .map_err(|e| anyhow::anyhow!("Failed to serialize state: {}", e))?;
        let state = serde_json::from_value::<State>(state)
            .map_err(|e| anyhow::anyhow!("Failed to parse state: {}", e))?;

        Ok(Arc::new(StateHandle {
            past: state,
            state_provider: Arc::new(Mutex::new(provider)),
            state_provider_resource: resource.clone(),
        }))
    }

    pub async fn resource_event(
        &self,
        resource_name: &ResourcePath,
        event: &str,
        past_resource: Option<&ResourceState>,
        current_resource: &ResourceState,
    ) -> Result<()> {
        // Patch does not have a prefixing operation, so we reconstruct the relevant part of the state file for this
        let current_json = serde_json::to_value(current_resource).with_context(|| {
            format!(
                "Failed to serialize current resource state for '{}'",
                resource_name.resource_name
            )
        })?;
        let current_json = serde_json::json!({
            "resources": {
                &resource_name.resource_name: current_json
            }
        });
        let past_json = match past_resource {
            None => serde_json::json!({
                "resources": {}
            }),
            Some(past) => {
                let past_json = serde_json::to_value(past).with_context(|| {
                    format!(
                        "Failed to serialize past resource state for '{}'",
                        resource_name.resource_name
                    )
                })?;
                serde_json::json!({
                    "resources": {
                        &resource_name.resource_name: past_json
                    }
                })
            }
        };

        let patch = json_patch::diff(&past_json, &current_json);
        // If there are no changes, don't send an event
        if patch.0.is_empty() {
            return Ok(());
        }

        let patch_count = patch.0.len();

        let event = v0::StateResourceEvent {
            resource: self.state_provider_resource.clone(),
            event: event.to_string(),
            nixops_version: "0.1.0".to_string(),
            patch,
        };

        self.state_provider
            .lock()
            .await
            .state_event(event)
            .await
            .with_context(|| {
                // TODO: In the future, we could log specific attribute paths that changed (without values)
                // to help with debugging while maintaining security. E.g. "resource.input_properties.password changed"
                format!(
                    "Failed to update state for resource '{}' (type: {}) - {} field(s) changed",
                    resource_name, current_resource.type_, patch_count
                )
            })?;

        Ok(())
    }

    pub async fn close(self: Arc<Self>) -> Result<()> {
        // Only close if this is the last reference
        if let Ok(handle) = Arc::try_unwrap(self) {
            let mut provider = handle.state_provider.lock().await;
            provider.close_wait().await?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for StateHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "State {{ ... }}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deployment_state_serde_roundtrip() {
        let mut resources = BTreeMap::new();
        resources.insert(
            "myresource".to_string(),
            ResourceState {
                type_: "file".to_string(),
                input_properties: serde_json::Map::from_iter([(
                    "name".to_string(),
                    serde_json::json!("test.txt"),
                )]),
                output_properties: serde_json::Map::new(),
            },
        );

        let mut nested_resources = BTreeMap::new();
        nested_resources.insert(
            "nested_res".to_string(),
            ResourceState {
                type_: "memo".to_string(),
                input_properties: serde_json::Map::new(),
                output_properties: serde_json::Map::from_iter([(
                    "value".to_string(),
                    serde_json::json!("hello"),
                )]),
            },
        );

        let mut deployments = BTreeMap::new();
        deployments.insert(
            "child".to_string(),
            DeploymentState {
                resources: nested_resources,
                deployments: BTreeMap::new(),
            },
        );

        let state = DeploymentState {
            resources,
            deployments,
        };

        // Verify serialization produces expected JSON format
        let json = serde_json::to_value(&state).unwrap();
        let expected = serde_json::json!({
            "resources": {
                "myresource": {
                    "type": "file",
                    "input_properties": {"name": "test.txt"},
                    "output_properties": {}
                }
            },
            "deployments": {
                "child": {
                    "resources": {
                        "nested_res": {
                            "type": "memo",
                            "input_properties": {},
                            "output_properties": {"value": "hello"}
                        }
                    },
                    "deployments": {}
                }
            }
        });
        assert_eq!(json, expected);

        // Verify deserialization roundtrip
        let roundtripped: DeploymentState = serde_json::from_value(json).unwrap();
        assert_eq!(state, roundtripped);
    }

    #[test]
    fn test_deployment_state_empty_defaults() {
        let json = serde_json::json!({});
        let state: DeploymentState = serde_json::from_value(json).unwrap();
        assert!(state.resources.is_empty());
        assert!(state.deployments.is_empty());
    }
}
