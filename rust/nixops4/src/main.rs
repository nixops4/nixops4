mod application;
mod apply;
mod complete;
mod control;
mod eval_client;
mod interrupt;
mod logging;
mod options;
mod provider;
mod state;
mod work;

use anyhow::{bail, Result};
use clap::{ColorChoice, CommandFactory as _, Parser, Subcommand};
use clap_complete::engine::ArgValueCompleter;
use clap_complete::env::CompleteEnv;
use interrupt::{set_up_process_interrupt_handler, InterruptState};
use nixops4_core::eval_api::ComponentPath;
use options::Options;
use work::{Goal, Outcome};

fn main() {
    // Handle shell completion if requested via environment.
    // Use .completer("nixops4") to emit the command name instead of the full path,
    // so that wrappers (like the Nix makeBinaryWrapper) work correctly.
    CompleteEnv::with_factory(Args::command)
        .completer("nixops4")
        .complete();

    let interrupt_state = set_up_process_interrupt_handler();

    let rt = application::runtime();
    rt.block_on(async {
        let args = Args::parse();
        application::handle_result(run_args(&interrupt_state, args).await);
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
                    let members =
                        members_list(interrupt_state, &args.options, path.as_deref()).await?;
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

/// List members at a given component path
async fn members_list(
    interrupt_state: &InterruptState,
    options: &Options,
    path: Option<&str>,
) -> Result<Vec<String>> {
    let target_path: ComponentPath = path.map_or(ComponentPath::root(), |s| s.parse().unwrap());

    // TODO: Support nested paths by traversing to the composite
    if !target_path.is_root() {
        bail!(
            "Listing members at nested paths is not yet implemented. Use root path (no argument)."
        );
    }

    application::with_eval(interrupt_state, options, |work_context, tasks| async move {
        let root_id = work_context.root_composite_id;

        // Use ListMembers goal without mutation capability (preview mode)
        let result = tasks
            .run(Goal::ListMembers(root_id, target_path.clone(), None))
            .await;

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

/// NixOps: manage resources declaratively
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,

    #[command(flatten)]
    options: Options,
}

#[derive(Subcommand, Debug)]
enum Members {
    /// List members at a component path (default: root)
    List {
        /// Component path (dot-separated, e.g., "production.database")
        #[arg(add = ArgValueCompleter::new(complete::component_path_completer_composite))]
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
}
