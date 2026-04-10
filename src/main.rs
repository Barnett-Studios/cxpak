use clap::Parser;
use cxpak::cli::{parse_token_count, Cli, Commands};
use cxpak::commands;

fn main() {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Clean { path } => commands::clean::run(path),
        #[cfg(feature = "daemon")]
        Commands::Serve {
            port,
            bind,
            tokens,
            verbose,
            token,
            mcp,
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
            if *mcp {
                commands::serve::run_mcp(path)
            } else {
                commands::serve::run(path, *port, bind, token.as_deref(), token_budget, *verbose)
            }
        }
        #[cfg(feature = "daemon")]
        Commands::Watch {
            tokens,
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
            commands::watch::run(path, token_budget, format, *verbose)
        }
        #[cfg(feature = "daemon")]
        Commands::Conventions { subcommand } => match subcommand {
            cxpak::cli::ConventionsSubcommand::Export { path } => {
                commands::conventions::run_export(path)
            }
            cxpak::cli::ConventionsSubcommand::Diff { path } => {
                commands::conventions::run_diff(path)
            }
        },
        Commands::Diff {
            tokens,
            out,
            format,
            verbose,
            all,
            git_ref,
            focus,
            since,
            timing,
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
            let effective_git_ref = match (git_ref, since) {
                (Some(_), _) => git_ref.clone(),
                (None, Some(since_expr)) => match commands::diff::resolve_since(path, since_expr) {
                    Ok(r) => Some(r),
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                },
                (None, None) => None,
            };
            commands::diff::run(
                path,
                effective_git_ref.as_deref(),
                token_budget,
                format,
                out.as_deref(),
                *verbose,
                *all,
                focus.as_deref(),
                *timing,
            )
        }
        Commands::Overview {
            tokens,
            out,
            format,
            verbose,
            focus,
            timing,
            health,
            workspace,
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
            commands::overview::run(
                path,
                token_budget,
                format,
                out.as_deref(),
                *verbose,
                focus.as_deref(),
                *timing,
                *health,
                workspace.as_deref(),
            )
        }
        Commands::Trace {
            tokens,
            out,
            format,
            verbose,
            all,
            focus,
            timing,
            workspace,
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
                focus.as_deref(),
                *timing,
                workspace.as_deref(),
            )
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
