use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{bail, Context as _, Result};
use nixops4_core::eval_api::{ComponentPath, ResourceProviderInfo};
use nixops4_resource::schema::v0;
use nixops4_resource::schema::v0::ExtantResource;
use nixops4_resource_runner::ResourceProviderClient;
use serde::Deserialize;
use serde::Deserializer;
use tokio::sync::Mutex;

use crate::provider;

/// The root of a state file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
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
    /// Get a resource by its path. The path must be relative to this deployment state.
    pub fn get_resource(&self, path: &ComponentPath) -> Option<&ResourceState> {
        // Navigate through nested deployment states to find the resource
        match path.0.as_slice() {
            [] => None, // Empty path is not a valid resource path
            [resource_name] => self.resources.get(resource_name),
            [first, rest @ ..] => {
                // Navigate into nested deployment
                self.deployments
                    .get(first)
                    .and_then(|nested| nested.get_resource(&ComponentPath(rest.to_vec())))
            }
        }
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
    pub current: Mutex<State>,
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
            current: Mutex::new(state),
            state_provider: Arc::new(Mutex::new(provider)),
            state_provider_resource: resource.clone(),
        }))
    }

    pub async fn resource_event(
        &self,
        resource_path: &ComponentPath,
        event: &str,
        _past_resource: Option<&ResourceState>,
        current_resource: &ResourceState,
    ) -> Result<()> {
        // Use whole-state diff approach for reliability
        let mut current_state = self.current.lock().await;

        // Convert current deployment state to JSON
        let old_state = serde_json::to_value(&current_state.deployment)
            .with_context(|| "Failed to serialize current deployment state")?;

        // Create new state with the resource updated
        let mut new_state = old_state.clone();
        update_resource_in_deployment_state(&mut new_state, resource_path, current_resource)?;

        // TODO: this is expensive for large states, O(n^2)
        let patch = json_patch::diff(&old_state, &new_state);
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
                    resource_path, current_resource.type_, patch_count
                )
            })?;

        // Update current state after successful patch application
        let new_deployment_state: DeploymentState = serde_json::from_value(new_state)
            .with_context(|| "Failed to deserialize updated deployment state")?;
        current_state.deployment = new_deployment_state;

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

