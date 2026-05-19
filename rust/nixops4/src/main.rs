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

use anyhow::{bail, Context as _, Result};
use clap::{ColorChoice, CommandFactory as _, Parser, Subcommand};
use clap_complete::engine::ArgValueCompleter;
use clap_complete::env::CompleteEnv;
use interrupt::{set_up_process_interrupt_handler, InterruptState};
use nixops4_core::eval_api::{ComponentHandle, ComponentPath};
use options::Options;
use work::{clone_anyhow_from_arc, Goal, Outcome};

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
                        members_list(interrupt_state, &args.options, path.clone()).await?;
                    logging.tear_down()?;
                    for m in members {
                        println!("{}", m);
                    }
                }
            };
            Ok(())
        }
        Commands::State(sub) => {
            match sub {
                State::Dump { path } => {
                    let mut logging = set_up_logging(interrupt_state, &args)?;
                    let json = dump_state(interrupt_state, &args.options, path.clone()).await?;
                    logging.tear_down()?;
                    println!("{}", json);
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
    target_path: ComponentPath,
) -> Result<Vec<String>> {
    application::with_eval(
        interrupt_state,
        options,
        |_work_context, tasks| async move {
            let composite_id = work::resolve_composite_path(&tasks, target_path.clone())
                .await
                .context(format!(
                    "Failed to resolve path '{}' for members list",
                    target_path
                ))?;

            // Use ListMembers goal without mutation capability (preview mode)
            let result = tasks
                .run(Goal::ListMembers(composite_id, target_path.clone(), None))
                .await;

            // Extract member names from the result
            match result.as_ref() {
                Ok(Outcome::MembersListed(Ok(names))) => Ok(names.clone()),
                Ok(Outcome::MembersListed(Err(dep))) => {
                    bail!(
                        "Cannot list members at '{}': blocked by structural dependency (depends on {}.{})",
                        target_path,
                        dep.depends_on.resource,
                        dep.depends_on.name,
                    )
                }
                Ok(other) => {
                    bail!("Unexpected outcome from ListMembers: {:?}", other)
                }
                Err(e) => bail!("Failed to list members at '{}': {}", target_path, e),
            }
        },
    )
    .await
}

/// Dump the state stored in a state-providing resource as JSON.
///
/// Reads the state without applying any resources. Data may be stale.
async fn dump_state(
    interrupt_state: &InterruptState,
    options: &Options,
    resource_path: ComponentPath,
) -> Result<String> {
    application::with_eval(
        interrupt_state,
        options,
        |_work_context, tasks| async move {
            // Split path into parent composite + resource name
            let (parent_path, name) = resource_path.parent().ok_or_else(|| {
                anyhow::anyhow!(
                    "empty path refers to the root deployment, not a resource; \
                     specify a state-providing resource, e.g. 'myDeployment.state'"
                )
            })?;

            // Resolve parent composite
            let parent_id = work::resolve_composite_path(&tasks, parent_path)
                .await
                .context(format!("Failed to dump state stored in {}", resource_path))?;

            // Load the member and verify it's a resource
            let load_result = tasks
                .run(Goal::LoadMember(parent_id, name.to_string(), None))
                .await;
            let resource_id = match load_result.as_ref() {
                Ok(Outcome::MemberLoaded(Ok(ComponentHandle::Resource(id)))) => *id,
                Ok(Outcome::MemberLoaded(Ok(ComponentHandle::Composite(_)))) => {
                    bail!(
                        "'{}' is a composite (deployment), not an individual resource. \
                         Specify a state-providing resource, e.g., '{}.state'.",
                        resource_path,
                        resource_path
                    )
                }
                Ok(Outcome::MemberLoaded(Err(dep))) => {
                    // TODO: resolve dependencies in read-only mode via state
                    //       (https://github.com/nixops4/nixops4/issues/164)
                    bail!(
                        "Cannot load '{}': blocked by structural dependency (depends on {}.{})",
                        resource_path,
                        dep.depends_on.resource,
                        dep.depends_on.name,
                    )
                }
                Ok(other) => bail!("Unexpected outcome from LoadMember: {:?}", other),
                Err(e) => {
                    return Err(clone_anyhow_from_arc(e))
                        .context(format!("Failed to dump state stored in {}", resource_path))
                }
            };

            let run_result = tasks
                .run(Goal::RunState(
                    resource_id,
                    resource_path.clone(),
                    None, /* read-only: resolves inputs without applying */
                ))
                .await;
            let state_handle = match run_result.as_ref() {
                Ok(Outcome::RunState(handle)) => handle.clone(),
                Ok(other) => bail!("Unexpected outcome from RunState: {:?}", other),
                Err(e) => {
                    return Err(clone_anyhow_from_arc(e)).context(format!(
                        "'{}' could not be read as a state provider",
                        resource_path
                    ))
                }
            };

            // Read current state and serialize to JSON
            let state = state_handle.current.lock().await;
            let json = serde_json::to_string_pretty(&*state)
                .map_err(|e| anyhow::anyhow!("Failed to serialize state: {}", e))?;
            Ok(json)
        },
    )
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
    /// List members at a component path (default: root).
    ///
    /// This is a read-only command that does not create or modify resources.
    /// If the member set depends on a resource output (a structural dependency),
    /// the command fails instead of showing incomplete results.
    List {
        /// Component path (dot-separated, e.g., "production.database").
        /// Must resolve to a composite (deployment), not a resource.
        // `default_value = ""` because `default_value_t = ComponentPath::root()`
        // round-trips through Display+FromStr, and `Display` of root is `(root)`,
        // which then parses back to a single-segment path `["(root)"]`. Empty
        // string is the right on-the-wire form and FromStr("") returns root.
        // `hide_default_value = true` because rendering the empty default as
        // `[default: ]` in --help reads oddly; the enum docstring already says
        // "(default: root)".
        #[arg(
            default_value = "",
            hide_default_value = true,
            add = ArgValueCompleter::new(complete::component_path_completer_composite),
        )]
        path: ComponentPath,
    },
}

#[derive(Subcommand, Debug)]
enum State {
    /// Dump the current state stored in a state-providing resource as JSON.
    ///
    /// This is a read-only command that does not create or modify resources.
    /// If the resource path cannot be resolved without applying other resources
    /// (e.g., it depends on another resource's output), the command fails.
    ///
    /// Plumbing: the output is the raw state contents, which may contain
    /// unencrypted secrets. Do not paste it into bug reports or send it over
    /// untrusted channels without redacting first.
    Dump {
        /// Path to a state-providing resource (dot-separated, e.g., "myDeployment.state")
        #[arg(add = ArgValueCompleter::new(complete::component_path_completer_resource))]
        path: ComponentPath,
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

    /// Commands that operate on deployment state
    #[command(subcommand)]
    State(State),

    /// Generate markdown documentation for nixops4-resource-runner
    #[command(hide = true)]
    GenerateMarkdown,

    /// Generate a manpage for nixops4-resource-runner
    #[command(hide = true)]
    GenerateMan,
}
