use clap::Command;
use std::process::exit;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn root_command() -> Command {
    Command::new("nixops4")
        .version(VERSION)
        .about("Deploy with Nix and manage resources declaratively")
}

fn main() {
    let matches = root_command().get_matches();
    match matches.subcommand() {
        Some((name, _)) => {
            eprintln!("nixops4 internal error: unknown subcommand: {}", name);
            exit(1);
        }
        None => {
            root_command().print_help().unwrap();
            eprintln!("\nNo subcommand given");
            exit(1);
        }
    }
}
