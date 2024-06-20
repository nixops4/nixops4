mod eval_client;

use anyhow::Context;
use anyhow::{bail, Result};
use clap::Command;
use eval_client::EvalClient;
use nixops4_core;
use nixops4_core::eval_api::{
    AssignRequest, DeploymentRequest, EvalRequest, EvalResponse, FlakeRequest, FlakeType, Id,
    NamedProperty, Property, ResourceProviderInfo, ResourceRequest, ResourceType, SimpleRequest,
};
use nixops4_resource::schema::v0::{CreateResourceRequest, CreateResourceResponse};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;
use std::io::Write;
use std::process::exit;
use std::sync::Mutex;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn root_command() -> Command {
    Command::new("nixops4")
        .version(VERSION)
        .about("Deploy with Nix and manage resources declaratively")
        .subcommand(
            Command::new("deployments").subcommand(Command::new("list").about("List deployments")),
        )
        .subcommand(Command::new("apply").about("Make it so"))
}

/// Convenience function that sets up an evaluator with a flake, asynchronously with regard to evaluation.
fn with_flake<T>(f: impl FnOnce(&mut EvalClient, Id<FlakeType>) -> Result<T>) -> Result<T> {
    EvalClient::with(|mut c| {
        let flake_id = c.next_id();
        // TODO: use better file path string type more
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        c.send(&EvalRequest::LoadFlake(AssignRequest {
            assign_to: flake_id,
            payload: FlakeRequest { abspath: cwd },
        }))?;
        f(&mut c, flake_id)
    })
}

