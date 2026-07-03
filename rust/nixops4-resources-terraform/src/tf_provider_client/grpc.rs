/// Generated protobuf code for Terraform Plugin Protocol v5.9
#[allow(dead_code)]
pub mod tfplugin5_9 {
    include!(concat!(env!("OUT_DIR"), "/tfplugin5.rs"));
}

/// Generated protobuf code for Terraform Plugin Protocol v6.9
#[allow(dead_code)]
pub mod tfplugin6_9 {
    include!(concat!(env!("OUT_DIR"), "/tfplugin6.rs"));
}

/// Protocol version 5 client
pub use tfplugin5_9::provider_client::ProviderClient as ProviderServiceClientV5;

/// Protocol version 6 client
pub use tfplugin6_9::provider_client::ProviderClient as ProviderServiceClientV6;
