/// Generated protobuf code for Terraform Plugin Protocol v5.9
pub mod tfplugin5_9 {
    include!(concat!(env!("OUT_DIR"), "/tfplugin5.rs"));
}

/// Generated protobuf code for Terraform Plugin Protocol v6.9
pub mod tfplugin6_9 {
    include!(concat!(env!("OUT_DIR"), "/tfplugin6.rs"));
}

/// Protocol version 5 client
pub use tfplugin5_9::provider_client::ProviderClient as ProviderServiceClientV5;

/// Protocol version 6 client  
pub use tfplugin6_9::provider_client::ProviderClient as ProviderServiceClientV6;

/// Re-export version 6 as default for backwards compatibility
pub use ProviderServiceClientV6 as ProviderServiceClient;

/// Re-export all types from both versions
pub use tfplugin5_9 as v5;
pub use tfplugin6_9 as v6;
