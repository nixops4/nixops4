use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Our unified schema format that abstracts over tfplugin5 and tfplugin6 differences
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSchema {
    /// Schema for the provider configuration itself
    pub provider: Option<Schema>,
    /// Schemas for each resource type this provider supports
    pub resource_schemas: HashMap<String, Schema>,
    /// Schemas for each data source type this provider supports  
    pub data_source_schemas: HashMap<String, Schema>,
}

/// Schema definition for a resource, data source, or provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    /// Version of this schema
    pub version: i64,
    /// The root block schema
    pub block: Option<Block>,
}

/// A configuration block containing attributes and nested blocks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    /// Map of attribute names to their schemas
    pub attributes: HashMap<String, Attribute>,
    /// Map of nested block type names to their schemas
    pub block_types: HashMap<String, NestedBlock>,
    /// Human-readable description of this block
    pub description: Option<String>,
    /// Whether this description is formatted as markdown
    pub description_kind: DescriptionKind,
}

/// Schema for a single configuration attribute
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attribute {
    /// The data type of this attribute
    pub r#type: String, // JSON representation of cty.Type
    /// Human-readable description
    pub description: Option<String>,
    /// Whether this description is formatted as markdown
    pub description_kind: DescriptionKind,
    /// Whether this attribute is required
    pub required: bool,
    /// Whether this attribute is optional
    pub optional: bool,
    /// Whether this attribute is computed (output-only)
    pub computed: bool,
    /// Whether this attribute is sensitive (should be redacted)
    pub sensitive: bool,
}

/// Schema for a nested configuration block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NestedBlock {
    /// The schema of this nested block
    pub block: Block,
    /// How many instances of this block are allowed
    pub nesting: NestingMode,
}

/// How nested blocks can be structured
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NestingMode {
    /// Invalid nesting
    Invalid,
    /// Single instance: block { }
    Single,
    /// Multiple instances: block { } block { }
    List,
    /// Multiple named instances: block "name" { }
    Set,
    /// Map with string keys: block { key = value }
    Map,
    /// Group nesting (deprecated in Terraform)
    Group,
}

/// How descriptions should be interpreted
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DescriptionKind {
    /// Plain text
    Plain,
    /// Markdown formatted text
    Markdown,
}

impl ProviderSchema {
    /// Check if provider schema is present
    pub fn has_provider(&self) -> bool {
        self.provider.is_some()
    }

    /// Check if resource schemas are present
    pub fn has_resources(&self) -> bool {
        !self.resource_schemas.is_empty()
    }

    /// Check if a specific resource type exists
    pub fn has_resource(&self, name: &str) -> bool {
        self.resource_schemas.contains_key(name)
    }

    /// Get the schema for a specific resource type
    pub fn get_resource_schema(&self, name: &str) -> Option<&Schema> {
        self.resource_schemas.get(name)
    }

    /// Check if data source schemas are present
    pub fn has_data_sources(&self) -> bool {
        !self.data_source_schemas.is_empty()
    }

    /// Check if a specific data source type exists
    pub fn has_data_source(&self, name: &str) -> bool {
        self.data_source_schemas.contains_key(name)
    }
}

impl ProviderSchema {
    /// Convert block types from protobuf format to our unified format (V5)
    fn convert_block_types_v5(
        block_types: Vec<crate::tf_provider_client::grpc::tfplugin5_9::schema::NestedBlock>,
    ) -> HashMap<String, NestedBlock> {
        block_types
            .into_iter()
            .map(|nested_block| {
                let type_name = nested_block.type_name.clone();
                let block_desc = nested_block.block.as_ref().and_then(|b| {
                    if b.description.is_empty() {
                        None
                    } else {
                        Some(b.description.clone())
                    }
                });

                (
                    type_name,
                    NestedBlock {
                        block: Block {
                            attributes: nested_block
                                .block
                                .map(|block| block.attributes)
                                .unwrap_or_default()
                                .into_iter()
                                .map(|attr| {
                                    (
                                        attr.name.clone(),
                                        Attribute {
                                            r#type: String::from_utf8_lossy(&attr.r#type)
                                                .to_string(),
                                            description: if attr.description.is_empty() {
                                                None
                                            } else {
                                                Some(attr.description)
                                            },
                                            description_kind: DescriptionKind::Plain,
                                            required: attr.required,
                                            optional: attr.optional,
                                            computed: attr.computed,
                                            sensitive: attr.sensitive,
                                        },
                                    )
                                })
                                .collect(),
                            block_types: HashMap::new(), // Keep nested blocks simple for now
                            description: block_desc,
                            description_kind: DescriptionKind::Plain,
                        },
                        nesting: match nested_block.nesting {
                            0 => NestingMode::Invalid,
                            1 => NestingMode::Single,
                            2 => NestingMode::List,
                            3 => NestingMode::Set,
                            4 => NestingMode::Map,
                            5 => NestingMode::Group,
                            _ => NestingMode::Invalid,
                        },
                    },
                )
            })
            .collect()
    }

