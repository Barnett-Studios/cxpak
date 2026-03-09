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
        #[arg(long)]
        tokens: String,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value = "markdown")]
        format: OutputFormat,
        #[arg(long)]
        verbose: bool,
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Trace from error/function, pack relevant code paths
    Trace {
        #[arg(long)]
        tokens: String,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value = "markdown")]
        format: OutputFormat,
        #[arg(long)]
        verbose: bool,
        #[arg(long)]
        all: bool,
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
    }
}
