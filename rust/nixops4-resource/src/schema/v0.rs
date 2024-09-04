use serde::{Deserialize, Serialize};
schemafy::schemafy!("resource-schema-v0.json");

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::collections::BTreeMap;

    #[test]
    fn examples_v0_create_resource_request() {
        let json = include_str!("../../examples/v0/CreateResourceRequest.json");
        let _value: CreateResourceRequest = serde_json::from_str(json).unwrap();
        assert_eq!(
            _value,
            CreateResourceRequest {
                type_: "file".to_string(),
                input_properties: BTreeMap::from_iter(vec![
                    ("path".to_string(), Value::String("pubkey.txt".to_string())),
                    (
                        "content".to_string(),
                        Value::String("ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQD".to_string())
                    ),
                ]),
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
                output_properties: BTreeMap::from_iter(vec![
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
                ]),
            }
        );
    }

    fn object_from_iter<T: IntoIterator<Item = (String, Value)>>(x: T) -> Value {
        Value::Object(serde_json::Map::from_iter(x))
    }
}
