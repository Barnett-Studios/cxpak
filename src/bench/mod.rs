// Benchmark harness — corpus loader and changed-file fetch.
//
// Gated behind the `bench` feature flag; never compiled into the default binary.
// Used by D2.x benchmark infrastructure: D2.1 corpus + fetch, D2.2 recall metric,
// D2.3 CI gate.
//
// The `bench` feature intentionally excludes network deps (reqwest) and relies
// on the `gh` CLI subprocess for all GitHub API calls, keeping network code out
// of the binary and delegating auth to the user's existing gh session.

pub mod fetch;

use serde::Deserialize;
use std::path::Path;

/// A single corpus entry: one real merged PR whose changed-file set is the
/// ground-truth for recall evaluation.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CorpusEntry {
    /// GitHub repo in `owner/name` form, e.g. `"BurntSushi/ripgrep"`.
    pub repo: String,
    /// PR number.
    pub pr: u32,
    /// Full 40-hex SHA of the base commit (the target branch tip at merge time).
    pub base_sha: String,
    /// Full 40-hex SHA of the head commit (the PR tip at merge time).
    pub head_sha: String,
    /// Primary language of the repo, e.g. `"Rust"`, `"Python"`.
    pub lang: String,
    /// Optional one-line description (from the PR title).
    pub title: Option<String>,
}

/// The top-level TOML document: `[[entry]]` array.
#[derive(Debug, Deserialize)]
struct CorpusDoc {
    entry: Vec<CorpusEntry>,
}

/// Loads `bench/corpus.toml` relative to the repo root at `base`.
///
/// Returns an error if the file is missing, unparseable, or empty.
pub fn load_corpus(base: &Path) -> Result<Vec<CorpusEntry>, String> {
    let path = base.join("bench/corpus.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    let doc: CorpusDoc =
        toml::from_str(&text).map_err(|e| format!("parse error in {}: {}", path.display(), e))?;
    if doc.entry.is_empty() {
        return Err(format!("{} contains no [[entry]] items", path.display()));
    }
    Ok(doc.entry)
}
