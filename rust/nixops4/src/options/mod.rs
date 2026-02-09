pub mod lenient_parse;

use clap::{ColorChoice, Parser};

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
