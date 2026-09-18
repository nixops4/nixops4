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
                private_state: None,
            }
        );
    }

    #[test]
    fn extant_resource_roundtrips_private_state() {
        let json = r#"{
            "type": "file",
            "inputProperties": { "path": "p" },
            "privateState": { "schemaVersion": 3, "cookies": [1, 2] }
        }"#;
        let value: ExtantResource = serde_json::from_str(json).unwrap();
        let expected_private = serde_json::json!({
            "schemaVersion": 3,
            "cookies": [1, 2]
        });
        assert_eq!(value.private_state, Some(PrivateState(expected_private)));
    }

    #[test]
    fn create_resource_response_carries_new_private_state() {
        let json = r#"{
            "outputProperties": { "id": "x" },
            "privateState": "opaque-blob-goes-here"
        }"#;
        let value: CreateResourceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            value.private_state,
            Some(PrivateState(Value::String(
                "opaque-blob-goes-here".to_string()
            )))
        );
    }

    #[test]
    fn update_resource_response_carries_new_private_state() {
        let json = r#"{
            "outputProperties": { "id": "x" },
            "privateState": [1, 2, 3]
        }"#;
        let value: UpdateResourceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            value.private_state,
            Some(PrivateState(serde_json::json!([1, 2, 3])))
        );
    }

    // We don't distinguish absent from null.
    #[test]
    fn update_resource_response_null_and_absent_both_none() {
        let with_null = r#"{ "outputProperties": {}, "privateState": null }"#;
        let absent = r#"{ "outputProperties": {} }"#;
        let a: UpdateResourceResponse = serde_json::from_str(with_null).unwrap();
        let b: UpdateResourceResponse = serde_json::from_str(absent).unwrap();
        assert_eq!(a.private_state, None);
        assert_eq!(b.private_state, None);
    }

    fn object_from_iter<T: IntoIterator<Item = (String, Value)>>(x: T) -> Value {
        Value::Object(serde_json::Map::from_iter(x))
    }
}
