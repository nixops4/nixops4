pub mod framework;
pub mod meta;
pub mod schema;

// type JSONObject = Map<String, Value>;

// #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// pub struct CreateResourceRequest {
//     /// Parameters that define the resource.
//     pub input_properties: JSONObject,
// }

// #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// pub struct CreateResourceResponse {
//     /// The output properties of the resource. The keys may be entirely disjoin from the input properties.
//     /// In fact, mirroring the input properties is discouraged, because it requires running the resource provider for no reason, and it makes the plan/preview functionality less effective.
//     pub output_properties: JSONObject,
// }
