//! Integration tests for `cxpak diff --review`.

use clap::Parser;

#[test]
fn diff_review_flag_parses_and_defaults_false() {
    let cli = cxpak::cli::Cli::try_parse_from(["cxpak", "diff", "."]).unwrap();
    match cli.command {
        cxpak::cli::Commands::Diff { review, .. } => assert!(!review),
        _ => panic!("expected Diff"),
    }
}

#[test]
fn diff_review_flag_parses_true_when_set() {
    let cli = cxpak::cli::Cli::try_parse_from(["cxpak", "diff", "--review", "."]).unwrap();
    match cli.command {
        cxpak::cli::Commands::Diff { review, .. } => assert!(review),
        _ => panic!("expected Diff"),
    }
}
