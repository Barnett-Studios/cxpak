use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "cxpak",
    about = "Spends CPU cycles so you don't spend tokens",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Structured repo summary within a token budget
    Overview {
        #[arg(long, default_value = "50k")]
        tokens: String,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value = "markdown")]
        format: OutputFormat,
        #[arg(long)]
        verbose: bool,
        /// Boost files under this path prefix in the ranking
        #[arg(long)]
        focus: Option<String>,
        /// Print pipeline stage durations to stderr
        #[arg(long)]
        timing: bool,
        /// Append codebase health score to the overview output
        #[arg(long)]
        health: bool,
        /// Monorepo workspace prefix (only index files under this path)
        #[arg(long)]
        workspace: Option<String>,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Remove .cxpak/ directory (cache + output files)
    Clean {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Show token-budgeted change summary with dependency context
    Diff {
        #[arg(long, default_value = "50k")]
        tokens: String,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value = "markdown")]
        format: OutputFormat,
        #[arg(long)]
        verbose: bool,
        #[arg(long)]
        all: bool,
        /// Git ref to diff against (default: HEAD for working tree changes)
        #[arg(long)]
        git_ref: Option<String>,
        /// Boost files under this path prefix in the ranking
        #[arg(long)]
        focus: Option<String>,
        /// Time expression for --since (e.g. "1d", "1 week", "yesterday")
        #[arg(long)]
        since: Option<String>,
        /// Print pipeline stage durations to stderr
        #[arg(long)]
        timing: bool,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Start HTTP server with hot index
    #[cfg(feature = "daemon")]
    Serve {
        #[arg(long, default_value = "3000")]
        port: u16,
        /// Bind address for the HTTP server
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
        #[arg(long, default_value = "50k")]
        tokens: String,
        #[arg(long)]
        verbose: bool,
        /// Require Bearer token for /v1/ endpoints
        #[arg(long)]
        token: Option<String>,
        /// Run as MCP server over stdio instead of HTTP
        #[arg(long)]
        mcp: bool,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Watch for file changes and keep index hot
    #[cfg(feature = "daemon")]
    Watch {
        #[arg(long, default_value = "50k")]
        tokens: String,
        #[arg(long, default_value = "markdown")]
        format: OutputFormat,
        #[arg(long)]
        verbose: bool,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Run LSP server over stdio
    #[cfg(feature = "lsp")]
    Lsp {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Generate interactive visual dashboard
    #[cfg(feature = "visual")]
    Visual {
        /// dashboard | architecture | risk | flow | timeline | diff
        #[arg(long, default_value = "dashboard")]
        visual_type: VisualTypeArg,
        /// html | mermaid | svg | png | c4 | json
        #[arg(long, default_value = "html")]
        format: VisualFormatArg,
        #[arg(long)]
        out: Option<PathBuf>,
        /// For flow type: the symbol to trace
        #[arg(long)]
        symbol: Option<String>,
        /// For diff type: comma-separated changed file paths
        #[arg(long)]
        files: Option<String>,
        #[arg(long)]
        focus: Option<String>,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Generate onboarding guide for the codebase
    #[cfg(feature = "visual")]
    Onboard {
        #[arg(long)]
        focus: Option<String>,
        #[arg(long, default_value = "markdown")]
        format: OutputFormat,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Export and diff convention profiles
    #[cfg(feature = "daemon")]
    Conventions {
        #[command(subcommand)]
        subcommand: ConventionsSubcommand,
    },
    /// Manage WASM plugins registered in .cxpak/plugins.json
    #[cfg(feature = "plugins")]
    Plugin {
        #[command(subcommand)]
        subcommand: PluginSubcommand,
    },
    /// Trace from error/function, pack relevant code paths
    Trace {
        #[arg(long, default_value = "50k")]
        tokens: String,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value = "markdown")]
        format: OutputFormat,
        #[arg(long)]
        verbose: bool,
        #[arg(long)]
        all: bool,
        /// Boost files under this path prefix in the ranking
        #[arg(long)]
        focus: Option<String>,
        /// Print pipeline stage durations to stderr
        #[arg(long)]
        timing: bool,
        /// Monorepo workspace prefix (only index files under this path)
        #[arg(long)]
        workspace: Option<String>,
        target: String,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

#[cfg(feature = "daemon")]
#[derive(Subcommand)]
pub enum ConventionsSubcommand {
    /// Write .cxpak/conventions.json from the current codebase
    Export {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Compare current conventions against .cxpak/conventions.json
    Diff {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

#[cfg(feature = "plugins")]
#[derive(Subcommand)]
pub enum PluginSubcommand {
    /// List all registered plugins
    List {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Register a new WASM plugin
    Add {
        /// Path to the .wasm file
        wasm_path: PathBuf,
        /// Optional name; defaults to the wasm filename stem
        #[arg(long)]
        name: Option<String>,
        /// Comma-separated glob patterns controlling which files the plugin sees
        #[arg(long, value_delimiter = ',')]
        patterns: Vec<String>,
        /// Grant the plugin access to raw file contents
        #[arg(long)]
        needs_content: bool,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

/// CLI argument type for visual type selection
#[cfg(feature = "visual")]
#[derive(Clone, Debug, clap::ValueEnum)]
pub enum VisualTypeArg {
    Dashboard,
    Architecture,
    Risk,
    Flow,
    Timeline,
    Diff,
}

/// CLI argument type for visual format selection
#[cfg(feature = "visual")]
#[derive(Clone, Debug, clap::ValueEnum)]
pub enum VisualFormatArg {
    Html,
    Mermaid,
    Svg,
    Png,
    C4,
    Json,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum OutputFormat {
    Markdown,
    Xml,
    Json,
}

/// Parse token count strings like "50000", "50k", "100K", "1m", "1M"
pub fn parse_token_count(s: &str) -> Result<usize, String> {
    let s = s.trim().to_lowercase();
    if let Some(prefix) = s.strip_suffix('k') {
        prefix
            .parse::<f64>()
            .map(|n| (n * 1_000.0) as usize)
            .map_err(|e| format!("invalid token count: {e}"))
    } else if let Some(prefix) = s.strip_suffix('m') {
        prefix
            .parse::<f64>()
            .map(|n| (n * 1_000_000.0) as usize)
            .map_err(|e| format!("invalid token count: {e}"))
    } else {
        s.parse::<usize>()
            .map_err(|e| format!("invalid token count: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_token_count_plain_number() {
        assert_eq!(parse_token_count("50000").unwrap(), 50000);
    }

    #[test]
    fn test_parse_token_count_k_suffix() {
        assert_eq!(parse_token_count("50k").unwrap(), 50000);
        assert_eq!(parse_token_count("50K").unwrap(), 50000);
        assert_eq!(parse_token_count("100k").unwrap(), 100000);
    }

    #[test]
    fn test_parse_token_count_m_suffix() {
        assert_eq!(parse_token_count("1m").unwrap(), 1000000);
        assert_eq!(parse_token_count("1M").unwrap(), 1000000);
    }

    #[test]
    fn test_parse_token_count_fractional() {
        assert_eq!(parse_token_count("1.5k").unwrap(), 1500);
        assert_eq!(parse_token_count("0.5m").unwrap(), 500000);
    }

    #[test]
    fn test_parse_token_count_invalid() {
        assert!(parse_token_count("abc").is_err());
        assert!(parse_token_count("").is_err());
        assert!(parse_token_count("k").is_err());
        assert!(parse_token_count("m").is_err()); // covers line 111
        assert!(parse_token_count("xyzm").is_err());
    }

    #[test]
    fn test_focus_flag_parses_for_overview() {
        let cli = Cli::try_parse_from([
            "cxpak", "overview", "--tokens", "50k", "--focus", "src/auth",
        ])
        .expect("should parse successfully");

        match cli.command {
            Commands::Overview { focus, .. } => {
                assert_eq!(focus.as_deref(), Some("src/auth"));
            }
            _ => panic!("expected Overview command"),
        }
    }

    #[test]
    fn test_focus_flag_parses_for_diff() {
        let cli = Cli::try_parse_from(["cxpak", "diff", "--tokens", "50k", "--focus", "src/api"])
            .expect("should parse successfully");

        match cli.command {
            Commands::Diff { focus, .. } => {
                assert_eq!(focus.as_deref(), Some("src/api"));
            }
            _ => panic!("expected Diff command"),
        }
    }

    #[test]
    fn test_focus_flag_parses_for_trace() {
        let cli = Cli::try_parse_from([
            "cxpak",
            "trace",
            "--tokens",
            "50k",
            "--focus",
            "src/lib",
            "my_function",
        ])
        .expect("should parse successfully");

        match cli.command {
            Commands::Trace { focus, .. } => {
                assert_eq!(focus.as_deref(), Some("src/lib"));
            }
            _ => panic!("expected Trace command"),
        }
    }

    #[test]
    fn test_focus_flag_is_optional() {
        let cli = Cli::try_parse_from(["cxpak", "overview", "--tokens", "50k"])
            .expect("should parse without --focus");

        match cli.command {
            Commands::Overview { focus, .. } => {
                assert!(focus.is_none());
            }
            _ => panic!("expected Overview command"),
        }
    }

    #[test]
    fn test_timing_flag_parses_for_overview() {
        let cli = Cli::try_parse_from(["cxpak", "overview", "--tokens", "50k", "--timing"])
            .expect("should parse with --timing");

        match cli.command {
            Commands::Overview { timing, .. } => {
                assert!(timing);
            }
            _ => panic!("expected Overview command"),
        }
    }

    #[test]
    fn test_timing_flag_defaults_to_false() {
        let cli = Cli::try_parse_from(["cxpak", "overview", "--tokens", "50k"])
            .expect("should parse without --timing");

        match cli.command {
            Commands::Overview { timing, .. } => {
                assert!(!timing);
            }
            _ => panic!("expected Overview command"),
        }
    }

    #[test]
    fn test_timing_flag_parses_for_diff() {
        let cli = Cli::try_parse_from(["cxpak", "diff", "--tokens", "50k", "--timing"])
            .expect("should parse with --timing");

        match cli.command {
            Commands::Diff { timing, .. } => {
                assert!(timing);
            }
            _ => panic!("expected Diff command"),
        }
    }

    #[test]
    fn test_timing_flag_parses_for_trace() {
        let cli = Cli::try_parse_from([
            "cxpak",
            "trace",
            "--tokens",
            "50k",
            "--timing",
            "my_function",
        ])
        .expect("should parse with --timing");

        match cli.command {
            Commands::Trace { timing, .. } => {
                assert!(timing);
            }
            _ => panic!("expected Trace command"),
        }
    }

    #[test]
    fn test_overview_default_tokens() {
        let cli =
            Cli::try_parse_from(["cxpak", "overview"]).expect("should parse without --tokens");
        match cli.command {
            Commands::Overview { tokens, .. } => {
                assert_eq!(tokens, "50k");
            }
            _ => panic!("expected Overview"),
        }
    }

    #[test]
    fn test_diff_default_tokens() {
        let cli = Cli::try_parse_from(["cxpak", "diff"]).expect("should parse without --tokens");
        match cli.command {
            Commands::Diff { tokens, .. } => {
                assert_eq!(tokens, "50k");
            }
            _ => panic!("expected Diff"),
        }
    }

    #[test]
    fn test_trace_default_tokens() {
        let cli = Cli::try_parse_from(["cxpak", "trace", "my_symbol"])
            .expect("should parse without --tokens");
        match cli.command {
            Commands::Trace { tokens, .. } => {
                assert_eq!(tokens, "50k");
            }
            _ => panic!("expected Trace"),
        }
    }

    #[test]
    fn test_health_flag_parses_for_overview() {
        let cli = Cli::try_parse_from(["cxpak", "overview", "--tokens", "50k", "--health"])
            .expect("should parse with --health");
        match cli.command {
            Commands::Overview { health, .. } => {
                assert!(health);
            }
            _ => panic!("expected Overview command"),
        }
    }

    #[test]
    fn test_health_flag_defaults_to_false() {
        let cli = Cli::try_parse_from(["cxpak", "overview", "--tokens", "50k"])
            .expect("should parse without --health");
        match cli.command {
            Commands::Overview { health, .. } => {
                assert!(!health);
            }
            _ => panic!("expected Overview command"),
        }
    }

    #[cfg(feature = "daemon")]
    #[test]
    fn cli_conventions_export_parses() {
        let cli = Cli::try_parse_from(["cxpak", "conventions", "export", "."])
            .expect("should parse conventions export");
        match cli.command {
            Commands::Conventions { subcommand } => match subcommand {
                super::ConventionsSubcommand::Export { path } => {
                    assert_eq!(path, std::path::PathBuf::from("."));
                }
                _ => panic!("expected Export subcommand"),
            },
            _ => panic!("expected Conventions command"),
        }
    }

    #[cfg(feature = "daemon")]
    #[test]
    fn cli_conventions_diff_parses() {
        let cli = Cli::try_parse_from(["cxpak", "conventions", "diff", "."])
            .expect("should parse conventions diff");
        match cli.command {
            Commands::Conventions { subcommand } => match subcommand {
                super::ConventionsSubcommand::Diff { path } => {
                    assert_eq!(path, std::path::PathBuf::from("."));
                }
                _ => panic!("expected Diff subcommand"),
            },
            _ => panic!("expected Conventions command"),
        }
    }

    #[test]
    fn test_tokens_override_still_works() {
        let cli = Cli::try_parse_from(["cxpak", "overview", "--tokens", "100k"])
            .expect("should parse with explicit --tokens");
        match cli.command {
            Commands::Overview { tokens, .. } => {
                assert_eq!(tokens, "100k");
            }
            _ => panic!("expected Overview"),
        }
    }

    #[cfg(feature = "lsp")]
    #[test]
    fn cli_lsp_parses() {
        let cli =
            Cli::try_parse_from(["cxpak", "lsp"]).expect("should parse lsp with default path");
        match cli.command {
            Commands::Lsp { path } => {
                assert_eq!(path, std::path::PathBuf::from("."));
            }
            _ => panic!("expected Lsp command"),
        }
    }

    #[cfg(feature = "lsp")]
    #[test]
    fn cli_lsp_custom_path() {
        let cli = Cli::try_parse_from(["cxpak", "lsp", "/tmp/repo"])
            .expect("should parse lsp with custom path");
        match cli.command {
            Commands::Lsp { path } => {
                assert_eq!(path, std::path::PathBuf::from("/tmp/repo"));
            }
            _ => panic!("expected Lsp command"),
        }
    }

    #[cfg(feature = "daemon")]
    #[test]
    fn cli_serve_default_bind() {
        let cli =
            Cli::try_parse_from(["cxpak", "serve"]).expect("should parse serve with defaults");
        match cli.command {
            Commands::Serve { bind, .. } => {
                assert_eq!(bind, "127.0.0.1");
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[cfg(feature = "daemon")]
    #[test]
    fn cli_serve_custom_bind() {
        let cli = Cli::try_parse_from(["cxpak", "serve", "--bind", "0.0.0.0"])
            .expect("should parse serve with custom bind");
        match cli.command {
            Commands::Serve { bind, .. } => {
                assert_eq!(bind, "0.0.0.0");
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[cfg(feature = "daemon")]
    #[test]
    fn cli_serve_token_flag() {
        let cli = Cli::try_parse_from(["cxpak", "serve", "--token", "secret"])
            .expect("should parse serve with token");
        match cli.command {
            Commands::Serve { token, .. } => {
                assert_eq!(token.as_deref(), Some("secret"));
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[cfg(feature = "daemon")]
    #[test]
    fn cli_serve_token_defaults_to_none() {
        let cli =
            Cli::try_parse_from(["cxpak", "serve"]).expect("should parse serve without token");
        match cli.command {
            Commands::Serve { token, .. } => {
                assert!(token.is_none());
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[cfg(feature = "visual")]
    #[test]
    fn test_visual_command_parses() {
        let cli = Cli::try_parse_from(["cxpak", "visual", "--visual-type", "dashboard"])
            .expect("should parse visual command");
        match cli.command {
            Commands::Visual { visual_type, .. } => {
                assert!(matches!(visual_type, VisualTypeArg::Dashboard));
            }
            _ => panic!("expected Visual command"),
        }
    }

    #[cfg(feature = "visual")]
    #[test]
    fn test_visual_command_flow_with_symbol() {
        let cli = Cli::try_parse_from([
            "cxpak",
            "visual",
            "--visual-type",
            "flow",
            "--format",
            "html",
            "--symbol",
            "main",
        ])
        .expect("should parse");
        match cli.command {
            Commands::Visual {
                visual_type,
                symbol,
                ..
            } => {
                assert!(matches!(visual_type, VisualTypeArg::Flow));
                assert_eq!(symbol.as_deref(), Some("main"));
            }
            _ => panic!("expected Visual"),
        }
    }

    #[cfg(feature = "visual")]
    #[test]
    fn test_onboard_command_parses() {
        let cli = Cli::try_parse_from(["cxpak", "onboard"]).expect("should parse onboard command");
        match cli.command {
            Commands::Onboard { format, .. } => {
                assert!(matches!(format, OutputFormat::Markdown));
            }
            _ => panic!("expected Onboard"),
        }
    }

    #[cfg(feature = "plugins")]
    #[test]
    fn cli_plugin_list_parses() {
        let cli = Cli::try_parse_from(["cxpak", "plugin", "list", "."])
            .expect("should parse plugin list");
        match cli.command {
            Commands::Plugin { subcommand } => match subcommand {
                PluginSubcommand::List { path } => {
                    assert_eq!(path, std::path::PathBuf::from("."));
                }
                _ => panic!("expected List"),
            },
            _ => panic!("expected Plugin"),
        }
    }

    #[cfg(feature = "plugins")]
    #[test]
    fn cli_plugin_add_parses() {
        let cli = Cli::try_parse_from([
            "cxpak",
            "plugin",
            "add",
            "foo.wasm",
            "--name",
            "foo",
            "--patterns",
            "**/*.py,**/*.pyi",
            "--needs-content",
        ])
        .expect("should parse plugin add");
        match cli.command {
            Commands::Plugin { subcommand } => match subcommand {
                PluginSubcommand::Add {
                    wasm_path,
                    name,
                    patterns,
                    needs_content,
                    ..
                } => {
                    assert_eq!(wasm_path, std::path::PathBuf::from("foo.wasm"));
                    assert_eq!(name, Some("foo".to_string()));
                    assert_eq!(
                        patterns,
                        vec!["**/*.py".to_string(), "**/*.pyi".to_string()]
                    );
                    assert!(needs_content);
                }
                _ => panic!("expected Add"),
            },
            _ => panic!("expected Plugin"),
        }
    }
}
