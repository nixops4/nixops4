use anyhow::{bail, Context, Result};
use hyper_util::rt::TokioIo;
use std::process::Stdio;
use tokio::net::UnixStream;
use tokio::process::{Child, Command as TokioCommand};
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use super::grpc::{ProviderServiceClientV5, ProviderServiceClientV6};

/// Wrapper for different protocol version clients
pub enum ClientConnection {
    V5(ProviderServiceClientV5<Channel>),
    V6(ProviderServiceClientV6<Channel>),
}

impl ClientConnection {
    /// Get provider schema using the appropriate protocol version
    pub async fn get_provider_schema(&mut self) -> Result<ProviderSchema> {
        match self {
            ClientConnection::V5(client) => {
                let request =
                    tonic::Request::new(super::grpc::tfplugin5_9::get_provider_schema::Request {});
                let response = client
                    .get_schema(request)
                    .await
                    .context("Failed to call GetSchema (v5)")?;
                let schema = response.into_inner();
                Ok(ProviderSchema::V5(schema))
            }
            ClientConnection::V6(client) => {
                let request =
                    tonic::Request::new(super::grpc::tfplugin6_9::get_provider_schema::Request {});
                let response = client
                    .get_provider_schema(request)
                    .await
                    .context("Failed to call GetProviderSchema (v6)")?;
                let schema = response.into_inner();
                Ok(ProviderSchema::V6(schema))
            }
        }
    }

