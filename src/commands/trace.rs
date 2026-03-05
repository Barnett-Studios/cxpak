use crate::cli::OutputFormat;
use std::path::Path;

pub fn run(
    _target: &str,
    _token_budget: usize,
    _format: &OutputFormat,
    _out: Option<&Path>,
    _verbose: bool,
    _all: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("cxpak: trace command is not yet implemented (coming in v2)");
    std::process::exit(1);
}
