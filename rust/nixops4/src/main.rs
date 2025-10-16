mod apply;
mod control;
mod dump_state;
mod eval_client;
mod import;
mod interrupt;
mod logging;
mod provider;
mod state;
mod work;

use anyhow::{bail, Result};
use clap::{ColorChoice, CommandFactory as _, Parser, Subcommand};
use interrupt::{set_up_process_interrupt_handler, InterruptState};
use nixops4_core::eval_api::{
    AssignRequest, EvalRequest, EvalResponse, FlakeRequest, QueryResponseValue,
};
use std::process::exit;

fn main() {
    let interrupt_state = set_up_process_interrupt_handler();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to initialize tokio runtime");
    rt.block_on(async {
        let args = Args::parse();
        handle_result(run_args(&interrupt_state, args).await);
    })
}

async fn run_args(interrupt_state: &InterruptState, args: Args) -> Result<()> {
    match &args.command {
        Commands::Apply(subargs) => {
            let mut logging = set_up_logging(interrupt_state, &args)?;
            apply::apply(interrupt_state, &args.options, subargs).await?;
            logging.tear_down()?;
            Ok(())
        }
        Commands::Deployments(sub) => {
            match sub {
                Deployments::List {} => {
                    let mut logging = set_up_logging(interrupt_state, &args)?;
                    let deployments = deployments_list(&args.options).await?;
                    logging.tear_down()?;
                    for d in deployments {
                        println!("{}", d);
                    }
                }
            };
            Ok(())
        }
        Commands::Import(subargs) => {
            let mut logging = set_up_logging(interrupt_state, &args)?;

            let state = import::import_resources(interrupt_state, &args.options, subargs).await;
            logging.tear_down()?;
            println!("{:?}", state);
            Ok(())
        }
        Commands::State(sub) => match sub {
            State::Dump {
                resource_path,
                deployment,
            } => {
                let mut logging = set_up_logging(interrupt_state, &args)?;
                let state = dump_state::dump_state(
                    interrupt_state,
                    &args.options,
                    resource_path,
                    deployment,
                )
                .await?;
                logging.tear_down()?;
                println!("{}", state);
                Ok(())
            }
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
            clap_complete::generate(*shell, &mut cmd, "nixops4", &mut std::io::stdout());
            Ok(())
        }
    }
}

fn determine_color(choice: ColorChoice) -> bool {
    match choice {
        ColorChoice::Auto => nix::unistd::isatty(nix::libc::STDERR_FILENO).unwrap_or(false),
        ColorChoice::Always => true,
        ColorChoice::Never => false,
    }
}

fn determine_interactive(options: &Options) -> bool {
    match (options.interactive, options.no_interactive) {
        (true, false) => true,
        (false, true) => false,
        // (true, true) is ambiguous and already rejected by clap
        _ => nix::unistd::isatty(nix::libc::STDIN_FILENO).unwrap_or(false),
    }
}

fn set_up_logging(
    interrupt_state: &InterruptState,
    args: &Args,
) -> Result<Box<dyn logging::Frontend>> {
    let color = determine_color(args.options.color);
    let interactive = determine_interactive(&args.options);
    logging::set_up(
        interrupt_state,
        logging::Options {
            verbose: args.options.verbose,
            color,
            interactive,
        },
    )
}

fn to_eval_options(options: &Options) -> eval_client::Options {
    eval_client::Options {
        verbose: options.verbose,
        show_trace: options.show_trace,
        flake_input_overrides: options
            .override_input
            .chunks(2)
            .map(|pair| (pair[0].to_string(), pair[1].to_string()))
            .collect(),
    }
}

// TODO: clean up, unify with apply infrastructure
async fn deployments_list(options: &Options) -> Result<Vec<String>> {
    let eval_options = to_eval_options(options);
    let eval_options_2 = eval_options.clone();
    eval_client::EvalSender::with(&eval_options, |s, mut r| async move {
        let flake_id = s.next_id();
        // TODO: use better file path string type more
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        s.send(&EvalRequest::LoadFlake(AssignRequest {
            assign_to: flake_id,
            payload: FlakeRequest {
                abspath: cwd,
                input_overrides: eval_options_2.flake_input_overrides.clone(),
            },
        }))
        .await?;

        s.query(s.next_id(), &EvalRequest::ListDeployments, flake_id)
            .await?;

        let deployments = loop {
            match r.recv().await {
                None => {
                    bail!("Error: no response from evaluator");
                }
                Some(EvalResponse::Error(_id, e)) => {
                    bail!("Error: {}", e);
                }
                Some(EvalResponse::QueryResponse(
                    _,
                    QueryResponseValue::ListDeployments((_id, deployments)),
                )) => {
                    break deployments.iter().cloned().collect::<Vec<_>>();
                }
                Some(EvalResponse::QueryResponse(_, _)) => {
                    // Ignore other query responses
                }
                Some(EvalResponse::TracingEvent(_value)) => {
                    // Already handled in an EvalSender::with thread => ignore
                }
            }
        };
        Ok(deployments)
    })
    .await
}

fn handle_result(r: Result<()>) {
    match r {
        Ok(()) => {}
        Err(e) => {
            eprintln!("nixops4 error: {:?}", e);
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

    #[arg(long, global = true, default_value_t = ColorChoice::Auto)]
    color: ColorChoice,

    #[arg(long, global = true, default_value_t = false)]
    interactive: bool,

    #[arg(
        long,
        global = true,
        default_value_t = false,
        conflicts_with = "interactive"
    )]
    no_interactive: bool,

    #[arg(long, global = true, default_value_t = false)]
    show_trace: bool,

    /// Temporarily use a different flake input
    // will be post-processed to pair them up
    #[arg(long, num_args = 2, value_names = &["INPUT_ATTR_PATH", "FLAKE_REF"], global = true)]
    override_input: Vec<String>,
}

#[derive(Subcommand, Debug)]
enum Deployments {
    /// List the deployments based on the expressions in the flake
    List {},
}

#[derive(Subcommand, Debug)]
enum State {
    /// Dump the resolved deployment state for a resource path
    Dump {
        /// Resource path to dump state for
        resource_path: String,
        /// Deployment name
        #[arg(short, long, default_value = "default")]
        deployment: String,
    },
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Apply changes so that the resources are in the desired state
    #[command()]
    Apply(apply::Args),

    /// Commands that operate on all deployments
    #[command(subcommand)]
    Deployments(Deployments),

    /// Commands that operate on state
    #[command(subcommand)]
    State(State),

    /// Import the current state of the resource
    #[command()]
    Import(import::Args),

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
