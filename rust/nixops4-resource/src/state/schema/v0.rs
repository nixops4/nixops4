mod generated {
    // This module only contains generated code.
    #![allow(clippy::all)]
    include!(concat!(env!("OUT_DIR"), "/generated/schema/state/v0.rs"));
}
pub use generated::*;

#[cfg(test)]
mod tests {

    mod event {
        use json_patch::{jsonptr::PointerBuf, AddOperation, Patch, PatchOperation};
        use serde_json::json;

        use super::super::*;

        #[test]
        fn examples_v0_create_resource_request() {
            let json = include_str!("../../../examples/state/v0/initial-event.json");
            let _value: StateEvent = serde_json::from_str(json).unwrap();
            assert_eq!(
                _value,
                StateEvent {
                    index: 0,
                    meta: StateEventMeta {
                        time: chrono::DateTime::parse_from_rfc3339(
                            "2025-04-15T11:48:06.434450914Z"
                        )
                        .unwrap()
                        .with_timezone(&chrono::offset::Utc)
                    },
                    patch: Patch(vec![PatchOperation::Add(AddOperation {
                        path: PointerBuf::root(),
                        value: json!({
                            "_type": "nixopsState".to_string(),
                            "deployments": { },
                            "resources": { }
                        })
                    })])
                }
            );
        }

        #[test]
        fn event_roundtrip_serialization() {
            let event = StateEvent {
                index: 42,
                meta: StateEventMeta {
                    time: chrono::Utc::now(),
                },
                patch: Patch(vec![PatchOperation::Add(AddOperation {
                    path: PointerBuf::from_tokens(vec!["test"]),
                    value: json!("value"),
                })]),
            };
            let json = serde_json::to_string(&event).unwrap();
            let deserialized: StateEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(event.index, deserialized.index);
            assert_eq!(event.patch, deserialized.patch);
        }
    }

    mod state {
        use super::super::*;

        #[test]
        fn examples_v0_empty() {
            let json = include_str!("../../../examples/state/v0/empty.json");
            let _value: State = serde_json::from_str(json).unwrap();
            assert_eq!(
                _value,
                State {
                    resources: Default::default(),
                    type_: StateType::NixopsState,
                    deployments: StateDeployments {},
                }
            );
        }

        #[test]
        fn state_roundtrip_serialization() {
            let state = State {
                resources: Default::default(),
                type_: StateType::NixopsState,
                deployments: StateDeployments {},
            };
            let json = serde_json::to_string(&state).unwrap();
            let deserialized: State = serde_json::from_str(&json).unwrap();
            assert_eq!(state, deserialized);
        }
    }
}
