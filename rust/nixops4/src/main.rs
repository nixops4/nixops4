mod apply;
mod control;
mod eval_client;
mod interrupt;
mod logging;
mod provider;
mod state;
mod work;

use anyhow::{bail, Context, Result};
use clap::{ColorChoice, CommandFactory as _, Parser, Subcommand};
use interrupt::{set_up_process_interrupt_handler, InterruptState};
use nixops4_core::eval_api::{
    AssignRequest, ComponentPath, EvalRequest, EvalResponse, FlakeRequest, RootRequest,
};
use std::process::exit;
use std::sync::Arc;
use work::{Goal, Outcome, WorkContext};

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
        Commands::Members(sub) => {
            match sub {
                Members::List { path } => {
                    let mut logging = set_up_logging(interrupt_state, &args)?;
                    let members = members_list(&args.options, path.as_deref()).await?;
                    logging.tear_down()?;
                    for m in members {
                        println!("{}", m);
                    }
                }
            };
            Ok(())
        }
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
            .map(|pair| {
                assert!(
                    pair.len() == 2,
                    "override_input must have an even number of elements (clap num_args = 2)"
                );
                (pair[0].to_string(), pair[1].to_string())
            })
            .collect(),
    }
}

/// List members at a given component path
async fn members_list(options: &Options, path: Option<&str>) -> Result<Vec<String>> {
    // TODO: Support nested paths by traversing to the composite
    let target_path = path.map_or(ComponentPath::root(), |s| s.parse().unwrap());
    if !target_path.is_root() {
        bail!(
            "Listing members at nested paths is not yet implemented. Use root path (no argument)."
        );
    }

    let eval_options = to_eval_options(options);
    let eval_options_2 = eval_options.clone();
    eval_client::EvalSender::with(&eval_options, |s, mut r| async move {
        let flake_id = s.next_id();
        let cwd = std::env::current_dir()
            .context("getting current directory")?
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

        let root_id = s.next_id();
        s.send(&EvalRequest::LoadRoot(AssignRequest {
            assign_to: root_id,
            payload: RootRequest { flake: flake_id },
        }))
        .await?;

        // Set up work context with the root and use Preview goal to list members
        let work_context = WorkContext {
            root_composite_id: root_id,
            options: options.clone(),
            interrupt_state: interrupt::InterruptState::new(),
            eval_sender: s.clone(),
            state: Default::default(),
            id_subscriptions: pubsub_rs::Pubsub::new(),
        };

        let id_subscriptions = work_context.id_subscriptions.clone();
        let work_context = Arc::new(work_context);
        let tasks = control::task_tracker::TaskTracker::new(work_context.clone());

        // Spawn response handler
        let h: tokio::task::JoinHandle<Result<()>> = tokio::spawn(async move {
            while let Some(msg) = r.recv().await {
                match &msg {
                    EvalResponse::Error(id, _) => {
                        id_subscriptions.publish(id.num(), msg).await;
                    }
                    EvalResponse::QueryResponse(id, _) => {
                        id_subscriptions.publish(id.num(), msg).await;
                    }
                    EvalResponse::TracingEvent(_value) => {
                        // Already handled in an EvalSender::with thread => ignore
                    }
                }
            }
            Ok(())
        });

        // Use ListMembers goal without mutation capability (preview mode)
        let result = tasks
            .run(Goal::ListMembers(root_id, target_path.clone(), None))
            .await;

        s.close().await;
        h.await??;

        // Extract member names from the result
        match result.as_ref() {
            Ok(Outcome::MembersListed(Ok(names))) => Ok(names.clone()),
            Ok(Outcome::MembersListed(Err(preview_item))) => {
                bail!(
                    "Cannot list members at '{}': blocked by structural dependency: {}",
                    target_path,
                    preview_item
                )
            }
            Ok(other) => {
                bail!("Unexpected outcome from ListMembers: {:?}", other)
            }
            Err(e) => bail!("Failed to list members at '{}': {}", target_path, e),
        }
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
enum Members {
    /// List members at a component path (default: root)
    List {
        /// Component path (dot-separated, e.g., "production.database")
        #[arg()]
        path: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Apply changes so that the resources are in the desired state.
    ///
    /// When paths are specified, all members below those paths are applied,
    /// as well as any resources they transitively depend on.
    #[command()]
    Apply(apply::Args),

    /// Commands that operate on component members
    #[command(subcommand)]
    Members(Members),

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
