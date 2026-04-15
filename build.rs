use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn main() {
    // Re-run whenever Cargo.lock changes so the grammar hash stays in sync.
    println!("cargo:rerun-if-changed=Cargo.lock");

    let grammar_hash = compute_grammar_hash();
    println!("cargo:rustc-env=CXPAK_GRAMMAR_HASH={grammar_hash}");
}

/// Read Cargo.lock, extract the version strings of all `tree-sitter-*` crates,
/// sort them for stability, concatenate, and hash with DefaultHasher.
fn compute_grammar_hash() -> u64 {
    let lock_content = std::fs::read_to_string("Cargo.lock").unwrap_or_default();

    // Collect (name, version) pairs for tree-sitter-* packages.
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_version: Option<String> = None;

    for line in lock_content.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            // Flush previous entry
            if let (Some(name), Some(version)) = (current_name.take(), current_version.take()) {
                if name.starts_with("tree-sitter") {
                    entries.push((name, version));
                }
            }
        } else if let Some(rest) = trimmed.strip_prefix("name = ") {
            current_name = Some(rest.trim_matches('"').to_string());
        } else if let Some(rest) = trimmed.strip_prefix("version = ") {
            current_version = Some(rest.trim_matches('"').to_string());
        }
    }
    // Flush final entry
    if let (Some(name), Some(version)) = (current_name, current_version) {
        if name.starts_with("tree-sitter") {
            entries.push((name, version));
        }
    }

    if entries.is_empty() {
        return 0;
    }

    // Sort for deterministic ordering regardless of lock file ordering.
    entries.sort();

    // Concatenate and hash.
    let combined: String = entries.iter().map(|(n, v)| format!("{n}={v};")).collect();

    let mut hasher = DefaultHasher::new();
    combined.hash(&mut hasher);
    hasher.finish()
}
