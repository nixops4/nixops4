mod apply;
mod eval_client;
mod provider;

use anyhow::Result;
use clap::{CommandFactory as _, Parser, Subcommand};
use eval_client::EvalClient;
use nixops4_core;
use nixops4_core::eval_api::{AssignRequest, EvalRequest, FlakeRequest, FlakeType, Id};
use std::process::exit;

fn main() {
    let args = Args::parse();
    handle_result(run_args(args));
}

fn run_args(args: Args) -> Result<()> {
    match &args.command {
        Commands::Apply(subargs) => apply::apply(&args.options, subargs),
        Commands::Deployments(sub) => match sub {
            Deployments::List {} => deployments_list(&args.options),
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

fn to_eval_options(options: &Options) -> eval_client::Options {
    eval_client::Options {
        verbose: options.verbose,
    }
}

/// Convenience function that sets up an evaluator with a flake, asynchronously with regard to evaluation.
fn with_flake<T>(
    options: &Options,
    f: impl FnOnce(&mut EvalClient, Id<FlakeType>) -> Result<T>,
) -> Result<T> {
    EvalClient::with(&to_eval_options(options), |mut c| {
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

fn deployments_list(options: &Options) -> Result<()> {
    with_flake(options, |c, flake_id| {
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
    #[arg(short, long, global = true, default_value = "false")]
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
    #[command()]
    Apply(apply::Args),

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
