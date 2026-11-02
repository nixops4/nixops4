mod codec;

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use nixops4_core::eval_api::{ComponentPath, ResourceProviderInfo};
use nixops4_resource_runner::{
    ProviderResource, ResourceProviderClient, StateEvent as ClientStateEvent,
};
use tokio::sync::Mutex;

use crate::provider;

/// The root of a state file, in memory
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub deployment: DeploymentState,
}

/// The state of a set of resources
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentState {
    pub resources: BTreeMap<String, ResourceState>,
    /// State of resources in nested deployments
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceState {
    /// The type of the resource
    pub type_: String,
    /// The input properties of the resource
    pub input_properties: serde_json::Map<String, serde_json::Value>,
    /// The output properties of the resource
    pub output_properties: serde_json::Map<String, serde_json::Value>,
}

impl From<&ResourceState> for crate::resource::ExtantResource {
    fn from(resource: &ResourceState) -> Self {
        crate::resource::ExtantResource {
            type_: resource.type_.clone(),
            input_properties: resource.input_properties.clone(),
            output_properties: Some(resource.output_properties.clone()),
        }
    }
}

pub struct StateHandle {
    pub current: Mutex<State>,
    state_store: ProviderResource,
    state_provider: Arc<Mutex<ResourceProviderClient>>,
}

impl StateHandle {
    pub async fn open(
        provider_info: &ResourceProviderInfo,
        resource: &crate::resource::ExtantResource,
    ) -> Result<Arc<StateHandle>> {
        let mut provider = provider::spawn_provider(&provider_info.provider).await?;
        let state_store = ProviderResource::from(resource);
        let state = Self::read_state(&mut provider, &state_store).await?;
        Ok(Arc::new(StateHandle {
            current: Mutex::new(state),
            state_provider: Arc::new(Mutex::new(provider)),
            state_store,
        }))
    }

    /// The current state as a persisted state document
    pub async fn state_json(&self) -> Result<String> {
        let state = self.current.lock().await;
        let document = codec::encode_state(&state)?;
        serde_json::to_string_pretty(&document).context("serializing state document")
    }

    async fn read_state(
        provider: &mut ResourceProviderClient,
        state_store: &ProviderResource,
    ) -> Result<State> {
        let document = provider.state_read(state_store).await?;
        codec::decode_state(serde_json::Value::Object(document))
    }

    pub async fn resource_event(
        &self,
        resource_path: &ComponentPath,
        event: &str,
        _past_resource: Option<&ResourceState>,
        current_resource: &ResourceState,
    ) -> Result<()> {
        let mut current_state = self.current.lock().await;

        let Some((patch, new_document)) =
            state_event_patch(&current_state.deployment, resource_path, current_resource)?
        else {
            return Ok(());
        };
        let patch_count = patch.0.len();

        self.state_provider
            .lock()
            .await
            .state_event(
                &self.state_store,
                ClientStateEvent {
                    event: event.to_string(),
                    nixops_version: env!("CARGO_PKG_VERSION").to_string(),
                    patch,
                },
            )
            .await
            .with_context(|| {
                // TODO: In the future, we could log specific attribute paths that changed (without values)
                // to help with debugging while maintaining security. E.g. "resource.input_properties.password changed"
                format!(
                    "Failed to update state for resource '{}' (type: {}) - {} field(s) changed",
                    resource_path, current_resource.type_, patch_count
                )
            })?;

        current_state.deployment = codec::decode_deployment(new_document)?;

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

/// The patch that turns the deployment state into the one with `resource`
/// set at `resource_path`, together with that new state document.
/// `None` when the patch is empty.
fn state_event_patch(
    deployment: &DeploymentState,
    resource_path: &ComponentPath,
    resource: &ResourceState,
) -> Result<Option<(json_patch::Patch, serde_json::Value)>> {
    let old_document = codec::encode_deployment(deployment)?;
    let mut new_document = old_document.clone();
    codec::set_resource_in_deployment_state(&mut new_document, resource_path, resource)?;
    // TODO: this is expensive for large states, O(n^2)
    let patch = json_patch::diff(&old_document, &new_document);
    if patch.0.is_empty() {
        Ok(None)
    } else {
        Ok(Some((patch, new_document)))
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

    fn resource(type_: &str) -> ResourceState {
        ResourceState {
            type_: type_.to_string(),
            input_properties: serde_json::Map::new(),
            output_properties: serde_json::Map::new(),
        }
    }

    fn set_resource(state: &mut serde_json::Value, path: &[&str], type_: &str) -> Result<()> {
        codec::set_resource_in_deployment_state(
            state,
            &ComponentPath(path.iter().map(|s| s.to_string()).collect()),
            &resource(type_),
        )
    }

    #[test]
    fn test_update_resource_in_state_root() {
        let mut state = serde_json::json!({
            "resources": {}
        });

        set_resource(&mut state, &["myresource"], "test").unwrap();

        assert_eq!(state["resources"]["myresource"]["type"], "test");
    }

    #[test]
    fn test_update_resource_in_state_nested() {
        let mut state = serde_json::json!({
            "resources": {}
        });

        set_resource(&mut state, &["deploy1", "myresource"], "test").unwrap();

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

        set_resource(&mut state, &["deploy1", "new_resource"], "new").unwrap();

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

        set_resource(
            &mut state,
            &["level1", "level2", "level3", "deep_new"],
            "deep_new",
        )
        .unwrap();

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

        set_resource(
            &mut state,
            &["new_branch", "new_subbranch", "new_resource"],
            "brand_new",
        )
        .unwrap();

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

        let new_resource = ResourceState {
            type_: "new_type".to_string(),
            input_properties: {
                let mut map = serde_json::Map::new();
                map.insert("new_input".to_string(), serde_json::json!("new_value"));
                map
            },
            output_properties: serde_json::Map::new(),
        };
        codec::set_resource_in_deployment_state(
            &mut state,
            &ComponentPath(vec!["deploy1".to_string(), "target_resource".to_string()]),
            &new_resource,
        )
        .unwrap();

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

    #[test]
    fn test_state_event_patch_empty() {
        let deployment = DeploymentState {
            resources: [("myresource".to_string(), resource("test"))]
                .into_iter()
                .collect(),
            deployments: BTreeMap::new(),
        };
        let path = ComponentPath(vec!["myresource".to_string()]);

        let patch = state_event_patch(&deployment, &path, &resource("test")).unwrap();
        assert!(patch.is_none());
    }

    #[test]
    fn test_state_event_patch_changed() {
        let deployment = DeploymentState {
            resources: [("myresource".to_string(), resource("old"))]
                .into_iter()
                .collect(),
            deployments: BTreeMap::new(),
        };
        let path = ComponentPath(vec!["myresource".to_string()]);

        let (patch, new_document) = state_event_patch(&deployment, &path, &resource("new"))
            .unwrap()
            .unwrap();
        assert_eq!(patch.0.len(), 1);
        assert_eq!(new_document["resources"]["myresource"]["type"], "new");
    }
}
