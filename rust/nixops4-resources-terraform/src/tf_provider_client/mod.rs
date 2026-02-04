//! Terraform Provider Client
//!
//! This module implements a client for communicating with Terraform providers
//! using the go-plugin protocol and gRPC.

mod client;
pub mod grpc;

pub use client::ProviderClient;
pub use client::ProviderSchema;