fn main() {
    let matches = root_command().get_matches();
    let r: Result<()> = match matches.subcommand() {
        Some(("deployments", sub_matches)) => {
            match sub_matches.subcommand() {
                Some(("list", _)) => with_flake(|c, flake_id| {
                    let deployments_id = c.next_id();
                    c.send(&EvalRequest::ListDeployments(SimpleRequest {
                        assign_to: deployments_id,
                        payload: flake_id,
                    }))?;
                    let deployments = c.receive_until(|client, _resp| {
                        client.check_error(flake_id)?;
                        client.check_error(deployments_id)?;
                        let x = client.get_deployments(flake_id);
                        Ok(x.map(|x| x.clone()))
                    })?;
                    for d in deployments {
                        println!("{}", d);
                    }
                    Ok(())
                }),
                Some((name, _)) => {
                    eprintln!("nixops4 internal error: unknown subcommand: {}", name);
                    exit(1);
                }
                None => {
                    // TODO: list instead?
                    eprintln!("nixops4 internal error: no subcommand given");
                    exit(1);
                }
            }
        }
        Some(("apply", _)) => with_flake(|c, flake_id| {
            let deployment_id = c.next_id();
            c.send(&EvalRequest::LoadDeployment(AssignRequest {
                assign_to: deployment_id,
                payload: DeploymentRequest {
                    flake: flake_id,
                    name: "default".to_string(),
                },
            }))?;
            let resources_list_id = c.next_id();
            c.send(&EvalRequest::ListResources(SimpleRequest {
                assign_to: resources_list_id,
                payload: deployment_id,
            }))?;
            let resources = c.receive_until(|client, _resp| {
                client.check_error(flake_id)?;
                client.check_error(deployment_id)?;
                client.check_error(resources_list_id)?;
                Ok(client.get_resources(deployment_id).map(|x| x.clone()))
            })?;
            if resources.is_empty() {
                eprintln!("Deployment contains no resources; nothing to apply.");
            } else {
                eprintln!("The following resources will be checked, created and/or updated:");
                for r in &resources {
                    eprintln!("  - {}", r);
                }
            }
            let resource_ids: BTreeMap<String, Id<ResourceType>> = resources
                .iter()
                .map(|name| (name.clone(), c.next_id()))
                .collect();
            for (r, id) in resource_ids.iter() {
                c.send(&EvalRequest::LoadResource(AssignRequest {
                    assign_to: *id,
                    payload: ResourceRequest {
                        deployment: deployment_id,
                        name: r.clone(),
                    },
                }))?;
                // TODO: check for errors on this id
                let get_resource_id = c.next_id();
                c.send(&EvalRequest::GetResource(SimpleRequest {
                    assign_to: get_resource_id,
                    payload: *id,
                }))?;
                // TODO: check for errors on this id
                c.send(&EvalRequest::ListResourceInputs(SimpleRequest {
                    assign_to: get_resource_id,
                    payload: *id,
                }))?;
            }
            let resource_ids_to_names: BTreeMap<Id<ResourceType>, String> =
                resource_ids.iter().map(|(k, v)| (*v, k.clone())).collect();
            let resource_ids_clone = resource_ids.clone();
            // Starting set of resources
            // let resources_to_eval: Vec<Id<ResourceType>> =
            //     resource_ids.values().map(Clone::clone).collect();

            // // key: blocking property, value: blocked properties
            let resources_blocked: Mutex<BTreeMap<Property, BTreeSet<Property>>> =
                Mutex::new(BTreeMap::new());
            let resources_outputs: Mutex<BTreeMap<Id<ResourceType>, BTreeMap<String, Value>>> =
                Mutex::new(BTreeMap::new());
            // let resource_state = Mutex::new(BTreeMap::new());
            let resource_inputs = Mutex::new(BTreeMap::new());
            let resource_input_values = Mutex::new(BTreeMap::new());
            let resource_provider_info = Mutex::new(BTreeMap::new());

            let (resource_inputs, resource_outputs, resource_input_values) = {
                c.receive_until(move |client, resp| {
                    match resp {
                        EvalResponse::Error(_id, e) => {
                            bail!("Error during evaluation: {}", e);
                        }
                        EvalResponse::ResourceInputs(res, input_names) => {
                            resource_inputs
                                .lock()
                                .unwrap()
                                .insert(*res, input_names.clone());
                            for input_name in input_names {
                                let input_id = client.next_id();
                                client.send(&EvalRequest::GetResourceInput(SimpleRequest {
                                    assign_to: input_id,
                                    payload: Property {
                                        resource: *res,
                                        name: input_name.clone(),
                                    },
                                }))?;
                            }
                        }
                        EvalResponse::ResourceProviderInfo(info) => {
                            resource_provider_info
                                .lock()
                                .unwrap()
                                .insert(info.id.clone(), info.clone());
                        }
                        EvalResponse::ResourceInputDependency(dep) => {
                            // We might have learned the value after we've asked to evaluate this,
                            // so we need to check if we have the value now.
                            let resource_output_opt = {
                                let resources_outputs = resources_outputs.lock().unwrap();
                                let resource_id =
                                    resource_ids.get(&dep.dependency.resource).unwrap();
                                resources_outputs.get(resource_id).cloned()
                            };
                            match resource_output_opt {
                                Some(_) => {
                                    // Have have already sent PutResourceOutput for this,
                                    // so all that's missing is the request to recompute the dependents

                                    // Trigger the dependent (TODO dedup?)
                                    let req_id = client.next_id();
                                    client.send(&EvalRequest::GetResourceInput(SimpleRequest {
                                        assign_to: req_id,
                                        payload: Property {
                                            resource: dep.dependent.resource,
                                            name: dep.dependent.name.clone(),
                                        },
                                    }))?;
                                }
                                None => {
                                    let mut resources_blocked = resources_blocked.lock().unwrap();
                                    let dependency =
                                        resource_ids.get(&dep.dependency.resource).unwrap();
                                    resources_blocked
                                        .entry(Property {
                                            resource: *dependency,
                                            name: dep.dependency.name.clone(),
                                        })
                                        .or_default()
                                        .insert(dep.dependent.clone());
                                }
                            }
                        }
                        EvalResponse::ResourceInputValue(prop, value) => {
                            // Store it
                            resource_input_values
                                .lock()
                                .unwrap()
                                .insert(prop.clone(), value.clone());

                            // // Trigger dependents
                            // {
                            //     let dependents: BTreeSet<Property> = {
                            //         resources_blocked
                            //             .lock()
                            //             .unwrap()
                            //             .entry(prop.clone())
                            //             .or_default()
                            //             .clone()
                            //     };
                            //     for dependent_property in dependents.iter() {
                            //         let req_id = client.next_id();
                            //         client.send(&EvalRequest::GetResourceInput(SimpleRequest {
                            //             assign_to: req_id,
                            //             payload: dependent_property.clone(),
                            //         }))?;
                            //     }
                            // }

                            // Is the resource ready to be created?
                            let this_resource_inputs = {
                                let resource_inputs = resource_inputs.lock().unwrap();
                                resource_inputs.get(&prop.resource).unwrap().clone()
                            };
                            {
                                let resource_input_values = resource_input_values.lock().unwrap();
                                let mut inputs = BTreeMap::new();
                                let is_complete = this_resource_inputs.iter().all(|input_name| {
                                    let input_prop = Property {
                                        resource: prop.resource,
                                        name: input_name.clone(),
                                    };
                                    if let Some(value) = resource_input_values.get(&input_prop) {
                                        inputs.insert(input_name.clone(), value.clone());
                                        true
                                    } else {
                                        false
                                    }
                                });

                                eprintln!("Resource complete: {}", is_complete);
                                eprintln!("Resource inputs: {:?}", inputs);

                                if is_complete {
                                    if resources_outputs
                                        .lock()
                                        .unwrap()
                                        .get(&prop.resource)
                                        .is_none()
                                    {
                                        let provider_info = {
                                            let resource_provider_info =
                                                resource_provider_info.lock().unwrap();
                                            resource_provider_info
                                                .get(&prop.resource)
                                                .unwrap()
                                                .clone()
                                        };

                                        eprintln!("Creating resource: {:?}", provider_info);

                                        // Run the provider
                                        let outputs = create(&provider_info, inputs)?;

                                        eprintln!("Resource outputs: {:?}", outputs);

                                        resources_outputs
                                            .lock()
                                            .unwrap()
                                            .insert(prop.resource, outputs.clone());

                                        // Push the outputs to the evaluator
                                        for (output_name, output_value) in outputs.iter() {
                                            let resource_name = {
                                                resource_ids_to_names
                                                    .get(&prop.resource)
                                                    .unwrap()
                                                    .clone()
                                            };
                                            let output_prop = NamedProperty {
                                                resource: resource_name,
                                                name: output_name.clone(),
                                            };
                                            client.send(&EvalRequest::PutResourceOutput(
                                                output_prop,
                                                output_value.clone(),
                                            ))?;
                                        }

                                        // Trigger dependents
                                        {
                                            let dependents: BTreeSet<Property> = {
                                                let resources_blocked =
                                                    resources_blocked.lock().unwrap();
                                                let blocker_resource = prop.resource;
                                                outputs
                                                    .iter()
                                                    .map(|(k, _)| {
                                                        let blocker_property = Property {
                                                            resource: blocker_resource,
                                                            name: k.clone(),
                                                        };
                                                        resources_blocked
                                                            .get(&blocker_property)
                                                            .unwrap_or(&BTreeSet::new())
                                                            .clone()
                                                    })
                                                    .flatten()
                                                    .collect()
                                            };
                                            for dependent_property in dependents.iter() {
                                                let req_id = client.next_id();
                                                client.send(&EvalRequest::GetResourceInput(
                                                    SimpleRequest {
                                                        assign_to: req_id,
                                                        payload: dependent_property.clone(),
                                                    },
                                                ))?;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                    for id in resource_ids.values() {
                        client.check_error(*id)?;
                    }

                    // Are we done?
                    {
                        if resources.len() == resources_outputs.lock().unwrap().len() {
                            let resources_inputs = resource_inputs.lock().unwrap();
                            let resources_outputs = resources_outputs.lock().unwrap();
                            Ok(Some((
                                resources_inputs.clone(),
                                resources_outputs.clone(),
                                resource_input_values.lock().unwrap().clone(),
                            )))
                        } else {
                            Ok(None)
                        }
                    }
                })?
            };
            // for (r, id) in resource_ids.iter() {
            //     let resource = c.get_resource(*id).unwrap();
            //     eprintln!("Resource {}: {}", r, resource);
            // }
            eprintln!("Done!");
            eprintln!("The following resources were created:");
            for (resource_name, resource_id) in resource_ids_clone {
                eprintln!("Resource {}:", resource_name);
                {
                    let inputs = resource_inputs.get(&resource_id).unwrap();
                    for input in inputs.iter() {
                        let property = Property {
                            resource: resource_id,
                            name: input.clone(),
                        };
                        let input_value = resource_input_values.get(&property).unwrap();
                        eprintln!("  - input {}: {:?}", input, input_value);
                    }
                }
                {
                    let outputs = resource_outputs.get(&resource_id).unwrap();
                    for (k, v) in outputs.iter() {
                        eprintln!("  - output {}: {:?}", k, v);
                    }
                }
            }
            Ok(())
        }),
        Some((name, _)) => {
            eprintln!("nixops4 internal error: unknown subcommand: {}", name);
            exit(1);
        }
        None => {
            root_command().print_help().unwrap();
            eprintln!("\nNo subcommand given");
            exit(1);
        }
    };
    handle_result(r);
}

#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
struct ProviderStdio {
    command: String,
    args: Vec<String>,
}

fn create(
    provider_info: &ResourceProviderInfo,
    inputs: BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>> {
    let provider: ProviderStdio = parse_provider(&provider_info.provider)?;

    let stdin_str = {
        let req = CreateResourceRequest {
            input_properties: inputs.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            type_: provider_info.resource_type.clone(),
        };
        serde_json::to_string(&req).unwrap()
    };

    let command = &provider.command;

    let mut process = std::process::Command::new(provider.command.clone())
        .args(provider.args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .with_context(|| format!("Could not spawn provider process {}", command))?;

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

fn parse_provider(provider_value: &Value) -> Result<ProviderStdio> {
    let provider = provider_value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Provider must be an object"))?;
    let type_ = provider
        .get("type")
        .ok_or_else(|| anyhow::anyhow!("Provider must have a type"))?;
    let type_ = type_
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Provider type must be a string"))?;
    match type_ {
        "stdio" => serde_json::from_value(provider_value.clone())
            .map_err(|e| e.into())
            .map(|x: ProviderStdio| x.clone()),
        _ => {
            bail!("Unknown provider type: {}", type_);
        }
    }
}

fn handle_result(r: Result<()>) {
    match r {
        Ok(()) => {}
        Err(e) => {
            eprintln!("nixops4 error: {}, {}", e.root_cause(), e.to_string());
            exit(1);
        }
    }
}