/// Update a resource in a complete deployment state JSON structure.
/// This modifies the deployment state JSON in-place to set the resource at the given path.
fn update_resource_in_deployment_state(
    deployment_state: &mut serde_json::Value,
    resource_path: &ComponentPath,
    resource_state: &ResourceState,
) -> Result<()> {
    let resource_json = serde_json::to_value(resource_state)
        .with_context(|| "Failed to serialize resource state")?;

    // The path is from root: all but last element are composite names, last is resource name
    let path_parts = &resource_path.0;
    if path_parts.is_empty() {
        bail!("Empty resource path");
    }

    // Navigate to the correct deployment
    let mut current = deployment_state;
    for deployment_name in &path_parts[..path_parts.len() - 1] {
        // Ensure deployments object exists
        if current.get("deployments").is_none() {
            current["deployments"] = serde_json::json!({});
        }
        current = &mut current["deployments"];

        // Ensure this specific deployment exists with proper structure
        if current.get(deployment_name).is_none() {
            current[deployment_name] = serde_json::json!({
                "resources": {},
                "deployments": {}
            });
        }
        current = &mut current[deployment_name];
    }

    // Ensure resources object exists at final level
    if current.get("resources").is_none() {
        current["resources"] = serde_json::json!({});
    }

    // Set the resource (last element of path is the resource name)
    let resource_name = &path_parts[path_parts.len() - 1];
    current["resources"][resource_name] = resource_json;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_resource_in_state_root() {
        let mut state = serde_json::json!({
            "resources": {}
        });

        // ComponentPath for a resource at root: just the resource name
        let resource_path = ComponentPath(vec!["myresource".to_string()]);

        let resource_state = ResourceState {
            type_: "test".to_string(),
            input_properties: serde_json::Map::new(),
            output_properties: serde_json::Map::new(),
        };

        update_resource_in_deployment_state(&mut state, &resource_path, &resource_state).unwrap();

        assert_eq!(state["resources"]["myresource"]["type"], "test");
    }

    #[test]
    fn test_update_resource_in_state_nested() {
        let mut state = serde_json::json!({
            "resources": {}
        });

        // ComponentPath: deployment name, then resource name
        let resource_path = ComponentPath(vec!["deploy1".to_string(), "myresource".to_string()]);

        let resource_state = ResourceState {
            type_: "test".to_string(),
            input_properties: serde_json::Map::new(),
            output_properties: serde_json::Map::new(),
        };

        update_resource_in_deployment_state(&mut state, &resource_path, &resource_state).unwrap();

        let expected = serde_json::json!({
            "resources": {},
            "deployments": {
                "deploy1": {
                    "resources": {
                        "myresource": {
                            "type": "test",
                            "input_properties": {},
                            "output_properties": {}
                        }
                    },
                    "deployments": {}
                }
            }
        });
        assert_eq!(state, expected);
    }

    #[test]
    fn test_update_resource_with_existing_siblings() {
        let mut state = serde_json::json!({
            "resources": {
                "existing_root": {"type": "existing"}
            },
            "deployments": {
                "deploy1": {
                    "resources": {
                        "existing_nested": {"type": "existing"}
                    }
                },
                "deploy2": {
                    "resources": {
                        "sibling_resource": {"type": "sibling"}
                    }
                }
            }
        });

        let resource_path = ComponentPath(vec!["deploy1".to_string(), "new_resource".to_string()]);

        let resource_state = ResourceState {
            type_: "new".to_string(),
            input_properties: serde_json::Map::new(),
            output_properties: serde_json::Map::new(),
        };

        update_resource_in_deployment_state(&mut state, &resource_path, &resource_state).unwrap();

        let expected = serde_json::json!({
            "resources": {
                "existing_root": {"type": "existing"}
            },
            "deployments": {
                "deploy1": {
                    "resources": {
                        "existing_nested": {"type": "existing"},
                        "new_resource": {
                            "type": "new",
                            "input_properties": {},
                            "output_properties": {}
                        }
                    }
                },
                "deploy2": {
                    "resources": {
                        "sibling_resource": {"type": "sibling"}
                    }
                }
            }
        });
        assert_eq!(state, expected);
    }

    #[test]
    fn test_update_resource_deep_nested_existing_partial_path() {
        let mut state = serde_json::json!({
            "resources": {},
            "deployments": {
                "level1": {
                    "resources": {"existing": {"type": "existing"}},
                    "deployments": {
                        "level2": {
                            "resources": {"deep_existing": {"type": "deep"}}
                        }
                    }
                }
            }
        });

        let resource_path = ComponentPath(vec![
            "level1".to_string(),
            "level2".to_string(),
            "level3".to_string(),
            "deep_new".to_string(),
        ]);

        let resource_state = ResourceState {
            type_: "deep_new".to_string(),
            input_properties: serde_json::Map::new(),
            output_properties: serde_json::Map::new(),
        };

        update_resource_in_deployment_state(&mut state, &resource_path, &resource_state).unwrap();

        let expected = serde_json::json!({
            "resources": {},
            "deployments": {
                "level1": {
                    "resources": {"existing": {"type": "existing"}},
                    "deployments": {
                        "level2": {
                            "resources": {"deep_existing": {"type": "deep"}},
                            "deployments": {
                                "level3": {
                                    "resources": {
                                        "deep_new": {
                                            "type": "deep_new",
                                            "input_properties": {},
                                            "output_properties": {}
                                        }
                                    },
                                    "deployments": {}
                                }
                            }
                        }
                    }
                }
            }
        });
        assert_eq!(state, expected);
    }

    #[test]
    fn test_update_resource_completely_new_path() {
        let mut state = serde_json::json!({
            "resources": {"root_resource": {"type": "root"}}
        });

        let resource_path = ComponentPath(vec![
            "new_branch".to_string(),
            "new_subbranch".to_string(),
            "new_resource".to_string(),
        ]);

        let resource_state = ResourceState {
            type_: "brand_new".to_string(),
            input_properties: serde_json::Map::new(),
            output_properties: serde_json::Map::new(),
        };

        update_resource_in_deployment_state(&mut state, &resource_path, &resource_state).unwrap();

        let expected = serde_json::json!({
            "resources": {"root_resource": {"type": "root"}},
            "deployments": {
                "new_branch": {
                    "resources": {},
                    "deployments": {
                        "new_subbranch": {
                            "resources": {
                                "new_resource": {
                                    "type": "brand_new",
                                    "input_properties": {},
                                    "output_properties": {}
                                }
                            },
                            "deployments": {}
                        }
                    }
                }
            }
        });
        assert_eq!(state, expected);
    }

    #[test]
    fn test_update_resource_overwrites_existing() {
        let mut state = serde_json::json!({
            "resources": {},
            "deployments": {
                "deploy1": {
                    "resources": {
                        "target_resource": {"type": "old_type", "data": "old"}
                    }
                }
            }
        });

        let resource_path =
            ComponentPath(vec!["deploy1".to_string(), "target_resource".to_string()]);

        let resource_state = ResourceState {
            type_: "new_type".to_string(),
            input_properties: {
                let mut map = serde_json::Map::new();
                map.insert("new_input".to_string(), serde_json::json!("new_value"));
                map
            },
            output_properties: serde_json::Map::new(),
        };

        update_resource_in_deployment_state(&mut state, &resource_path, &resource_state).unwrap();

        let expected = serde_json::json!({
            "resources": {},
            "deployments": {
                "deploy1": {
                    "resources": {
                        "target_resource": {
                            "type": "new_type",
                            "input_properties": {
                                "new_input": "new_value"
                            },
                            "output_properties": {}
                        }
                    }
                }
            }
        });
        assert_eq!(state, expected);
    }
}
