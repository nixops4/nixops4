use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};

use crate::{interrupt::InterruptState, provider};
use crate::{with_flake, Options};
use anyhow::{bail, Result};
use nixops4_core::eval_api::{
    AssignRequest, DeploymentRequest, EvalRequest, EvalResponse, Id, NamedProperty, Property,
    QueryRequest, QueryResponseValue, ResourceInputState, ResourceRequest, ResourceType,
};
use nixops4_resource_runner::{ResourceProviderClient, ResourceProviderConfig};
use serde_json::Value;
use tracing::info_span;

#[derive(clap::Parser, Debug)]
pub(crate) struct Args {
    #[arg(default_value = "default")]
    deployment: String,
}

/// Run the `apply` command.
pub(crate) fn apply(
    interrupt_state: &InterruptState,
    options: &Options, /* global options; apply options tbd, extra param */
    args: &Args,
) -> Result<()> {
    with_flake(options, |c, flake_id| {
        let deployment_id = c.next_id();
        c.send(&EvalRequest::LoadDeployment(AssignRequest {
            assign_to: deployment_id,
            payload: DeploymentRequest {
                flake: flake_id,
                name: args.deployment.to_string(),
            },
        }))?;
        let resources_list_id = c.query(EvalRequest::ListResources, deployment_id)?;
        let resources = c.receive_until(|client, _resp| {
            client.check_error(flake_id)?;
            client.check_error(deployment_id)?;
            client.check_error(resources_list_id)?;
            Ok(client.get_resources(deployment_id).cloned())
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
            c.query(EvalRequest::GetResource, *id)?;
            // TODO: check for errors on this id
            c.query(EvalRequest::ListResourceInputs, *id)?;
        }
        let resource_ids_to_names: BTreeMap<Id<ResourceType>, String> =
            resource_ids.iter().map(|(k, v)| (*v, k.clone())).collect();
        let resource_ids_clone = resource_ids.clone();
        // key: blocking property, value: blocked properties
        let resources_blocked: Mutex<BTreeMap<Property, BTreeSet<Property>>> =
            Mutex::new(BTreeMap::new());
        let resources_outputs: Mutex<BTreeMap<Id<ResourceType>, BTreeMap<String, Value>>> =
            Mutex::new(BTreeMap::new());
        let resource_inputs = Mutex::new(BTreeMap::new());
        let resource_input_values = Mutex::new(BTreeMap::new());
        let resource_provider_info = Mutex::new(BTreeMap::new());

        let (resource_inputs, resource_outputs, resource_input_values) = {
            c.receive_until(move |client, resp| {
                // TODO: stop asynchronously
                // TODO: when concurrent track critical tasks and wait for them
                interrupt_state.check_interrupted()?;
                match resp {
                    EvalResponse::Error(id, e) => {
                        if options.verbose {
                            eprintln!("Error on id {}: {}", id.num(), e);
                        }
                        bail!("Error during evaluation: {}", e);
                    }
                    EvalResponse::QueryResponse(_id, payload) => match payload {
                        QueryResponseValue::ListResourceInputs((res, input_names)) => {
                            resource_inputs
                                .lock()
                                .unwrap()
                                .insert(*res, input_names.clone());
                            for input_name in input_names {
                                let input_id = client.next_id();
                                client.send(&EvalRequest::GetResourceInput(QueryRequest::new(
                                    input_id,
                                    Property {
                                        resource: *res,
                                        name: input_name.clone(),
                                    },
                                )))?;
                            }
                        }
                        QueryResponseValue::ListDeployments(_) => {}
                        QueryResponseValue::ListResources(_) => todo!(),
                        QueryResponseValue::ResourceProviderInfo(info) => {
                            resource_provider_info
                                .lock()
                                .unwrap()
                                .insert(info.id, info.clone());
                        }

                        QueryResponseValue::ResourceInputState((_property, st)) => match st {
                            ResourceInputState::ResourceInputValue((prop, value)) => {
                                // Store it
                                resource_input_values
                                    .lock()
                                    .unwrap()
                                    .insert(prop.clone(), value.clone());

                                // Is the resource ready to be created?
                                let this_resource_inputs = {
                                    let resource_inputs = resource_inputs.lock().unwrap();
                                    resource_inputs.get(&prop.resource).unwrap().clone()
                                };
                                {
                                    let resource_input_values =
                                        resource_input_values.lock().unwrap();
                                    let mut inputs = BTreeMap::new();
                                    let is_complete =
                                        this_resource_inputs.iter().all(|input_name| {
                                            let input_prop = Property {
                                                resource: prop.resource,
                                                name: input_name.clone(),
                                            };
                                            if let Some(value) =
                                                resource_input_values.get(&input_prop)
                                            {
                                                inputs.insert(input_name.clone(), value.clone());
                                                true
                                            } else {
                                                false
                                            }
                                        });

                                    if options.verbose {
                                        eprintln!("Resource complete: {}", is_complete);
                                        eprintln!("Resource inputs: {:?}", inputs);
                                    }

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

                                            let resource_name = {
                                                resource_ids_to_names
                                                    .get(&prop.resource)
                                                    .unwrap()
                                                    .clone()
                                            };

                                            let span = info_span!(
                                                "creating resource",
                                                name = resource_name
                                            );

                                            if options.verbose {
                                                eprintln!(
                                                    "Resource details for {}: {:?}",
                                                    resource_name, provider_info
                                                );
                                            }

                                            let provider_argv =
                                                provider::parse_provider(&provider_info.provider)?;
                                            // Run the provider
                                            let provider = ResourceProviderClient::new(
                                                ResourceProviderConfig {
                                                    provider_executable: provider_argv.command,
                                                    provider_args: provider_argv.args,
                                                },
                                            );
                                            let outputs = provider.create(
                                                provider_info.resource_type.as_str(),
                                                &inputs,
                                            )?;

                                            drop(span);

                                            if options.verbose {
                                                eprintln!("Resource outputs: {:?}", outputs);
                                            }

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
                                                        .keys()
                                                        .flat_map(|k| {
                                                            let blocker_property = Property {
                                                                resource: blocker_resource,
                                                                name: k.clone(),
                                                            };
                                                            resources_blocked
                                                                .get(&blocker_property)
                                                                .unwrap_or(&BTreeSet::new())
                                                                .clone()
                                                        })
                                                        .collect()
                                                };
                                                for dependent_property in dependents.iter() {
                                                    let req_id = client.next_id();
                                                    client.send(&EvalRequest::GetResourceInput(
                                                        QueryRequest::new(
                                                            req_id,
                                                            dependent_property.clone(),
                                                        ),
                                                    ))?;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            ResourceInputState::ResourceInputDependency(dep) => {
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
                                        // TODO: handle errors on _req_id
                                        let _req_id = client.query(
                                            EvalRequest::GetResourceInput,
                                            Property {
                                                resource: dep.dependent.resource,
                                                name: dep.dependent.name.clone(),
                                            },
                                        )?;
                                    }
                                    None => {
                                        let mut resources_blocked =
                                            resources_blocked.lock().unwrap();
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
                        },
                    },
                    EvalResponse::TracingEvent(_) => {
                        // already handled in EvalClient
                    }
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

        if options.verbose {
            eprintln!();
            eprintln!("Done!");
        }
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
                    eprintln!("  - input {}: {}", input, indented_json(input_value));
                }
            }
            {
                let outputs = resource_outputs.get(&resource_id).unwrap();
                for (k, v) in outputs.iter() {
                    eprintln!("  - output {}: {}", k, indented_json(v));
                }
            }
        }
        Ok(())
    })
}

fn indented_json(v: &Value) -> String {
    let s = serde_json::to_string_pretty(v).unwrap();
    s.replace("\n", "\n            ")
}
