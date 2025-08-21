use anyhow::{Context, Result};
use clap::{arg, CommandFactory};
use clap::{Parser, Subcommand};
use core::str;
use nixops4_resource_runner::{ResourceProviderClient, ResourceProviderConfig};

/// The nixops4-resource-runner executable
///
/// This is a separate executable because this functionality is not needed
/// during normal nixops4 operation, and it would pollute the shell autocompletion.
fn main() -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async_main())
}

async fn async_main() -> Result<()> {
    let args = Args::parse();

    match &args.command {
        Commands::Create {
            provider_exe,
            resource_type,
            input_properties_json,
            input_property_json,
            input_property_str,
            stateful,
        } => {
            // NOTE (loss of ordering):
            //
            // clap_derive appears incapable of preserving the order of flags,
            // as it rejects a Vec of enums that would allow for this. This
            // means that we can't tell which input property was specified last,
            // and so we can't make later inputs override earlier ones, as we
            // would like to do, mirroring Nix's `//` operator. Eventually this
            // may be supported, so we carve out this possibility by rejecting
            // duplicate inputs. Otherwise, this improvement would be a breaking
            // change.

            // Mutable map that is used for gathering all input properties.
            let mut inputs = match input_properties_json {
                Some(json_string) => serde_json::from_str(json_string.as_str())
                    .with_context(|| "failed to parse value of --inputs-json")?,
                None => serde_json::Map::new(),
            };

            for pair in input_property_json.chunks(2) {
                assert!(pair.len() == 2);
                let k = &pair[0];
                let v = &pair[1];
                if inputs.contains_key(k) {
                    // No overriding; see note "loss of ordering"
                    eprintln!("error: duplicate input: {}", k);
                    std::process::exit(1);
                }
                inputs.insert(
                    k.clone(),
                    serde_json::from_str(v.as_str())
                        .with_context(|| format!("failed to parse JSON value for input: {}", k))?,
                );
            }
            for pair in input_property_str.chunks(2) {
                assert!(pair.len() == 2);
                let k = &pair[0];
                let v = &pair[1];
                if inputs.contains_key(k) {
                    // No overriding; see note "loss of ordering"
                    eprintln!("error: duplicate input: {}", k);
                    std::process::exit(1);
                }
                inputs.insert(k.clone(), serde_json::Value::String(v.clone()));
            }

            let mut provider = ResourceProviderClient::new(ResourceProviderConfig {
                provider_executable: provider_exe.clone(),
                provider_args: vec![],
            })
            .await?;

            let result = provider
                .create(resource_type, &inputs, *stateful)
                .await
                .with_context(|| "failed to create resource");

            provider.close_wait().await?;

            match result {
                Ok(response) => {
                    println!("{}", serde_json::to_string_pretty(&response)?);
                    Ok(())
                }
                Err(err) => {
                    eprintln!("error: {:#}", err);
                    std::process::exit(1);
                }
            }
        }
        Commands::Update {
            provider_exe,
            resource_type,
            input_properties_json,
            previous_input_properties_json,
            previous_output_properties_json,
        } => {
            let input_properties_json = serde_json::from_str(input_properties_json.as_str())
                .with_context(|| "failed to parse value of --inputs-json")?;
            let previous_input_properties_json =
                serde_json::from_str(previous_input_properties_json.as_str())
                    .with_context(|| "failed to parse value of --previous-inputs-json")?;
            let previous_output_properties_json =
                serde_json::from_str(previous_output_properties_json.as_str())
                    .with_context(|| "failed to parse value of --previous-outputs-json")?;

            let mut provider = ResourceProviderClient::new(ResourceProviderConfig {
                provider_executable: provider_exe.clone(),
                provider_args: vec![],
            })
            .await?;

            let result = provider
                .update(
                    resource_type,
                    &input_properties_json,
                    &previous_input_properties_json,
                    &previous_output_properties_json,
                )
                .await;

            provider.close_wait().await?;

            match result {
                Ok(response) => {
                    println!("{}", serde_json::to_string_pretty(&response)?);
                    Ok(())
                }
                Err(err) => {
                    eprintln!("error: {:#}", err);
                    std::process::exit(1);
                }
            }
        }
        Commands::GenerateMan => {
            let cmd = Args::command();
            let man = clap_mangen::Man::new(cmd);
            let mut buffer: Vec<u8> = Default::default();
            man.render(&mut buffer)?;
            println!("{}", String::from_utf8(buffer)?);
            Ok(())
        }
        Commands::GenerateMarkdown => {
            let opts = clap_markdown::MarkdownOptions::new().show_footer(false);
            let markdown: String = clap_markdown::help_markdown_custom::<Args>(&opts);
            println!("{}", markdown);
            Ok(())
        }
        Commands::GenerateCompletion { shell } => {
            // TODO: remove the generate-* commands from the completion
            //       same problem in nixops4 cli
            let mut cmd = Args::command();
            clap_complete::generate(
                *shell,
                &mut cmd,
                "nixops4-resource-runner",
                &mut std::io::stdout(),
            );
            Ok(())
        }
    }
}

/// Simple program to run NixOps resources
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Create a resource
    Create {
        /// The executable that implements the resource operations
        #[arg(long)]
        provider_exe: String,

        /// The type of resource to create: an identifier recognized by the resource provider
        #[arg(long("type"))]
        resource_type: String,

        /// The (whole) JSON input properties for the resource
        ///
        /// This is a JSON object with the values needed to create the resource.
        /// The structure of this object is defined by the resource provider behavior.
        #[arg(long("inputs-json"))]
        input_properties_json: Option<String>,

        /// An individual input property for the resource, in JSON format
        #[arg(long("input-json"),short('j'),number_of_values = 2, value_names = &["NAME", "JSON"])]
        input_property_json: Vec<String>,

        /// An individual input property for the resource, as a raw string.
        ///
        /// This is equivalent to `--input-json NAME JSON` if JSON is the JSON string formatting of STR.
        #[arg(long("input-str"),short('s'),number_of_values = 2, value_names = &["NAME", "STR"])]
        input_property_str: Vec<String>,

        /// Whether state persistence will be provided to the resource
        #[arg(long)]
        stateful: bool,
    },

    /// Update a stateful resource
    Update {
        /// The executable that implements the resource operations
        #[arg(long)]
        provider_exe: String,

        /// The type of resource to update: an identifier recognized by the resource provider
        #[arg(long("type"))]
        resource_type: String,

        /// The new JSON input properties for the resource
        ///
        /// This is a JSON object with the values needed to update the resource.
        /// The structure of this object is defined by the resource provider behavior.
        #[arg(long("inputs-json"))]
        input_properties_json: String,

        /// The previous JSON input properties for the resource, as recorded in the state
        #[arg(long("previous-inputs-json"))]
        previous_input_properties_json: String,

        /// The previous JSON output properties for the resource, as recorded in the state
        #[arg(long("previous-outputs-json"))]
        previous_output_properties_json: String,
    },

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
