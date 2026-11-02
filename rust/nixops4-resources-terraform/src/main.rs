use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};

mod provider;
mod schema;
mod tf_provider_client;

fn main() -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async_main())
}

async fn async_main() -> Result<()> {
    let args = Args::parse();

    match &args.command {
        Commands::Run { provider_exe } => {
            // Run as NixOps4 resource provider using the framework
            nixops4_resource::framework::run_main(provider::TerraformProvider::new(
                provider_exe.clone(),
            ))
            .await;
            Ok(())
        }
        Commands::Schema { provider_path } => {
            // Launch provider and get schema
            let mut client = tf_provider_client::ProviderClient::launch(provider_path)
                .await
                .context("Failed to launch Terraform provider")?;

            let raw_schema = client
                .client_connection()
                .context("Failed to get gRPC client")?
                .get_provider_schema()
                .await
                .context("Failed to get provider schema")?;

            // Convert to our unified schema format
            let unified_schema = schema::ProviderSchema::from_raw_response(raw_schema);

            // Output unified schema as JSON
            println!("{}", serde_json::to_string_pretty(&unified_schema)?);

            client
                .shutdown()
                .await
                .context("Failed to shutdown provider")?;

            Ok(())
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
            let mut cmd = Args::command();
            clap_complete::generate(
                *shell,
                &mut cmd,
                "nixops4-resources-terraform",
                &mut std::io::stdout(),
            );
            Ok(())
        }
    }
}

/// Terraform provider adapter for NixOps4
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run as NixOps4 resource provider
    Run {
        /// The Terraform provider executable to wrap
        #[arg(long)]
        provider_exe: String,
    },

    /// Get provider schema information
    Schema {
        /// Path to the Terraform provider binary
        #[arg(long)]
        provider_path: String,
    },

    /// Generate markdown documentation
    #[command(hide = true)]
    GenerateMarkdown,

    /// Generate a manpage
    #[command(hide = true)]
    GenerateMan,

    /// Generate shell completion
    #[command(hide = true)]
    GenerateCompletion {
        /// The shell to generate completion for
        #[arg(long)]
        shell: clap_complete::Shell,
    },
}
