use nixops4_resource_runner::{ProviderResource, ResourceType};
use serde_json::Value;

/// A resource that exists as far as the engine is concerned: its type, its
/// input properties and, once applied, its output properties.
#[derive(Debug, Clone)]
pub struct ExtantResource {
    pub type_: String,
    pub input_properties: serde_json::Map<String, Value>,
    /// `None` until the resource has been applied
    pub output_properties: Option<serde_json::Map<String, Value>>,
}

impl From<&ExtantResource> for ProviderResource {
    fn from(resource: &ExtantResource) -> Self {
        ProviderResource {
            type_: ResourceType::new(resource.type_.clone()),
            input_properties: resource.input_properties.clone(),
            output_properties: resource.output_properties.clone(),
        }
    }
}