    /// Configure the provider with configuration values
    pub async fn configure_provider(
        &mut self,
        config: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<()> {
        // Get provider schema to complete configuration
        let raw_schema = self.get_provider_schema().await?;
        let schema = crate::schema::ProviderSchema::from_raw_response(raw_schema);

        // Complete provider configuration with missing optional attributes
        let complete_config = if let Some(provider_schema) = &schema.provider {
            if let Some(block) = &provider_schema.block {
                Self::complete_resource_state(config, block)?
            } else {
                config
            }
        } else {
            config
        };
        match self {
            ClientConnection::V5(client) => {
                let request = tonic::Request::new(super::grpc::tfplugin5_9::configure::Request {
                    terraform_version: "1.0.0".to_string(), // TODO: get from somewhere
                    config: Some(Self::json_map_to_dynamic_value_v5(complete_config)?),
                    client_capabilities: None, // Not required for basic operation
                });
                let response = client
                    .configure(request)
                    .await
                    .context("Failed to call Configure (v5)")?;

                // Check for diagnostics in response
                let response = response.into_inner();
                for diagnostic in &response.diagnostics {
                    if diagnostic.severity == 1 {
                        // ERROR severity
                        bail!("Provider configuration error: {}", diagnostic.summary);
                    }
                }
                Ok(())
            }
            ClientConnection::V6(client) => {
                let request =
                    tonic::Request::new(super::grpc::tfplugin6_9::configure_provider::Request {
                        terraform_version: "1.0.0".to_string(), // TODO: get from somewhere
                        config: Some(Self::json_map_to_dynamic_value_v6(complete_config)?),
                        client_capabilities: None, // Not required for basic operation
                    });
                let response = client
                    .configure_provider(request)
                    .await
                    .context("Failed to call ConfigureProvider (v6)")?;

                // Check for diagnostics in response
                let response = response.into_inner();
                for diagnostic in &response.diagnostics {
                    if diagnostic.severity == 1 {
                        // ERROR severity
                        bail!("Provider configuration error: {}", diagnostic.summary);
                    }
                }
                Ok(())
            }
        }
    }

    /// Create a null DynamicValue for resource creation (v5)
    fn null_dynamic_value_v5() -> Result<super::grpc::tfplugin5_9::DynamicValue> {
        let null_value = serde_json::Value::Null;
        let json_bytes =
            serde_json::to_vec(&null_value).context("Failed to serialize null to JSON")?;
        let msgpack_bytes =
            rmp_serde::to_vec(&null_value).context("Failed to serialize null to msgpack")?;
        Ok(super::grpc::tfplugin5_9::DynamicValue {
            msgpack: msgpack_bytes,
            json: json_bytes,
        })
    }

    /// Convert a JSON map to Terraform's DynamicValue format (v5)
    fn json_map_to_dynamic_value_v5(
        config: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<super::grpc::tfplugin5_9::DynamicValue> {
        let json_object = serde_json::Value::Object(
            config
                .into_iter()
                .collect::<serde_json::Map<String, serde_json::Value>>(),
        );
        let json_bytes =
            serde_json::to_vec(&json_object).context("Failed to serialize config to JSON")?;
        let msgpack_bytes =
            rmp_serde::to_vec(&json_object).context("Failed to serialize config to msgpack")?;
        // eprintln!(
        //     "DEBUG: Sending JSON to terraform provider v5: {}",
        //     String::from_utf8_lossy(&json_bytes)
        // );
        // eprintln!(
        //     "DEBUG: Sending msgpack to terraform provider v5: {} bytes",
        //     msgpack_bytes.len()
        // );

        Ok(super::grpc::tfplugin5_9::DynamicValue {
            msgpack: msgpack_bytes,
            json: json_bytes,
        })
    }

    /// Create a null DynamicValue for resource creation (v6)
    fn null_dynamic_value_v6() -> Result<super::grpc::tfplugin6_9::DynamicValue> {
        let null_value = serde_json::Value::Null;
        let json_bytes =
            serde_json::to_vec(&null_value).context("Failed to serialize null to JSON")?;
        let msgpack_bytes =
            rmp_serde::to_vec(&null_value).context("Failed to serialize null to msgpack")?;
        Ok(super::grpc::tfplugin6_9::DynamicValue {
            msgpack: msgpack_bytes,
            json: json_bytes,
        })
    }

    /// Convert a JSON map to Terraform's DynamicValue format (v6)
    fn json_map_to_dynamic_value_v6(
        config: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<super::grpc::tfplugin6_9::DynamicValue> {
        let json_object = serde_json::Value::Object(
            config
                .into_iter()
                .collect::<serde_json::Map<String, serde_json::Value>>(),
        );
        let json_bytes =
            serde_json::to_vec(&json_object).context("Failed to serialize config to JSON")?;
        let msgpack_bytes =
            rmp_serde::to_vec(&json_object).context("Failed to serialize config to msgpack")?;

        Ok(super::grpc::tfplugin6_9::DynamicValue {
            msgpack: msgpack_bytes,
            json: json_bytes,
        })
    }

    /// Read resource state
    pub async fn read_resource(
        &mut self,
        resource_type: &str,
        current_state: Option<std::collections::HashMap<String, serde_json::Value>>,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>> {
        // Get resource schema to fill in missing optional attributes with null values
        let raw_schema = self.get_provider_schema().await?;
        let schema = crate::schema::ProviderSchema::from_raw_response(raw_schema);

        // Helper function to get schema and terraform type name for resources vs data sources
        let is_data_source = resource_type.starts_with("data-source-");

        let (resource_schema, terraform_type_name) = if is_data_source {
            let data_source_name = resource_type.strip_prefix("data-source-").unwrap();
            let schema = schema
                .data_source_schemas
                .get(data_source_name)
                .context(format!(
                    "Data source type '{}' not found in provider schema",
                    data_source_name
                ))?;
            (schema, data_source_name)
        } else {
            let schema = schema.resource_schemas.get(resource_type).context(format!(
                "Resource type '{}' not found in provider schema",
                resource_type
            ))?;
            (schema, resource_type)
        };

        let resource_block = resource_schema
            .block
            .as_ref()
            .context("Resource schema missing block definition")?;

        let complete_current_state = match &current_state {
            Some(state) => Some(Self::complete_resource_state(
                state.clone(),
                resource_block,
            )?),
            None => None,
        };

        match self {
            ClientConnection::V5(client) => {
                let current_state_dv = match &complete_current_state {
                    Some(s) => Self::json_map_to_dynamic_value_v5(s.clone())?,
                    None => Self::null_dynamic_value_v5()?, // Null state for creation
                };

                let request =
                    tonic::Request::new(super::grpc::tfplugin5_9::read_resource::Request {
                        type_name: terraform_type_name.to_string(),
                        current_state: Some(current_state_dv),
                        provider_meta: None,
                        client_capabilities: None,
                        private: vec![], // TODO handle private state
                        current_identity: None,
                    });
                let response = client
                    .read_resource(request)
                    .await
                    .context("Failed to call ReadDataSource (v5)")?;

                let response = response.into_inner();

                // Check for error diagnostics
                for diagnostic in &response.diagnostics {
                    if diagnostic.severity
                        == super::grpc::tfplugin5_9::diagnostic::Severity::Error as i32
                    {
                        bail!(
                            "Terraform provider error: {} - {}",
                            diagnostic.summary,
                            diagnostic.detail
                        );
                    }
                }

                // Convert new_state DynamicValue back to JSON
                if let Some(new_state) = response.new_state {
                    Self::dynamic_value_v5_to_optional_json_map(new_state).map(|opt| {
                        opt.unwrap_or_else(|| {
                            complete_current_state
                                .clone()
                                .unwrap_or_else(|| std::collections::HashMap::new())
                        })
                    })
                } else {
                    Ok(complete_current_state
                        .clone()
                        .unwrap_or_else(|| std::collections::HashMap::new()))
                }
            }
            ClientConnection::V6(client) => {
                let current_state_dv = match &complete_current_state {
                    Some(s) => Self::json_map_to_dynamic_value_v6(s.clone())?,
                    None => Self::null_dynamic_value_v6()?, // Null state for creation
                };
                let request =
                    tonic::Request::new(super::grpc::tfplugin6_9::read_resource::Request {
                        type_name: terraform_type_name.to_string(),
                        current_state: Some(current_state_dv),
                        provider_meta: None,
                        client_capabilities: None,
                        private: vec![], // TODO handle private state
                        current_identity: None,
                    });
                let response = client
                    .read_resource(request)
                    .await
                    .context("Failed to call ReadDataSource (v6)")?;

                let response = response.into_inner();

                // Check for error diagnostics
                for diagnostic in &response.diagnostics {
                    if diagnostic.severity
                        == super::grpc::tfplugin6_9::diagnostic::Severity::Error as i32
                    {
                        bail!(
                            "Terraform provider error: {} - {}",
                            diagnostic.summary,
                            diagnostic.detail
                        );
                    }
                }

                // Convert new_state DynamicValue back to JSON
                if let Some(new_state) = response.new_state {
                    Self::dynamic_value_v6_to_optional_json_map(new_state).map(|opt| {
                        opt.unwrap_or_else(|| {
                            complete_current_state
                                .clone()
                                .unwrap_or_else(|| std::collections::HashMap::new())
                        })
                    })
                } else {
                    Ok(complete_current_state
                        .clone()
                        .unwrap_or_else(|| std::collections::HashMap::new()))
                }
            }
        }
    }

    /// Apply resource changes (create or update)
    pub async fn apply_resource_change(
        &mut self,
        resource_type: &str,
        prior_state: Option<std::collections::HashMap<String, serde_json::Value>>,
        planned_state: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>> {
        // Get resource schema to fill in missing optional attributes with null values
        let raw_schema = self.get_provider_schema().await?;
        let schema = crate::schema::ProviderSchema::from_raw_response(raw_schema);
        // Helper function to get schema and terraform type name for resources vs data sources
        let is_data_source = resource_type.starts_with("data-source-");
        let (resource_schema, terraform_type_name) = if is_data_source {
            let data_source_name = resource_type.strip_prefix("data-source-").unwrap();
            let schema = schema
                .data_source_schemas
                .get(data_source_name)
                .context(format!(
                    "Data source type '{}' not found in provider schema",
                    data_source_name
                ))?;
            (schema, data_source_name)
        } else {
            let schema = schema.resource_schemas.get(resource_type).context(format!(
                "Resource type '{}' not found in provider schema",
                resource_type
            ))?;
            (schema, resource_type)
        };
        let resource_block = resource_schema
            .block
            .as_ref()
            .context("Resource schema missing block definition")?;

        // Create complete planned state with null values for missing optional attributes
        let complete_planned_state = Self::complete_resource_state(planned_state, resource_block)?;
        // eprintln!(
        //     "DEBUG: Complete planned state: {:?}",
        //     complete_planned_state
        // );

        // Also complete the prior state if it exists
        let complete_prior_state = match &prior_state {
            Some(state) => Some(Self::complete_resource_state(
                state.clone(),
                resource_block,
            )?),
            None => None,
        };

        if is_data_source {
            // Use ReadDataSource for data sources
            match self {
                ClientConnection::V5(client) => {
                    let config_dv = Self::json_map_to_dynamic_value_v5(complete_planned_state)?;
                    let request =
                        tonic::Request::new(super::grpc::tfplugin5_9::read_data_source::Request {
                            type_name: terraform_type_name.to_string(),
                            config: Some(config_dv),
                            provider_meta: None,
                            client_capabilities: None,
                        });
                    let response = client
                        .read_data_source(request)
                        .await
                        .context("Failed to call ReadDataSource (v5)")?;

                    let response = response.into_inner();

                    // Check for error diagnostics
                    for diagnostic in &response.diagnostics {
                        if diagnostic.severity
                            == super::grpc::tfplugin5_9::diagnostic::Severity::Error as i32
                        {
                            bail!(
                                "Terraform provider error: {} - {}",
                                diagnostic.summary,
                                diagnostic.detail
                            );
                        }
                    }

                    // Convert state DynamicValue back to JSON
                    if let Some(state) = response.state {
                        Self::dynamic_value_v5_to_optional_json_map(state)
                            .map(|opt| opt.unwrap_or_else(|| std::collections::HashMap::new()))
                    } else {
                        Ok(std::collections::HashMap::new())
                    }
                }
                ClientConnection::V6(client) => {
                    let config_dv = Self::json_map_to_dynamic_value_v6(complete_planned_state)?;
                    let request =
                        tonic::Request::new(super::grpc::tfplugin6_9::read_data_source::Request {
                            type_name: terraform_type_name.to_string(),
                            config: Some(config_dv),
                            provider_meta: None,
                            client_capabilities: None,
                        });
                    let response = client
                        .read_data_source(request)
                        .await
                        .context("Failed to call ReadDataSource (v6)")?;

                    let response = response.into_inner();

                    // Check for error diagnostics
                    for diagnostic in &response.diagnostics {
                        if diagnostic.severity
                            == super::grpc::tfplugin6_9::diagnostic::Severity::Error as i32
                        {
                            bail!(
                                "Terraform provider error: {} - {}",
                                diagnostic.summary,
                                diagnostic.detail
                            );
                        }
                    }

                    // Convert state DynamicValue back to JSON
                    if let Some(state) = response.state {
                        Self::dynamic_value_v6_to_optional_json_map(state)
                            .map(|opt| opt.unwrap_or_else(|| std::collections::HashMap::new()))
                    } else {
                        Ok(std::collections::HashMap::new())
                    }
                }
            }
        } else {
            // Use ApplyResourceChange for regular resources
            match self {
                ClientConnection::V5(client) => {
                    let prior_state_dv = match &complete_prior_state {
                        Some(s) => Self::json_map_to_dynamic_value_v5(s.clone())?,
                        None => Self::null_dynamic_value_v5()?, // Null state for creation
                    };
                    let config_dv =
                        Self::json_map_to_dynamic_value_v5(complete_planned_state.clone())?;
                    let planned_state_dv =
                        Self::json_map_to_dynamic_value_v5(complete_planned_state)?;
                    let request = tonic::Request::new(
                        super::grpc::tfplugin5_9::apply_resource_change::Request {
                            type_name: terraform_type_name.to_string(),
                            prior_state: Some(prior_state_dv),
                            planned_state: Some(planned_state_dv),
                            config: Some(config_dv), // Config contains the input configuration
                            planned_private: vec![], // TODO: handle private state
                            provider_meta: None,     // Not needed for basic operation
                            planned_identity: None,  // Not needed for basic operation
                        },
                    );
                    let response = client
                        .apply_resource_change(request)
                        .await
                        .context("Failed to call ApplyResourceChange (v5)")?;

                    let response = response.into_inner();

                    // Check for error diagnostics
                    for diagnostic in &response.diagnostics {
                        if diagnostic.severity
                            == super::grpc::tfplugin5_9::diagnostic::Severity::Error as i32
                        {
                            bail!(
                                "Terraform provider error: {} - {}",
                                diagnostic.summary,
                                diagnostic.detail
                            );
                        }
                    }

                    // Convert new_state DynamicValue back to JSON
                    if let Some(new_state) = response.new_state {
                        Self::dynamic_value_v5_to_optional_json_map(new_state).map(|opt| {
                            opt.unwrap_or_else(|| {
                                complete_prior_state
                                    .clone()
                                    .unwrap_or_else(|| std::collections::HashMap::new())
                            })
                        })
                    } else {
                        Ok(complete_prior_state
                            .clone()
                            .unwrap_or_else(|| std::collections::HashMap::new()))
                    }
                }
                ClientConnection::V6(client) => {
                    let prior_state_dv = match &complete_prior_state {
                        Some(s) => Self::json_map_to_dynamic_value_v6(s.clone())?,
                        None => Self::null_dynamic_value_v6()?, // Null state for creation
                    };
                    let config_dv =
                        Self::json_map_to_dynamic_value_v6(complete_planned_state.clone())?;
                    let planned_state_dv =
                        Self::json_map_to_dynamic_value_v6(complete_planned_state)?;
                    let request = tonic::Request::new(
                        super::grpc::tfplugin6_9::apply_resource_change::Request {
                            type_name: terraform_type_name.to_string(),
                            prior_state: Some(prior_state_dv),
                            planned_state: Some(planned_state_dv),
                            config: Some(config_dv), // Config contains the input configuration
                            planned_private: vec![],
                            provider_meta: None, // Not needed for basic operation
                            planned_identity: None, // Not needed for basic operation
                        },
                    );
                    let response = client
                        .apply_resource_change(request)
                        .await
                        .context("Failed to call ApplyResourceChange (v6)")?;

                    let response = response.into_inner();

                    // Check for error diagnostics
                    for diagnostic in &response.diagnostics {
                        if diagnostic.severity
                            == super::grpc::tfplugin6_9::diagnostic::Severity::Error as i32
                        {
                            bail!(
                                "Terraform provider error: {} - {}",
                                diagnostic.summary,
                                diagnostic.detail
                            );
                        }
                    }

                    // Convert new_state DynamicValue back to JSON
                    if let Some(new_state) = response.new_state {
                        Self::dynamic_value_v6_to_optional_json_map(new_state).map(|opt| {
                            opt.unwrap_or_else(|| {
                                complete_prior_state
                                    .clone()
                                    .unwrap_or_else(|| std::collections::HashMap::new())
                            })
                        })
                    } else {
                        Ok(complete_prior_state
                            .clone()
                            .unwrap_or_else(|| std::collections::HashMap::new()))
                    }
                }
            }
        }
    }

    /// Convert DynamicValue back to JSON map, handling null responses (v5)
    fn dynamic_value_v5_to_optional_json_map(
        dynamic_value: super::grpc::tfplugin5_9::DynamicValue,
    ) -> Result<Option<std::collections::HashMap<String, serde_json::Value>>> {
        // Try JSON format first
        if !dynamic_value.json.is_empty() {
            let json_value: serde_json::Value = serde_json::from_slice(&dynamic_value.json)
                .context("Failed to parse JSON from DynamicValue")?;

            match json_value {
                serde_json::Value::Object(map) => Ok(Some(map.into_iter().collect())),
                serde_json::Value::Null => Ok(None),
                _ => bail!("Expected JSON object or null in DynamicValue"),
            }
        } else if !dynamic_value.msgpack.is_empty() {
            // Parse msgpack format
            let json_value: serde_json::Value = rmp_serde::from_slice(&dynamic_value.msgpack)
                .context("Failed to parse MessagePack from DynamicValue")?;

            match json_value {
                serde_json::Value::Object(map) => Ok(Some(map.into_iter().collect())),
                serde_json::Value::Null => Ok(None),
                _ => bail!("Expected JSON object or null from MessagePack in DynamicValue"),
            }
        } else {
            // Empty dynamic value
            Ok(None)
        }
    }

    /// Convert DynamicValue back to JSON map (v5)
    fn dynamic_value_v5_to_json_map(
        dynamic_value: super::grpc::tfplugin5_9::DynamicValue,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>> {
        // Try JSON format first
        if !dynamic_value.json.is_empty() {
            let json_value: serde_json::Value = serde_json::from_slice(&dynamic_value.json)
                .context("Failed to parse JSON from DynamicValue")?;

            if let serde_json::Value::Object(map) = json_value {
                Ok(map.into_iter().collect())
            } else {
                // eprintln!("DEBUG: JSON value is not an object: {:?}", json_value);
                bail!("Expected JSON object in DynamicValue")
            }
        } else if !dynamic_value.msgpack.is_empty() {
            // Parse msgpack format
            let json_value: serde_json::Value = rmp_serde::from_slice(&dynamic_value.msgpack)
                .context("Failed to parse MessagePack from DynamicValue")?;

            // eprintln!("DEBUG: Parsed msgpack value: {:?}", json_value);
            if let serde_json::Value::Object(map) = json_value {
                Ok(map.into_iter().collect())
            } else {
                // eprintln!(
                //     "DEBUG: MessagePack value is not an object: {:?}",
                //     json_value
                // );
                bail!("Expected JSON object from MessagePack in DynamicValue")
            }
        } else {
            // Empty dynamic value
            Ok(std::collections::HashMap::new())
        }
    }

    /// Convert DynamicValue back to JSON map (v6)
    fn dynamic_value_v6_to_json_map(
        dynamic_value: super::grpc::tfplugin6_9::DynamicValue,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>> {
        // Try JSON format first
        if !dynamic_value.json.is_empty() {
            let json_value: serde_json::Value = serde_json::from_slice(&dynamic_value.json)
                .context("Failed to parse JSON from DynamicValue")?;

            if let serde_json::Value::Object(map) = json_value {
                Ok(map.into_iter().collect())
            } else {
                bail!("Expected JSON object in DynamicValue")
            }
        } else if !dynamic_value.msgpack.is_empty() {
            // Parse msgpack format
            let json_value: serde_json::Value = rmp_serde::from_slice(&dynamic_value.msgpack)
                .context("Failed to parse MessagePack from DynamicValue")?;

            if let serde_json::Value::Object(map) = json_value {
                Ok(map.into_iter().collect())
            } else {
                bail!("Expected JSON object from MessagePack in DynamicValue")
            }
        } else {
            // Empty dynamic value
            Ok(std::collections::HashMap::new())
        }
    }

    /// Convert DynamicValue back to JSON map, handling null responses (v6)
    fn dynamic_value_v6_to_optional_json_map(
        dynamic_value: super::grpc::tfplugin6_9::DynamicValue,
    ) -> Result<Option<std::collections::HashMap<String, serde_json::Value>>> {
        // Parse msgpack format to check if it's null first
        let json_value: serde_json::Value = rmp_serde::from_slice(&dynamic_value.msgpack)
            .context("Failed to parse MessagePack from DynamicValue")?;

        match json_value {
            serde_json::Value::Null => Ok(None),
            _ => Self::dynamic_value_v6_to_json_map(dynamic_value).map(Some),
        }
    }

    /// Complete resource state by adding null values for missing optional attributes
    ///
    /// Terraform providers expect all schema attributes to be present in the request,
    /// with null values for optional attributes that weren't provided by the user.
    /// This fixes the "object with N attributes required (M given)" error.
    fn complete_resource_state(
        user_provided_state: std::collections::HashMap<String, serde_json::Value>,
        resource_block: &crate::schema::Block,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>> {
        let mut complete_state = user_provided_state;

        // Add null values for missing attributes
        for (attr_name, attr) in &resource_block.attributes {
            if !complete_state.contains_key(attr_name) {
                // Only add null for optional or computed attributes (not required)
                if attr.optional || attr.computed {
                    complete_state.insert(attr_name.clone(), serde_json::Value::Null);
                }
            }
        }

        // Add appropriate default values for missing block types based on nesting mode
        for (block_name, block_type) in &resource_block.block_types {
            if !complete_state.contains_key(block_name) {
                use crate::schema::NestingMode;
                let default_value = match block_type.nesting {
                    NestingMode::Single => serde_json::Value::Object(serde_json::Map::new()),
                    NestingMode::List | NestingMode::Set => serde_json::Value::Array(vec![]),
                    NestingMode::Map => serde_json::Value::Object(serde_json::Map::new()),
                    NestingMode::Group => serde_json::Value::Object(serde_json::Map::new()),
                    NestingMode::Invalid => serde_json::Value::Null,
                };
                complete_state.insert(block_name.clone(), default_value);
            }
        }

        Ok(complete_state)
    }
}

/// Wrapper for provider schema responses from different protocol versions
pub enum ProviderSchema {
    V5(super::grpc::tfplugin5_9::get_provider_schema::Response),
    V6(super::grpc::tfplugin6_9::get_provider_schema::Response),
}

impl ProviderSchema {
    /// Check if provider schema is present
    pub fn has_provider(&self) -> bool {
        match self {
            ProviderSchema::V5(schema) => schema.provider.is_some(),
            ProviderSchema::V6(schema) => schema.provider.is_some(),
        }
    }

    /// Check if resource schemas are present
    pub fn has_resources(&self) -> bool {
        match self {
            ProviderSchema::V5(schema) => !schema.resource_schemas.is_empty(),
            ProviderSchema::V6(schema) => !schema.resource_schemas.is_empty(),
        }
    }

    /// Check if a specific resource type exists
    pub fn has_resource(&self, name: &str) -> bool {
        match self {
            ProviderSchema::V5(schema) => schema.resource_schemas.contains_key(name),
            ProviderSchema::V6(schema) => schema.resource_schemas.contains_key(name),
        }
    }
}

/// Terraform provider client that manages a provider process and gRPC connection
///
/// This struct handles the full lifecycle of a Terraform provider:
/// 1. Launching the provider process with required environment variables
/// 2. Reading and validating the handshake from provider stdout
/// 3. Establishing a gRPC connection using handshake network information
/// 4. Providing access to the gRPC client for service method calls
/// 5. Graceful shutdown via gRPC stop signal and process termination
pub struct ProviderClient {
    /// The provider process handle
    process: Child,
    /// The gRPC client connection to the provider
    client: Option<ClientConnection>,
}

/// Handshake information parsed from provider stdout
///
/// The go-plugin handshake protocol uses pipe-delimited fields:
/// CORE-PROTOCOL-VERSION | APP-PROTOCOL-VERSION | NETWORK-TYPE | NETWORK-ADDR | PROTOCOL
///
/// Example: "1|6|tcp|127.0.0.1:12345|grpc"
#[derive(Debug, Clone)]
struct HandshakeInfo {
    /// Core protocol version (must be "1")
    core_protocol_version: String,
    /// Application protocol version ("5" or "6" for Terraform)
    app_protocol_version: String,
    /// Network transport type ("tcp" or "unix")
    network_type: String,
    /// Network address (IP:port for tcp, socket path for unix)
    network_address: String,
    /// RPC protocol type (must be "grpc" for modern providers)
    protocol: String,
}

impl ProviderClient {
    /// Launch a new provider process and establish gRPC connection
    ///
    /// This method handles the complete provider initialization:
    /// 1. Spawns the provider binary with required environment variables
    /// 2. Reads the handshake from provider stdout
    /// 3. Validates handshake protocol compatibility
    /// 4. Establishes gRPC connection to the provider
    ///
    /// # Arguments
    /// * `provider_path` - Path to the provider binary executable
    ///
    /// # Returns
    /// A `ProviderClient` with an active gRPC connection to the provider
    ///
    /// # Errors
    /// Returns error if:
    /// - Provider process fails to launch
    /// - Handshake cannot be read or is invalid
    /// - gRPC connection establishment fails
    pub async fn launch(provider_path: &str) -> Result<Self> {
        // Set up environment variables for the provider
        let mut cmd = TokioCommand::new(provider_path);

        // Configure required environment variables
        cmd.env("PLUGIN_PROTOCOL_VERSIONS", "5,6");
        cmd.env(
            "TF_PLUGIN_MAGIC_COOKIE",
            "d602bf8f470bc67ca7faa0386276bbdd4330efaf76d1a219cb4d6991ca9872b2",
        );
        // Force TCP instead of Unix domain sockets for better tonic compatibility
        cmd.env("PLUGIN_MIN_PORT", "12000");
        cmd.env("PLUGIN_MAX_PORT", "13000");

        // Set up stdout capture and forward stderr
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::inherit());

        // Launch the provider process
        let mut process = cmd.spawn().context("Failed to launch provider process")?;

        // Read and parse the handshake from stdout
        let stdout = process
            .stdout
            .take()
            .context("Failed to capture provider stdout")?;

        let handshake = Self::read_handshake(stdout).await?;

        // Validate the handshake
        Self::validate_handshake(&handshake)?;

        // Create connection based on network type
        let channel = match handshake.network_type.as_str() {
            "tcp" => {
                let endpoint = format!("http://{}", handshake.network_address);
                Channel::from_shared(endpoint)?
                    .connect()
                    .await
                    .context("Failed to establish TCP gRPC connection to provider")?
            }
            "unix" => {
                // Create a custom connector for Unix domain sockets
                let socket_path = handshake.network_address.clone();
                Endpoint::try_from("http://[::]:50051")?
                    .connect_with_connector(service_fn(move |_: Uri| {
                        let socket_path = socket_path.clone();
                        async move {
                            let stream = UnixStream::connect(socket_path).await.map_err(|e| {
                                Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                            })?;
                            Ok::<TokioIo<UnixStream>, Box<dyn std::error::Error + Send + Sync>>(
                                TokioIo::new(stream),
                            )
                        }
                    }))
                    .await
                    .context("Failed to establish Unix socket gRPC connection to provider")?
            }
            _ => bail!("Unsupported network type: {}", handshake.network_type),
        };

        // Create client based on protocol version
        let client = match handshake.app_protocol_version.as_str() {
            "5" => ClientConnection::V5(
                ProviderServiceClientV5::new(channel)
                    .max_decoding_message_size(16 * 1024 * 1024)
                    .max_encoding_message_size(16 * 1024 * 1024),
            ),
            "6" => ClientConnection::V6(
                ProviderServiceClientV6::new(channel)
                    .max_decoding_message_size(16 * 1024 * 1024)
                    .max_encoding_message_size(16 * 1024 * 1024),
            ),
            _ => bail!(
                "Unsupported app protocol version: {}",
                handshake.app_protocol_version
            ),
        };

        Ok(ProviderClient {
            process,
            client: Some(client),
        })
    }

    /// Read the handshake line from provider stdout
    ///
    /// Providers write a single handshake line to stdout immediately after launch.
    /// This line contains protocol negotiation information in pipe-delimited format.
    async fn read_handshake<R: tokio::io::AsyncRead + Unpin>(stdout: R) -> Result<HandshakeInfo> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut reader = BufReader::new(stdout);
        let mut line = String::new();

        // Read the first line - this must be the handshake
        reader
            .read_line(&mut line)
            .await
            .context("Failed to read handshake from provider")?;

        // Parse the handshake line
        Self::parse_handshake(&line)
    }

    /// Parse handshake format: CORE-PROTOCOL-VERSION | APP-PROTOCOL-VERSION | NETWORK-TYPE | NETWORK-ADDR | PROTOCOL | [EMPTY]
    ///
    /// Validates the handshake line has 5 or 6 pipe-delimited fields and creates a HandshakeInfo struct.
    /// The 6th field may be empty and is ignored.
    // TODO: cross-check with docs
    fn parse_handshake(line: &str) -> Result<HandshakeInfo> {
        let parts: Vec<&str> = line.trim().split('|').collect();

        if parts.len() < 5 || parts.len() > 6 {
            bail!(
                "Invalid handshake format: expected 5-6 pipe-delimited fields, got {}",
                parts.len()
            );
        }

        Ok(HandshakeInfo {
            core_protocol_version: parts[0].to_string(),
            app_protocol_version: parts[1].to_string(),
            network_type: parts[2].to_string(),
            network_address: parts[3].to_string(),
            protocol: parts[4].to_string(),
        })
    }

    /// Validate the handshake information against supported protocol versions
    fn validate_handshake(handshake: &HandshakeInfo) -> Result<()> {
        // Core protocol version must be "1"
        if handshake.core_protocol_version != "1" {
            bail!(
                "Unsupported core protocol version: {}",
                handshake.core_protocol_version
            );
        }

        // Protocol must be "grpc" for modern providers
        if handshake.protocol != "grpc" {
            bail!(
                "Unsupported protocol: {} (expected 'grpc')",
                handshake.protocol
            );
        }

        // Network type must be "tcp" or "unix"
        match handshake.network_type.as_str() {
            "tcp" | "unix" => {}
            _ => bail!("Unsupported network type: {}", handshake.network_type),
        }

        // App protocol version should be 5 or 6 for Terraform
        match handshake.app_protocol_version.as_str() {
            "5" | "6" => {}
            _ => bail!(
                "Unsupported app protocol version: {}",
                handshake.app_protocol_version
            ),
        }

        Ok(())
    }

    /// Get a mutable reference to the gRPC client connection
    ///
    /// # Errors
    /// Returns error if the provider connection was not established
    pub fn client_connection(&mut self) -> Result<&mut ClientConnection> {
        self.client
            .as_mut()
            .context("Provider client not connected")
    }

    /// Shutdown the provider gracefully
    ///
    /// Sends a gRPC StopProvider request to the provider if connected, then terminates the process.
    pub async fn shutdown(mut self) -> Result<()> {
        // Send stop signal via gRPC if connected (only available in v6)
        if let Some(client) = self.client {
            match client {
                ClientConnection::V5(_) => {
                    // Protocol v5 doesn't have stop_provider, just kill the process
                }
                ClientConnection::V6(mut client) => {
                    let request =
                        tonic::Request::new(super::grpc::tfplugin6_9::stop_provider::Request {});
                    let _ = client.stop_provider(request).await; // Ignore errors on shutdown
                }
            }
        }

        // Kill the process
        self.process
            .kill()
            .await
            .context("Failed to kill provider process")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tf_provider_client::grpc;

    #[test]
    fn test_parse_handshake() {
        let line = "1|6|tcp|127.0.0.1:12345|grpc\n";
        let handshake = ProviderClient::parse_handshake(line).unwrap();

        assert_eq!(handshake.core_protocol_version, "1");
        assert_eq!(handshake.app_protocol_version, "6");
        assert_eq!(handshake.network_type, "tcp");
        assert_eq!(handshake.network_address, "127.0.0.1:12345");
        assert_eq!(handshake.protocol, "grpc");
    }

    #[test]
    fn test_validate_handshake() {
        let valid = HandshakeInfo {
            core_protocol_version: "1".to_string(),
            app_protocol_version: "6".to_string(),
            network_type: "tcp".to_string(),
            network_address: "127.0.0.1:12345".to_string(),
            protocol: "grpc".to_string(),
        };

        assert!(ProviderClient::validate_handshake(&valid).is_ok());

        let invalid_core = HandshakeInfo {
            core_protocol_version: "2".to_string(),
            ..valid.clone()
        };
        assert!(ProviderClient::validate_handshake(&invalid_core).is_err());

        let invalid_protocol = HandshakeInfo {
            protocol: "netrpc".to_string(),
            ..valid.clone()
        };
        assert!(ProviderClient::validate_handshake(&invalid_protocol).is_err());
    }

    #[tokio::test]
    async fn test_integration_terraform_provider_local() {
        // Skip test if provider path not available
        let provider_path = match std::env::var("_NIXOPS4_TEST_TERRAFORM_PROVIDER_LOCAL") {
            Ok(path) => path,
            Err(_) => {
                eprintln!(
                    "Skipping integration test: _NIXOPS4_TEST_TERRAFORM_PROVIDER_LOCAL not set"
                );
                return;
            }
        };

        // Launch the provider and establish connection
        let mut client = ProviderClient::launch(&provider_path)
            .await
            .expect("Failed to launch terraform-provider-local");

        // Get the gRPC client connection
        let client_connection = client
            .client_connection()
            .expect("Failed to get gRPC client");

        // Test GetProviderSchema call
        let schema = client_connection
            .get_provider_schema()
            .await
            .expect("Failed to get provider schema");

        // Verify we got a valid schema response
        assert!(schema.has_provider(), "Provider schema should be present");
        assert!(schema.has_resources(), "Should have resource schemas");

        // Verify local provider has expected resources
        assert!(
            schema.has_resource("local_file"),
            "Should have local_file resource"
        );
        assert!(
            schema.has_resource("local_sensitive_file"),
            "Should have local_sensitive_file resource"
        );

        // Test graceful shutdown
        client
            .shutdown()
            .await
            .expect("Failed to shutdown provider");
    }

    #[tokio::test]
    async fn test_provider_schema_content() {
        // Skip test if provider path not available
        let provider_path = match std::env::var("_NIXOPS4_TEST_TERRAFORM_PROVIDER_LOCAL") {
            Ok(path) => path,
            Err(_) => {
                eprintln!("Skipping schema test: _NIXOPS4_TEST_TERRAFORM_PROVIDER_LOCAL not set");
                return;
            }
        };

        // Launch the provider
        let mut client = ProviderClient::launch(&provider_path)
            .await
            .expect("Failed to launch terraform-provider-local");

        // Get schema
        let schema = client
            .client_connection()
            .expect("Failed to get gRPC client")
            .get_provider_schema()
            .await
            .expect("Failed to get provider schema");

        // Examine schema content based on protocol version
        match &schema {
            ProviderSchema::V5(s) => {
                // Check provider schema exists
                assert!(s.provider.is_some(), "Provider schema should exist");

                // Check we have expected resources
                assert!(
                    s.resource_schemas.contains_key("local_file"),
                    "Should have local_file resource"
                );
                assert!(
                    s.resource_schemas.contains_key("local_sensitive_file"),
                    "Should have local_sensitive_file resource"
                );

                // Examine local_file resource schema
                let local_file = &s.resource_schemas["local_file"];
                assert!(
                    local_file.block.is_some(),
                    "local_file should have block schema"
                );

                let block = local_file.block.as_ref().unwrap();

                // Check for expected attributes
                let has_content = block.attributes.iter().any(|attr| attr.name == "content");
                let has_filename = block.attributes.iter().any(|attr| attr.name == "filename");
                assert!(has_content, "Should have content attribute");
                assert!(has_filename, "Should have filename attribute");

                println!("V5 Provider schema validation passed");
                println!("Resource count: {}", s.resource_schemas.len());
                println!("Data source count: {}", s.data_source_schemas.len());
            }
            ProviderSchema::V6(s) => {
                // Check provider schema exists
                assert!(s.provider.is_some(), "Provider schema should exist");

                // Check we have expected resources
                assert!(
                    s.resource_schemas.contains_key("local_file"),
                    "Should have local_file resource"
                );
                assert!(
                    s.resource_schemas.contains_key("local_sensitive_file"),
                    "Should have local_sensitive_file resource"
                );

                // Examine local_file resource schema
                let local_file = &s.resource_schemas["local_file"];
                assert!(
                    local_file.block.is_some(),
                    "local_file should have block schema"
                );

                let block = local_file.block.as_ref().unwrap();

                // Check for expected attributes
                let has_content = block.attributes.iter().any(|attr| attr.name == "content");
                let has_filename = block.attributes.iter().any(|attr| attr.name == "filename");
                assert!(has_content, "Should have content attribute");
                assert!(has_filename, "Should have filename attribute");

                println!("V6 Provider schema validation passed");
                println!("Resource count: {}", s.resource_schemas.len());
                println!("Data source count: {}", s.data_source_schemas.len());
            }
        }

        client
            .shutdown()
            .await
            .expect("Failed to shutdown provider");
    }

    #[tokio::test]
    async fn test_configure_provider() {
        // Skip test if provider path not available
        let provider_path = match std::env::var("_NIXOPS4_TEST_TERRAFORM_PROVIDER_LOCAL") {
            Ok(path) => path,
            Err(_) => {
                eprintln!(
                    "Skipping configure test: _NIXOPS4_TEST_TERRAFORM_PROVIDER_LOCAL not set"
                );
                return;
            }
        };

        // Launch the provider
        let mut client = ProviderClient::launch(&provider_path)
            .await
            .expect("Failed to launch terraform-provider-local");

        // Test provider configuration (local provider doesn't require config, but test the call)
        let config = std::collections::HashMap::new();

        let result = client
            .client_connection()
            .expect("Failed to get gRPC client")
            .configure_provider(config)
            .await;

        assert!(
            result.is_ok(),
            "ConfigureProvider should succeed: {:?}",
            result
        );

        client
            .shutdown()
            .await
            .expect("Failed to shutdown provider");
    }

    #[tokio::test]
    async fn test_create_local_file() {
        // Skip test if provider path not available
        let provider_path = match std::env::var("_NIXOPS4_TEST_TERRAFORM_PROVIDER_LOCAL") {
            Ok(path) => path,
            Err(_) => {
                eprintln!("Skipping create test: _NIXOPS4_TEST_TERRAFORM_PROVIDER_LOCAL not set");
                return;
            }
        };

        // Launch the provider
        let mut client = ProviderClient::launch(&provider_path)
            .await
            .expect("Failed to launch terraform-provider-local");

        // Configure provider (empty config for local provider)
        let config = std::collections::HashMap::new();
        client
            .client_connection()
            .expect("Failed to get gRPC client")
            .configure_provider(config)
            .await
            .expect("Failed to configure provider");

        // Create a local_file resource in a temporary directory
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let test_file = temp_dir.path().join("nixops4-test-file.txt");
        let test_file_path = test_file.to_string_lossy().to_string();

        let mut planned_state = std::collections::HashMap::new();
        planned_state.insert(
            "filename".to_string(),
            serde_json::Value::String(test_file_path.clone()),
        );
        planned_state.insert(
            "content".to_string(),
            serde_json::Value::String("Hello from NixOps4 Terraform integration!".to_string()),
        );
        planned_state.insert(
            "file_permission".to_string(),
            serde_json::Value::String("0644".to_string()),
        );

        let result = client
            .client_connection()
            .expect("Failed to get gRPC client")
            .apply_resource_change(
                "local_file",
                None, // no prior state for create
                planned_state,
            )
            .await;

        assert!(
            result.is_ok(),
            "ApplyResourceChange (create) should succeed: {:?}",
            result
        );

        let new_state = result.unwrap();

        // Validate the terraform provider response (msgpack parsing successful!)
        assert!(
            !new_state.is_empty(),
            "Response should not be empty with msgpack parsing"
        );
        assert!(
            new_state.contains_key("filename"),
            "Response should contain filename. Keys: {:?}",
            new_state.keys().collect::<Vec<_>>()
        );
        assert!(
            new_state.contains_key("content"),
            "Response should contain content"
        );
        assert!(
            new_state.contains_key("id"),
            "Response should contain id (computed field)"
        );

        // Verify the content matches what we sent
        assert_eq!(
            new_state.get("content"),
            Some(&serde_json::Value::String(
                "Hello from NixOps4 Terraform integration!".to_string()
            ))
        );

        // Verify the filename matches what we sent
        assert_eq!(
            new_state.get("filename"),
            Some(&serde_json::Value::String(test_file_path.clone()))
        );

        // Verify we got computed fields like content hashes
        assert!(
            new_state.contains_key("content_md5"),
            "Should have content_md5"
        );
        assert!(
            new_state.contains_key("content_sha256"),
            "Should have content_sha256"
        );

        // Note: We skip file content verification due to permission differences between
        // the terraform provider process and test process in the nix environment

        // temp_dir will be automatically cleaned up when it goes out of scope

        client
            .shutdown()
            .await
            .expect("Failed to shutdown provider");
    }

    #[tokio::test]
    async fn test_update_local_file() {
        // Skip test if provider path not available
        let provider_path = match std::env::var("_NIXOPS4_TEST_TERRAFORM_PROVIDER_LOCAL") {
            Ok(path) => path,
            Err(_) => {
                eprintln!("Skipping update test: _NIXOPS4_TEST_TERRAFORM_PROVIDER_LOCAL not set");
                return;
            }
        };

        // Launch the provider
        let mut client = ProviderClient::launch(&provider_path)
            .await
            .expect("Failed to launch terraform-provider-local");

        // Configure provider
        let config = std::collections::HashMap::new();
        client
            .client_connection()
            .expect("Failed to get gRPC client")
            .configure_provider(config)
            .await
            .expect("Failed to configure provider");

        // First create a file in temp directory
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let test_file = temp_dir.path().join("nixops4-test-update-file.txt");
        let test_file_path = test_file.to_string_lossy().to_string();

        let mut initial_state = std::collections::HashMap::new();
        initial_state.insert(
            "filename".to_string(),
            serde_json::Value::String(test_file_path.clone()),
        );
        initial_state.insert(
            "content".to_string(),
            serde_json::Value::String("Initial content".to_string()),
        );
        initial_state.insert(
            "file_permission".to_string(),
            serde_json::Value::String("0644".to_string()),
        );

        let create_result = client
            .client_connection()
            .expect("Failed to get gRPC client")
            .apply_resource_change("local_file", None, initial_state.clone())
            .await
            .expect("Failed to create initial file");

        // Verify the create operation returned expected fields
        assert!(
            create_result.contains_key("filename"),
            "Create result should contain filename"
        );
        assert!(
            create_result.contains_key("content"),
            "Create result should contain content"
        );
        assert_eq!(
            create_result.get("content"),
            Some(&serde_json::Value::String("Initial content".to_string()))
        );

        // Now attempt to update the file with new content
        let mut updated_state = std::collections::HashMap::new();
        updated_state.insert(
            "filename".to_string(),
            serde_json::Value::String(test_file_path.clone()),
        );
        updated_state.insert(
            "content".to_string(),
            serde_json::Value::String("Updated content from NixOps4!".to_string()),
        );
        updated_state.insert(
            "file_permission".to_string(),
            serde_json::Value::String("0644".to_string()),
        );

        let update_result = client
            .client_connection()
            .expect("Failed to get gRPC client")
            .apply_resource_change(
                "local_file",
                Some(create_result), // pass prior state
                updated_state,
            )
            .await;

        assert!(
            update_result.is_ok(),
            "ApplyResourceChange (update) should succeed: {:?}",
            update_result
        );

        let new_state = update_result.unwrap();

        // Verify the response
        assert!(
            new_state.contains_key("filename"),
            "Response should contain filename"
        );
        assert!(
            new_state.contains_key("content"),
            "Response should contain content"
        );
        // The provider claims the update succeeded and returns the planned state
        assert_eq!(
            new_state.get("content"),
            Some(&serde_json::Value::String(
                "Updated content from NixOps4!".to_string()
            ))
        );

        // NOTE: terraform provider local_file does not actually support updates
        // The Update method is a no-op that just returns the planned state without
        // performing any file operations. This is confirmed by examining the source.
        //
        // TODO: Consider using https://github.com/rancher/terraform-provider-file/blob/main/internal/provider/file_local_resource.go
        // which may have proper update support
        //
        // We assert the current behavior (no actual update) to document this limitation
        let actual_file_content =
            std::fs::read_to_string(&test_file_path).expect("File should still exist on disk");
        assert_eq!(actual_file_content, "Initial content",
                   "terraform provider local_file doesn't actually update files - this documents current behavior");

        // temp_dir will be automatically cleaned up when it goes out of scope

        client
            .shutdown()
            .await
            .expect("Failed to shutdown provider");
    }

    #[tokio::test]
    async fn test_create_with_provider_config() {
        // Skip test if provider path not available
        let provider_path = match std::env::var("_NIXOPS4_TEST_TERRAFORM_PROVIDER_LOCAL") {
            Ok(path) => path,
            Err(_) => {
                eprintln!(
                    "Skipping provider config test: _NIXOPS4_TEST_TERRAFORM_PROVIDER_LOCAL not set"
                );
                return;
            }
        };

        // Launch the provider
        let mut client = ProviderClient::launch(&provider_path)
            .await
            .expect("Failed to launch terraform-provider-local");

        // Test with empty configuration (local provider accepts no configuration)
        let config = std::collections::HashMap::new();

        let configure_result = client
            .client_connection()
            .expect("Failed to get gRPC client")
            .configure_provider(config)
            .await;

        // Should succeed with empty config
        assert!(
            configure_result.is_ok(),
            "ConfigureProvider with config should succeed: {:?}",
            configure_result
        );

        // Create a resource after configuration
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let test_file = temp_dir.path().join("nixops4-test-configured-file.txt");

        let mut planned_state = std::collections::HashMap::new();
        planned_state.insert(
            "filename".to_string(),
            serde_json::Value::String(test_file.to_string_lossy().to_string()),
        );
        planned_state.insert(
            "content".to_string(),
            serde_json::Value::String("File created after provider config".to_string()),
        );
        planned_state.insert(
            "file_permission".to_string(),
            serde_json::Value::String("0644".to_string()),
        );

        let create_result = client
            .client_connection()
            .expect("Failed to get gRPC client")
            .apply_resource_change("local_file", None, planned_state)
            .await;

        assert!(
            create_result.is_ok(),
            "Resource creation after configuration should succeed: {:?}",
            create_result
        );

        // temp_dir will be automatically cleaned up when it goes out of scope

        client
            .shutdown()
            .await
            .expect("Failed to shutdown provider");
    }
}