    /// Convert block types from protobuf format to our unified format (V6)
    fn convert_block_types_v6(
        block_types: Vec<crate::tf_provider_client::grpc::tfplugin6_9::schema::NestedBlock>,
    ) -> HashMap<String, NestedBlock> {
        block_types
            .into_iter()
            .map(|nested_block| {
                let type_name = nested_block.type_name.clone();
                let block_desc = nested_block.block.as_ref().and_then(|b| {
                    if b.description.is_empty() {
                        None
                    } else {
                        Some(b.description.clone())
                    }
                });

                (
                    type_name,
                    NestedBlock {
                        block: Block {
                            attributes: nested_block
                                .block
                                .map(|block| block.attributes)
                                .unwrap_or_default()
                                .into_iter()
                                .map(|attr| {
                                    (
                                        attr.name.clone(),
                                        Attribute {
                                            r#type: String::from_utf8_lossy(&attr.r#type)
                                                .to_string(),
                                            description: if attr.description.is_empty() {
                                                None
                                            } else {
                                                Some(attr.description)
                                            },
                                            description_kind: DescriptionKind::Plain,
                                            required: attr.required,
                                            optional: attr.optional,
                                            computed: attr.computed,
                                            sensitive: attr.sensitive,
                                        },
                                    )
                                })
                                .collect(),
                            block_types: HashMap::new(), // Keep nested blocks simple for now
                            description: block_desc,
                            description_kind: DescriptionKind::Plain,
                        },
                        nesting: match nested_block.nesting {
                            0 => NestingMode::Invalid,
                            1 => NestingMode::Single,
                            2 => NestingMode::List,
                            3 => NestingMode::Set,
                            4 => NestingMode::Map,
                            5 => NestingMode::Group,
                            _ => NestingMode::Invalid,
                        },
                    },
                )
            })
            .collect()
    }

