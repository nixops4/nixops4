mod generated {
    // This module only contains generated code.
    #![allow(clippy::all)]
    include!(concat!(env!("OUT_DIR"), "/generated/schema/resource/v0.rs"));
}
pub use generated::*;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn examples_v0_create_resource_request() {
        let json = include_str!("../../examples/v0/CreateResourceRequest.json");
        let _value: CreateResourceRequest = serde_json::from_str(json).unwrap();
        assert_eq!(
            _value,
            CreateResourceRequest {
                type_: ResourceType("file".to_string()),
                input_properties: InputProperties(serde_json::Map::from_iter(vec![
                    ("path".to_string(), Value::String("pubkey.txt".to_string())),
                    (
                        "content".to_string(),
                        Value::String("ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQD".to_string())
                    ),
                ])),
                is_stateful: false,
            }
        );
    }

    #[test]
    fn examples_v0_create_resource_response() {
        let json = include_str!("../../examples/v0/CreateResourceResponse.json");
        let _value: CreateResourceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            _value,
            CreateResourceResponse {
                output_properties: OutputProperties(serde_json::Map::from_iter(vec![
                    ("id".to_string(), Value::String("vm-12w94ty8".to_string())),
                    (
                        "interfaces".to_string(),
                        object_from_iter(vec![(
                            "eth0".to_string(),
                            object_from_iter(vec![(
                                "ipv4".to_string(),
                                Value::String("198.51.100.11".to_string())
                            )])
                        )])
                    )
                ])),
            }
        );
    }

    fn object_from_iter<T: IntoIterator<Item = (String, Value)>>(x: T) -> Value {
        Value::Object(serde_json::Map::from_iter(x))
    }
}
