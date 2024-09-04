use std::{
    collections::BTreeMap,
    io::{BufRead, Write},
};

use anyhow::{Context, Result};
use nixops4_resource::schema::v0::{CreateResourceRequest, CreateResourceResponse};
use serde_json::Value;

pub struct ResourceProviderConfig {
    pub provider_executable: String,
    pub provider_args: Vec<String>,
}

pub struct ResourceProviderClient {
    provider_config: ResourceProviderConfig,
    // TODO: maintain a long-lived process
}

impl ResourceProviderClient {
    pub fn new(provider_config: ResourceProviderConfig) -> Self {
        ResourceProviderClient { provider_config }
    }

    pub fn create(
        &self,
        type_: &str,
        inputs: &BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Value>> {
        let stdin_str = {
            let req = CreateResourceRequest {
                input_properties: inputs.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                type_: type_.to_string(),
            };
            serde_json::to_string(&req).unwrap()
        };

        let mut process =
            std::process::Command::new(self.provider_config.provider_executable.clone())
                .args(self.provider_config.provider_args.clone())
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::inherit())
                .spawn()
                .with_context(|| {
                    format!(
                        "Could not spawn provider process {}",
                        self.provider_config.provider_executable
                    )
                })?;

        // Get the handles
        let (response, mut process) = {
            let child_in = process.stdin.as_mut().unwrap();
            let child_out = process.stdout.as_mut().unwrap();
            let mut child_reader = std::io::BufReader::new(child_out);

            // Write the request
            child_in.write_all(stdin_str.as_bytes()).unwrap();
            child_in.write_all(b"\n").unwrap();
            child_in.flush().unwrap();

            // Read the response
            let response: CreateResourceResponse = {
                let mut response = String::new();
                child_reader.read_line(&mut response).unwrap();
                serde_json::from_str(&response)?
            };
            (response, process)
            // This closes stdin
        };

        // Wait for the process to finish
        process.wait()?;

        Ok(response
            .output_properties
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }
}