    /// Convert from raw protocol response to our unified format
    pub fn from_raw_response(raw_schema: crate::tf_provider_client::ProviderSchema) -> Self {
        match raw_schema {
            crate::tf_provider_client::ProviderSchema::V5(response) => {
                ProviderSchema {
                    provider: response.provider.map(|s| Schema {
                        version: s.version,
                        block: s.block.map(|b| crate::schema::Block {
                            attributes: b
                                .attributes
                                .into_iter()
                                .map(|attr| {
                                    (
                                        attr.name.clone(),
                                        crate::schema::Attribute {
                                            r#type: String::from_utf8_lossy(&attr.r#type)
                                                .to_string(),
                                            description: if attr.description.is_empty() {
                                                None
                                            } else {
                                                Some(attr.description)
                                            },
                                            description_kind: crate::schema::DescriptionKind::Plain, // Simplified for now
                                            required: attr.required,
                                            optional: attr.optional,
                                            computed: attr.computed,
                                            sensitive: attr.sensitive,
                                        },
                                    )
                                })
                                .collect(),
                            block_types: Self::convert_block_types_v5(b.block_types),
                            description: if b.description.is_empty() {
                                None
                            } else {
                                Some(b.description)
                            },
                            description_kind: DescriptionKind::Plain, // Simplified for now
                        }),
                    }),
                    resource_schemas: response
                        .resource_schemas
                        .into_iter()
                        .map(|(name, s)| {
                            (
                                name,
                                Schema {
                                    version: s.version,
                                    block: s.block.map(|b| crate::schema::Block {
                                        attributes: b
                                            .attributes
                                            .into_iter()
                                            .map(|attr| {
                                                (
                                                    attr.name.clone(),
                                                    crate::schema::Attribute {
                                                        r#type: String::from_utf8_lossy(
                                                            &attr.r#type,
                                                        )
                                                        .to_string(),
                                                        description: if attr.description.is_empty()
                                                        {
                                                            None
                                                        } else {
                                                            Some(attr.description)
                                                        },
                                                        description_kind:
                                                            crate::schema::DescriptionKind::Plain,
                                                        required: attr.required,
                                                        optional: attr.optional,
                                                        computed: attr.computed,
                                                        sensitive: attr.sensitive,
                                                    },
                                                )
                                            })
                                            .collect(),
                                        block_types: Self::convert_block_types_v5(b.block_types),
                                        description: if b.description.is_empty() {
                                            None
                                        } else {
                                            Some(b.description)
                                        },
                                        description_kind: crate::schema::DescriptionKind::Plain,
                                    }),
                                },
                            )
                        })
                        .collect(),
                    data_source_schemas: response
                        .data_source_schemas
                        .into_iter()
                        .map(|(name, s)| {
                            (
                                name,
                                Schema {
                                    version: s.version,
                                    block: s.block.map(|b| crate::schema::Block {
                                        attributes: b
                                            .attributes
                                            .into_iter()
                                            .map(|attr| {
                                                (
                                                    attr.name.clone(),
                                                    crate::schema::Attribute {
                                                        r#type: String::from_utf8_lossy(
                                                            &attr.r#type,
                                                        )
                                                        .to_string(),
                                                        description: if attr.description.is_empty()
                                                        {
                                                            None
                                                        } else {
                                                            Some(attr.description)
                                                        },
                                                        description_kind:
                                                            crate::schema::DescriptionKind::Plain,
                                                        required: attr.required,
                                                        optional: attr.optional,
                                                        computed: attr.computed,
                                                        sensitive: attr.sensitive,
                                                    },
                                                )
                                            })
                                            .collect(),
                                        block_types: Self::convert_block_types_v5(b.block_types),
                                        description: if b.description.is_empty() {
                                            None
                                        } else {
                                            Some(b.description)
                                        },
                                        description_kind: crate::schema::DescriptionKind::Plain,
                                    }),
                                },
                            )
                        })
                        .collect(),
                }
            }
            crate::tf_provider_client::ProviderSchema::V6(response) => {
                ProviderSchema {
                    provider: response.provider.map(|s| Schema {
                        version: s.version,
                        block: s.block.map(|b| crate::schema::Block {
                            attributes: b
                                .attributes
                                .into_iter()
                                .map(|attr| {
                                    (
                                        attr.name.clone(),
                                        crate::schema::Attribute {
                                            r#type: String::from_utf8_lossy(&attr.r#type)
                                                .to_string(),
                                            description: if attr.description.is_empty() {
                                                None
                                            } else {
                                                Some(attr.description)
                                            },
                                            description_kind: crate::schema::DescriptionKind::Plain, // Simplified for now
                                            required: attr.required,
                                            optional: attr.optional,
                                            computed: attr.computed,
                                            sensitive: attr.sensitive,
                                        },
                                    )
                                })
                                .collect(),
                            block_types: Self::convert_block_types_v6(b.block_types),
                            description: if b.description.is_empty() {
                                None
                            } else {
                                Some(b.description)
                            },
                            description_kind: DescriptionKind::Plain, // Simplified for now
                        }),
                    }),
                    resource_schemas: response
                        .resource_schemas
                        .into_iter()
                        .map(|(name, s)| {
                            (
                                name,
                                Schema {
                                    version: s.version,
                                    block: s.block.map(|b| crate::schema::Block {
                                        attributes: b
                                            .attributes
                                            .into_iter()
                                            .map(|attr| {
                                                (
                                                    attr.name.clone(),
                                                    crate::schema::Attribute {
                                                        r#type: String::from_utf8_lossy(
                                                            &attr.r#type,
                                                        )
                                                        .to_string(),
                                                        description: if attr.description.is_empty()
                                                        {
                                                            None
                                                        } else {
                                                            Some(attr.description)
                                                        },
                                                        description_kind:
                                                            crate::schema::DescriptionKind::Plain,
                                                        required: attr.required,
                                                        optional: attr.optional,
                                                        computed: attr.computed,
                                                        sensitive: attr.sensitive,
                                                    },
                                                )
                                            })
                                            .collect(),
                                        block_types: Self::convert_block_types_v6(b.block_types),
                                        description: if b.description.is_empty() {
                                            None
                                        } else {
                                            Some(b.description)
                                        },
                                        description_kind: crate::schema::DescriptionKind::Plain,
                                    }),
                                },
                            )
                        })
                        .collect(),
                    data_source_schemas: response
                        .data_source_schemas
                        .into_iter()
                        .map(|(name, s)| {
                            (
                                name,
                                Schema {
                                    version: s.version,
                                    block: s.block.map(|b| crate::schema::Block {
                                        attributes: b
                                            .attributes
                                            .into_iter()
                                            .map(|attr| {
                                                (
                                                    attr.name.clone(),
                                                    crate::schema::Attribute {
                                                        r#type: String::from_utf8_lossy(
                                                            &attr.r#type,
                                                        )
                                                        .to_string(),
                                                        description: if attr.description.is_empty()
                                                        {
                                                            None
                                                        } else {
                                                            Some(attr.description)
                                                        },
                                                        description_kind:
                                                            crate::schema::DescriptionKind::Plain,
                                                        required: attr.required,
                                                        optional: attr.optional,
                                                        computed: attr.computed,
                                                        sensitive: attr.sensitive,
                                                    },
                                                )
                                            })
                                            .collect(),
                                        block_types: Self::convert_block_types_v6(b.block_types),
                                        description: if b.description.is_empty() {
                                            None
                                        } else {
                                            Some(b.description)
                                        },
                                        description_kind: crate::schema::DescriptionKind::Plain,
                                    }),
                                },
                            )
                        })
                        .collect(),
                }
            }
        }
    }
}

pub fn translate_terraform_schema(
    _terraform_schema: serde_json::Value,
) -> Result<serde_json::Value> {
    todo!("Implement schema translation from Terraform to NixOps4 format")
}
