// Changed-file fetch via `gh api` subprocess.
//
// Uses the GitHub CLI (`gh`) so no network code enters the binary and auth is
// handled by the user's existing gh session (or GITHUB_TOKEN in CI).
//
// Pagination: `--paginate` is passed to gh, which automatically follows
// `Link: <next>` headers and concatenates all pages before returning.

use super::CorpusEntry;
use std::process::Command;

/// Returns the sorted, deduplicated list of changed filenames for `entry`.
///
/// Shells out to:
///   `gh api repos/{repo}/pulls/{pr}/files --paginate --jq '.[].filename'`
///
/// Errors (descriptive, never panics):
/// - `gh` not found on PATH
/// - non-zero exit (rate-limit, 404, network failure) — includes stderr
/// - empty response (a merged PR with zero changed files is unusual and likely
///   indicates a data problem in the corpus)
pub fn fetch_changed_files(entry: &CorpusEntry) -> Result<Vec<String>, String> {
    let endpoint = format!("repos/{}/pulls/{}/files", entry.repo, entry.pr);

    let output = Command::new("gh")
        .args(["api", &endpoint, "--paginate", "--jq", ".[].filename"])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "gh CLI not found on PATH — install via https://cli.github.com".to_string()
            } else {
                format!("failed to spawn gh: {}", e)
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "gh api {endpoint} exited with {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files: Vec<String> = stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();

    if files.is_empty() {
        return Err(format!(
            "gh api returned no filenames for {}/{}; corpus entry may be invalid",
            entry.repo, entry.pr
        ));
    }

    files.sort();
    files.dedup();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::load_corpus;
    use std::collections::HashSet;
    use std::path::Path;

    // ── corpus integrity (no network) ──────────────────────────────────────────
    //
    // Runs in default `cargo test` (no feature gate, no --ignore).
    // Guards the committed corpus.toml without touching the network.

    fn repo_root() -> &'static Path {
        // Tests run with cwd = repo root (standard Cargo behaviour).
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn corpus_has_minimum_entries_and_repos() {
        let entries = load_corpus(repo_root()).expect("corpus should load");

        assert!(
            entries.len() >= 50,
            "expected ≥50 corpus entries, got {}",
            entries.len()
        );

        let repos: HashSet<&str> = entries.iter().map(|e| e.repo.as_str()).collect();
        assert!(
            repos.len() >= 5,
            "expected entries from ≥5 distinct repos, got {} ({:?})",
            repos.len(),
            repos
        );
    }

    #[test]
    fn corpus_entries_have_required_fields() {
        let entries = load_corpus(repo_root()).expect("corpus should load");
        let sha_re = regex::Regex::new(r"^[0-9a-f]{40}$").unwrap();

        for e in &entries {
            assert!(!e.repo.is_empty(), "empty repo in pr {}", e.pr);
            assert!(e.repo.contains('/'), "repo missing slash: {}", e.repo);
            assert!(e.pr > 0, "pr must be > 0, repo {}", e.repo);
            assert!(
                sha_re.is_match(&e.base_sha),
                "bad base_sha '{}' for {}/{}",
                e.base_sha,
                e.repo,
                e.pr
            );
            assert!(
                sha_re.is_match(&e.head_sha),
                "bad head_sha '{}' for {}/{}",
                e.head_sha,
                e.repo,
                e.pr
            );
            assert!(!e.lang.is_empty(), "empty lang for {}/{}", e.repo, e.pr);
        }
    }

    #[test]
    fn corpus_has_no_duplicate_repo_pr_pairs() {
        let entries = load_corpus(repo_root()).expect("corpus should load");
        let mut seen: HashSet<(String, u32)> = HashSet::new();

        for e in &entries {
            let key = (e.repo.clone(), e.pr);
            assert!(
                seen.insert(key.clone()),
                "duplicate corpus entry: {}/{}",
                e.repo,
                e.pr
            );
        }
    }

    // ── fetch reconstruction (NETWORK, #[ignore]) ──────────────────────────────
    //
    // Run explicitly with:
    //   cargo test --features bench fetch_reconstruction -- --ignored
    // or in CI:
    //   CXPAK_BENCH_NET=1 cargo test --features bench fetch_reconstruction -- --ignored
    //
    // Pinned entry: BurntSushi/ripgrep PR #3271 (ignore/types: add Containerfile).
    // Verified changed files (stable — merged PR): crates/ignore/src/default_types.rs

    #[test]
    #[ignore = "network test — run with: cargo test --features bench -- --ignored"]
    fn fetch_reconstruction_pinned_pr() {
        let entry = CorpusEntry {
            repo: "BurntSushi/ripgrep".to_string(),
            pr: 3271,
            base_sha: "0a88cccd5188074de96f54a4b6b44a63971ac157".to_string(),
            head_sha: "a50ddc7ce17f98b9624e7a663bb080dd8580427a".to_string(),
            lang: "Rust".to_string(),
            title: Some("ignore/types: add Containerfile".to_string()),
        };

        let files = fetch_changed_files(&entry).expect("fetch should succeed");

        assert!(!files.is_empty(), "expected non-empty file list");
        assert!(
            files.iter().any(|f| f.contains("types")),
            "expected a file containing 'types' in changed files for PR #3271; got: {:?}",
            files
        );
    }
}
