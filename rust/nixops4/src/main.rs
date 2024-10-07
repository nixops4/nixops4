mod apply;
mod eval_client;

use anyhow::{bail, Result};
use clap::{CommandFactory as _, Parser, Subcommand};
use eval_client::EvalClient;
use nixops4_core;
use nixops4_core::eval_api::{AssignRequest, EvalRequest, FlakeRequest, FlakeType, Id};
use serde_json::Value;
use std::process::exit;

fn main() {
    let args = Args::parse();
    handle_result(run_args(args));
}

fn run_args(args: Args) -> Result<()> {
    match &args.command {
        Commands::Apply {} => apply::apply(args.options),
        Commands::Deployments(sub) => match sub {
            Deployments::List {} => deployments_list(),
        },
        Commands::GenerateMan => (|| {
            let cmd = Args::command();
            let man = clap_mangen::Man::new(cmd);
            let mut buffer: Vec<u8> = Default::default();
            man.render(&mut buffer)?;
            println!("{}", String::from_utf8(buffer)?);
            Ok(())
        })(),
        Commands::GenerateMarkdown => {
            let opts = clap_markdown::MarkdownOptions::new().show_footer(false);
            let markdown: String = clap_markdown::help_markdown_custom::<Args>(&opts);
            println!("{}", markdown);
            Ok(())
        }
        Commands::GenerateCompletion { shell } => {
            // TODO: remove the generate-* commands from the completion
            //       same problem in nixops4-resource-runner
            let mut cmd = Args::command();
            clap_complete::generate(shell.clone(), &mut cmd, "nixops4", &mut std::io::stdout());
            Ok(())
        }
    }
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

fn deployments_list() -> Result<()> {
    with_flake(|c, flake_id| {
        let deployments_id = c.query(EvalRequest::ListDeployments, flake_id)?;
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
    })
}

// TODO: rename to ProviderArgv?
#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
struct ProviderStdio {
    command: String,
    args: Vec<String>,
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

/// NixOps: manage resources declaratively
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,

    #[command(flatten)]
    options: Options,
}

#[derive(Parser, Debug, Clone)]
struct Options {
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand, Debug)]
enum Deployments {
    /// List the deployments based on the expressions in the flake
    List {},
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Apply changes so that the resources are in the desired state
    Apply {},

    /// Commands that operate on all deployments
    #[command(subcommand)]
    Deployments(Deployments),

    /// Generate markdown documentation for nixops4-resource-runner
    #[command(hide = true)]
    GenerateMarkdown,

    /// Generate a manpage for nixops4-resource-runner
    #[command(hide = true)]
    GenerateMan,

    /// Generate shell completion for nixops4-resource-runner
    #[command(hide = true)]
    GenerateCompletion {
        /// The shell to generate completion for
        #[arg(long)]
        shell: clap_complete::Shell,
    },
}
