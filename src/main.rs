mod cli;
mod commands;

pub mod budget;
pub mod git;
pub mod index;
pub mod output;
pub mod parser;
pub mod scanner;
pub mod util;

use clap::Parser;
use cli::{parse_token_count, Cli, Commands};

fn main() {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Overview {
            tokens,
            out,
            format,
            verbose,
            path,
        } => {
            let token_budget = match parse_token_count(tokens) {
                Ok(0) => {
                    eprintln!("Error: --tokens must be greater than 0");
                    std::process::exit(1);
                }
                Ok(n) => n,
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            };
            commands::overview::run(path, token_budget, format, out.as_deref(), *verbose)
        }
        Commands::Trace {
            tokens,
            out,
            format,
            verbose,
            all,
            target,
            path,
        } => {
            let token_budget = match parse_token_count(tokens) {
                Ok(0) => {
                    eprintln!("Error: --tokens must be greater than 0");
                    std::process::exit(1);
                }
                Ok(n) => n,
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            };
            commands::trace::run(
                path,
                target,
                token_budget,
                format,
                out.as_deref(),
                *verbose,
                *all,
            )
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
