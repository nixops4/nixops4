mod eval_client;

use nixops4_core;
use anyhow::Result;
use clap::Command;
use eval_client::EvalClient;
use nixops4_core::eval_api::{
    AssignRequest, EvalRequest, FlakeRequest, FlakeType, Id, SimpleRequest,
};
use std::process::exit;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn root_command() -> Command {
    Command::new("nixops4")
        .version(VERSION)
        .about("Deploy with Nix and manage resources declaratively")
        .subcommand(
            Command::new("deployments").subcommand(Command::new("list").about("List deployments")),
        )
}

/** Convenience function that sets up an evaluator with a flake, asynchronously with regard to evaluation. */
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
                Some(("list", _)) => with_flake(|mut c, flake_id| {
                    let deployments_id = c.next_id();
                    c.send(&EvalRequest::ListDeployments(SimpleRequest {
                        assign_to: deployments_id,
                        payload: flake_id,
                    }))?;
                    let deployments = c.receive_until(|client| {
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

fn handle_result(r: Result<()>) {
    match r {
        Ok(()) => {}
        Err(e) => {
            eprintln!("nixops4 error: {}, {}", e.root_cause(), e.to_string());
            exit(1);
        }
    }
}
