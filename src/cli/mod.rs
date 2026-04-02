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
        #[arg(long, default_value = "50k")]
        tokens: String,
        #[arg(long)]
        verbose: bool,
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
        target: String,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
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
}
