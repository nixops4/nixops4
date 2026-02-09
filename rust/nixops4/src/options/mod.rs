pub mod lenient_parse;

use clap::{ColorChoice, Parser};
use lenient_parse::parse_longest_prefix;

#[derive(Parser, Debug, Clone)]
pub struct Options {
    #[arg(short, long, global = true, default_value = "false")]
    pub verbose: bool,

    #[arg(long, global = true, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,

    #[arg(long, global = true, default_value_t = false)]
    pub interactive: bool,

    #[arg(
        long,
        global = true,
        default_value_t = false,
        conflicts_with = "interactive"
    )]
    pub no_interactive: bool,

    #[arg(long, global = true, default_value_t = false)]
    pub show_trace: bool,

    /// Temporarily use a different flake input
    // will be post-processed to pair them up
    #[arg(long, num_args = 2, value_names = &["INPUT_ATTR_PATH", "FLAKE_REF"], global = true)]
    pub override_input: Vec<String>,
}

/// Wrapper to parse global Options from a partial command line.
///
/// Used by lenient parsing during shell completion.
#[derive(Parser, Debug)]
#[command(no_binary_name = true)]
struct OptionsWrapper {
    #[command(flatten)]
    options: Options,

    // Catch subcommands and their args without failing
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    _rest: Vec<String>,
}

/// Parse global options from the current environment's command line.
///
/// For use during shell completion, where we have a partial command line
/// but want to extract options like `--override-input` that the user has
/// already specified.
///
/// Returns the successfully parsed options, or clap's default options if parsing fails.
pub fn parse_options_for_completion() -> Options {
    // Get the raw args, skipping everything before "--" which is clap_complete's
    // convention for separating completer args from the actual command line
    let args: Vec<String> = std::env::args()
        .skip_while(|a| a != "--")
        .skip(1) // skip the "--" itself
        .skip(1) // skip the command name (e.g., "nixops4")
        .collect();

    parse_longest_prefix::<OptionsWrapper>(&args)
        .expect("OptionsWrapper parses empty args with default values")
        .options
}
