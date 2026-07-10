use clap::Parser;
use cxpak::cli::{parse_token_count, Cli, Commands};
use cxpak::commands;

fn main() {
    cxpak::dev_maintenance::maybe_sweep();

    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Clean { path } => commands::clean::run(path),
        Commands::Schema { capability } => commands::schema::run(capability.as_deref()),
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
        #[cfg(feature = "lsp")]
        Commands::Lsp { path } => cxpak::lsp::run_stdio(path),
        #[cfg(feature = "daemon")]
        Commands::Conventions { subcommand } => match subcommand {
            cxpak::cli::ConventionsSubcommand::Export { path } => {
                commands::conventions::run_export(path)
            }
            cxpak::cli::ConventionsSubcommand::Diff { path } => {
                commands::conventions::run_diff(path)
            }
        },
        #[cfg(feature = "plugins")]
        Commands::Plugin { subcommand } => match subcommand {
            cxpak::cli::PluginSubcommand::List { path } => commands::plugin::run_list(path),
            cxpak::cli::PluginSubcommand::Add {
                wasm_path,
                name,
                patterns,
                needs_content,
                path,
            } => commands::plugin::run_add(
                path,
                wasm_path,
                name.as_deref(),
                patterns,
                *needs_content,
            ),
        },
        Commands::Diff {
            tokens,
            out,
            format,
            verbose,
            all,
            review,
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
                *review,
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
        Commands::Graph {
            op,
            id,
            from,
            to,
            direction,
            seeds,
            depth,
            workspace,
            path,
        } => commands::graph::run(
            path,
            op,
            id.as_deref(),
            from.as_deref(),
            to.as_deref(),
            direction,
            seeds,
            *depth,
            workspace.as_deref(),
        ),
        Commands::Search {
            op,
            query,
            symbol,
            seeds,
            depth,
            limit,
            workspace,
            path,
        } => commands::search::run(
            path,
            op,
            query.as_deref(),
            symbol.as_deref(),
            seeds,
            *depth,
            *limit,
            workspace.as_deref(),
        ),
        Commands::Hook { subcommand } => match subcommand {
            cxpak::cli::HookSubcommand::Install { path } => commands::hook::install(path),
            cxpak::cli::HookSubcommand::PostCommit { path } => commands::hook::post_commit(path),
            cxpak::cli::HookSubcommand::MergeDriver {
                ancestor,
                current,
                other,
            } => commands::hook::merge_driver(ancestor, current, other),
        },
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
        #[cfg(feature = "visual")]
        Commands::Visual {
            visual_type,
            format,
            out,
            symbol,
            files,
            focus,
            path,
        } => commands::visual::run(
            path,
            visual_type,
            format,
            out.as_deref(),
            symbol.as_deref(),
            files.as_deref(),
            focus.as_deref(),
        ),
        #[cfg(feature = "visual")]
        Commands::Onboard {
            focus,
            format,
            out,
            path,
        } => commands::onboard::run(path, focus.as_deref(), format, out.as_deref()),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
