// D2.2 — recall@budget metric + retrieval baselines.
//
// Measures how well a retrieval system recovers the files a real merged PR
// actually changed, within a token budget, against the repo AT its `base_sha`.
//
// Layering (deliberate, so the metric math is testable without a network):
//   * Pure metric core — `recall_at_budget`, `mrr`, `render_comparison_table`,
//     `ground_truth_at_base`. No I/O, no network, fully unit-tested in the
//     default `cargo test --features bench` run.
//   * Gated harness — `run_harness` and its helpers. These fetch each corpus
//     repo at `base_sha`, build a cxpak index, and run every system. They touch
//     the network and disk and are only invoked from the `#[ignore]`d, env-gated
//     smoke test; nothing in the default test path calls them.
//
// Ground-truth definition (the base-tree nuance):
//   A PR's changed files come back from `gh api .../files` tagged with a
//   `status` (added / modified / removed / renamed). Retrieval runs over the
//   tree AT `base_sha`, so files that only *appear* in the head tree cannot be
//   retrieved. We therefore EXCLUDE `added` files from the ground-truth set:
//   they don't exist at base and would unfairly depress every system's recall.
//   For `renamed`, the *previous* path exists at base, so we count that path.
//   `modified` and `removed`/`deleted` paths exist at base and are kept as-is.

use crate::bench::CorpusEntry;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

// ===========================================================================
// Pure metric core (no I/O — unit-tested in default `cargo test --features bench`)
// ===========================================================================

/// Fraction of ground-truth files that appear in `selected`.
///
/// `recall = |selected ∩ ground_truth| / |ground_truth|`, computed over sets so
/// duplicates in `selected` never inflate the numerator.
///
/// Edge case: an empty `ground_truth` is treated as `1.0` (vacuously perfect) —
/// there is nothing to retrieve, so the entry should not drag the corpus mean
/// toward zero.
pub fn recall_at_budget(selected: &[String], ground_truth: &HashSet<String>) -> f64 {
    if ground_truth.is_empty() {
        return 1.0;
    }
    let selected_set: HashSet<&String> = selected.iter().collect();
    let hits = ground_truth
        .iter()
        .filter(|g| selected_set.contains(g))
        .count();
    hits as f64 / ground_truth.len() as f64
}

/// Mean reciprocal rank of the FIRST ground-truth file in `ranked`.
///
/// `mrr = 1 / rank_of_first_hit` (1-based rank), or `0.0` if no ground-truth
/// file appears in `ranked` or `ground_truth` is empty. Only the first hit
/// counts — that is the standard single-query MRR contribution.
pub fn mrr(ranked: &[String], ground_truth: &HashSet<String>) -> f64 {
    if ground_truth.is_empty() {
        return 0.0;
    }
    for (i, path) in ranked.iter().enumerate() {
        if ground_truth.contains(path) {
            return 1.0 / (i as f64 + 1.0);
        }
    }
    0.0
}

/// A row in the comparison table: one retrieval system's mean metrics over the
/// corpus subset that was run.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemResult {
    pub system: String,
    pub recall_8k: f64,
    pub recall_32k: f64,
    pub mrr: f64,
}

/// Render the comparison table as deterministic markdown.
///
/// Rows are emitted in the order given by `results` (the caller fixes the
/// ordering). A header row is always emitted, even for an empty input, so the
/// output is never blank.
pub fn render_comparison_table(results: &[SystemResult]) -> String {
    let mut out = String::new();
    out.push_str("| system | recall@8k | recall@32k | MRR |\n");
    out.push_str("|---|---|---|---|\n");
    for r in results {
        out.push_str(&format!(
            "| {} | {:.3} | {:.3} | {:.3} |\n",
            r.system, r.recall_8k, r.recall_32k, r.mrr
        ));
    }
    out
}

/// Compute the ground-truth set (files that EXIST at `base_sha`) from the raw
/// `gh api .../files` records.
///
/// See the module-level note on the base-tree nuance:
///   * `added`            → excluded (does not exist at base).
///   * `modified`         → kept (current path exists at base).
///   * `removed`/`deleted`→ kept (path exists at base; the PR deletes it).
///   * `renamed`          → kept under `previous_filename` (the base-tree path).
///     Falls back to `filename` only if the previous path is absent (defensive
///     — GitHub always supplies it for a rename).
pub fn ground_truth_at_base(files: &[ChangedFile]) -> HashSet<String> {
    let mut gt = HashSet::new();
    for f in files {
        match f.status.as_str() {
            "added" => {}
            "renamed" => {
                let path = f
                    .previous_filename
                    .clone()
                    .unwrap_or_else(|| f.filename.clone());
                gt.insert(path);
            }
            // modified, removed, changed, copied, unchanged, or any unknown
            // status: the named path exists at base, so keep it.
            _ => {
                gt.insert(f.filename.clone());
            }
        }
    }
    gt
}

/// One changed-file record from the GitHub PR files API.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ChangedFile {
    pub filename: String,
    pub status: String,
    #[serde(default)]
    pub previous_filename: Option<String>,
}

// ===========================================================================
// Gated harness (network + disk — only invoked from the env-gated smoke test)
// ===========================================================================

/// Token budgets at which recall is measured (the gate metrics for C2/D1).
pub const BUDGET_8K: usize = 8_000;
pub const BUDGET_32K: usize = 32_000;

/// Run the full recall harness over `entries` and aggregate per-system means.
///
/// Per entry: fetch the repo at `base_sha` into a gitignored cache, build a
/// cxpak index, fetch the PR's changed files, derive the base-tree ground
/// truth, and run every system at both budgets. Entries whose fetch/index/GT
/// derivation fails are skipped (with a stderr note) rather than aborting the
/// whole run — a single dead corpus entry must not sink the benchmark.
///
/// Returns one [`SystemResult`] per system, with metrics averaged over the
/// entries that ran. Errors only on a total wipe-out (no entry produced data).
pub fn run_harness(entries: &[CorpusEntry], repo_root: &Path) -> Result<Vec<SystemResult>, String> {
    // Accumulators keyed by system name → (sum_recall_8k, sum_recall_32k, sum_mrr).
    // Order MUST match the row order produced by `run_entry`.
    //
    // Two cxpak rows, clearly labeled:
    //   * "cxpak (auto_context)"     — the shipped product C2/D1 gate against:
    //     seed selection + 1-hop graph fan-out + noise filtering, then budgeting.
    //   * "cxpak (score_all ranking)" — the raw multi-signal ranking, kept as the
    //     fair apples-to-apples cross-system comparison vs the other baselines.
    let systems = [
        "cxpak (auto_context)",
        "cxpak (score_all ranking)",
        "ripgrep",
        "embeddings-only",
        "repomap (PageRank proxy)",
    ];
    let mut acc: Vec<(f64, f64, f64)> = vec![(0.0, 0.0, 0.0); systems.len()];
    let mut counted = 0usize;

    for entry in entries {
        match run_entry(entry, repo_root) {
            Ok(per_system) => {
                for (i, m) in per_system.iter().enumerate() {
                    acc[i].0 += m.0;
                    acc[i].1 += m.1;
                    acc[i].2 += m.2;
                }
                counted += 1;
            }
            Err(e) => {
                eprintln!(
                    "skipping {}/{} ({}): {}",
                    entry.repo, entry.pr, entry.lang, e
                );
            }
        }
    }

    if counted == 0 {
        return Err("no corpus entry produced data (all fetch/index attempts failed)".to_string());
    }

    let n = counted as f64;
    Ok(systems
        .iter()
        .enumerate()
        .map(|(i, name)| SystemResult {
            system: (*name).to_string(),
            recall_8k: acc[i].0 / n,
            recall_32k: acc[i].1 / n,
            mrr: acc[i].2 / n,
        })
        .collect())
}

/// Run every system for a single corpus entry, returning per-system
/// `(recall_8k, recall_32k, mrr)` triples in the fixed system order used by
/// [`run_harness`].
fn run_entry(entry: &CorpusEntry, repo_root: &Path) -> Result<Vec<(f64, f64, f64)>, String> {
    // 1. Fetch the repo at base_sha (cached) and index it.
    let checkout = ensure_repo_at_base(entry, repo_root)?;
    let index = crate::commands::serve::build_index(&checkout)
        .map_err(|e| format!("index build failed: {e}"))?;

    // 2. Ground truth = changed files that exist at base.
    let changed = fetch_changed_files_with_status(entry)?;
    let ground_truth = ground_truth_at_base(&changed);

    // 3. Query string from the PR title (fallback to a repo/pr tag).
    let query = entry
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| format!("{} pr {}", entry.repo, entry.pr));

    // Pre-compute the single global ranked list once; every ranking-based
    // system slices/reorders from index data, so this is shared work.
    let ranked_cxpak = rank_cxpak(&query, &index);
    let ranked_embeddings = rank_embeddings_only(&query, &index);
    let ranked_ripgrep = rank_ripgrep(&query, &checkout, &index);
    let ranked_repomap = rank_repomap_proxy(&index);

    // 4. Ranking-based systems: take the budget prefix for recall and the full
    //    ranked list for MRR.
    let row = |ranked: &[(String, usize)]| -> (f64, f64, f64) {
        let sel_8k = take_within_budget(ranked, BUDGET_8K);
        let sel_32k = take_within_budget(ranked, BUDGET_32K);
        let order: Vec<String> = ranked.iter().map(|(p, _)| p.clone()).collect();
        (
            recall_at_budget(&sel_8k, &ground_truth),
            recall_at_budget(&sel_32k, &ground_truth),
            mrr(&order, &ground_truth),
        )
    };

    // 5. cxpak (auto_context): the SHIPPED product C2/D1 gate against. Unlike the
    //    raw `score_all` ranking, auto_context applies seed selection + 1-hop
    //    graph fan-out + noise filtering before its OWN budget allocation, so we
    //    call it once per budget and read back the files it actually selected
    //    (`sections.target_files`) — no external `take_within_budget`.
    //    MRR is undefined for a budget-selected *set* (no global rank beyond the
    //    cutoff), so we report MRR on the underlying `score_all` ranking — the
    //    same ordering auto_context's seed selection consumes — and document it.
    let ac_mrr = {
        let order: Vec<String> = ranked_cxpak.iter().map(|(p, _)| p.clone()).collect();
        mrr(&order, &ground_truth)
    };
    let ac_recall = |budget: usize| -> f64 {
        let opts = crate::auto_context::AutoContextOpts {
            tokens: budget,
            focus: None,
            include_tests: false,
            include_blast_radius: false,
            mode: "full".to_string(),
            cost_model: None,
        };
        let result = crate::auto_context::auto_context(&query, &index, &opts);
        let selected: Vec<String> = result
            .sections
            .target_files
            .files
            .iter()
            .map(|f| f.path.clone())
            .collect();
        recall_at_budget(&selected, &ground_truth)
    };
    let row_auto_context = (ac_recall(BUDGET_8K), ac_recall(BUDGET_32K), ac_mrr);

    Ok(vec![
        row_auto_context,
        row(&ranked_cxpak),
        row(&ranked_ripgrep),
        row(&ranked_embeddings),
        row(&ranked_repomap),
    ])
}

/// Pack file paths from the ranked list within `budget` tokens, skipping any
/// file that would overflow and continuing with later (smaller) files rather
/// than stopping at the first overflow. Rank order is authoritative; a file is
/// included iff it fits in the remaining budget. This skip-and-continue policy
/// is applied uniformly across every ranking-based system, so it cannot bias the
/// cross-system comparison.
fn take_within_budget(ranked: &[(String, usize)], budget: usize) -> Vec<String> {
    let mut used = 0usize;
    let mut selected = Vec::new();
    for (path, tokens) in ranked {
        if used + tokens > budget {
            continue;
        }
        used += tokens;
        selected.push(path.clone());
    }
    selected
}

/// cxpak's raw multi-signal ranking — the "cxpak (score_all ranking)" row.
///
/// This is the underlying `MultiSignalScorer` ranking that auto_context's seed
/// selection consumes, BEFORE auto_context layers on seed selection, 1-hop graph
/// fan-out, and noise filtering. Kept as the fair apples-to-apples cross-system
/// comparison against ripgrep / embeddings-only / repomap; the separate
/// "cxpak (auto_context)" row measures the shipped product end-to-end. Sorted by
/// score descending with a path tiebreak for determinism. Returns
/// `(relative_path, token_count)` pairs.
fn rank_cxpak(query: &str, index: &crate::index::CodebaseIndex) -> Vec<(String, usize)> {
    let expanded = crate::context_quality::expansion::expand_query(query, &index.domains);
    let scorer = crate::relevance::MultiSignalScorer::new_for_index(index).with_expansion(expanded);
    let mut scored = scorer.score_all(query, index);
    scored.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.path.cmp(&b.path)));
    scored
        .into_iter()
        .map(|s| (s.path, s.token_count))
        .collect()
}

/// Embeddings-only baseline: rank purely by the embedding-similarity signal.
///
/// With the `embeddings` feature (on by default), this embeds the query once
/// and scores every file by cosine similarity to its stored embedding. Without
/// the feature — or with no embedding index — every file scores a neutral 0.5,
/// so the ranking degenerates to the deterministic path tiebreak; that is an
/// honest "no semantic signal available" baseline, not a silent cxpak fallback.
fn rank_embeddings_only(query: &str, index: &crate::index::CodebaseIndex) -> Vec<(String, usize)> {
    #[cfg(feature = "embeddings")]
    let query_embedding: Option<Vec<f32>> = {
        use crate::embeddings::{create_provider, EmbeddingConfig};
        create_provider(EmbeddingConfig::local_default())
            .ok()
            .and_then(|p| p.embed(query).ok())
    };

    let mut scored: Vec<(String, usize, f64)> = index
        .files
        .iter()
        .map(|f| {
            #[cfg(feature = "embeddings")]
            let score = crate::relevance::signals::embedding_similarity_signal(
                query_embedding.as_deref(),
                &f.relative_path,
                index,
            )
            .score;
            #[cfg(not(feature = "embeddings"))]
            let score = {
                let _ = query;
                0.5_f64
            };
            (f.relative_path.clone(), f.token_count, score)
        })
        .collect();

    scored.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
    scored.into_iter().map(|(p, t, _)| (p, t)).collect()
}

/// ripgrep baseline: rank files by keyword-hit count from the query terms.
///
/// Splits the query into alphanumeric keywords (length ≥ 3) and runs a single
/// `rg --count-matches` over the checkout, summing hits per file. Files are
/// ordered by total hit count descending, path ascending; files with no hits
/// fall to the end in path order so the ranking still spans the whole repo.
fn rank_ripgrep(
    query: &str,
    checkout: &Path,
    index: &crate::index::CodebaseIndex,
) -> Vec<(String, usize)> {
    use std::collections::HashMap;

    let keywords: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_lowercase())
        .collect();

    let mut hits: HashMap<String, u64> = HashMap::new();
    if !keywords.is_empty() {
        // Build a single alternation pattern: (kw1|kw2|...), case-insensitive.
        let pattern = keywords.join("|");
        let out = Command::new("rg")
            .args(["--count-matches", "--no-messages", "-i", "-e", &pattern])
            .current_dir(checkout)
            .output();
        if let Ok(out) = out {
            // Each line: "relative/path:NN". rg uses the cwd-relative path.
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                if let Some((path, count)) = line.rsplit_once(':') {
                    let path = path.replace('\\', "/");
                    if let Ok(n) = count.trim().parse::<u64>() {
                        *hits.entry(path).or_insert(0) += n;
                    }
                }
            }
        }
    }

    // Rank all indexed files: by hit count desc, then path asc. Files with no
    // rg hit get count 0 and sort to the tail, but remain in the ranking.
    let mut ranked: Vec<(String, usize, u64)> = index
        .files
        .iter()
        .map(|f| {
            let h = hits.get(&f.relative_path).copied().unwrap_or(0);
            (f.relative_path.clone(), f.token_count, h)
        })
        .collect();
    ranked.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    ranked.into_iter().map(|(p, t, _)| (p, t)).collect()
}

/// aider-repomap proxy: rank files by PageRank over the dependency graph.
///
/// aider's repo-map ranks files by a personalized PageRank over the symbol/
/// reference graph; the ranking core IS PageRank centrality. cxpak already
/// computes `index.pagerank` over its dependency graph, so this proxy uses that
/// directly. Labeled "repomap (PageRank proxy)" in the table — aider-proper
/// (its tree-sitter repo-map + chat-history personalization) is deferred.
fn rank_repomap_proxy(index: &crate::index::CodebaseIndex) -> Vec<(String, usize)> {
    let mut ranked: Vec<(String, usize, f64)> = index
        .files
        .iter()
        .map(|f| {
            let pr = index.pagerank.get(&f.relative_path).copied().unwrap_or(0.0);
            (f.relative_path.clone(), f.token_count, pr)
        })
        .collect();
    ranked.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
    ranked.into_iter().map(|(p, t, _)| (p, t)).collect()
}

// ── repo fetch + checkout (gitignored cache) ───────────────────────────────

/// Ensure the repo for `entry` is checked out at `base_sha` in a gitignored
/// cache dir under `target/bench-repos/`, returning the checkout path.
///
/// Idempotent: if a non-empty checkout already exists for this `(repo, sha)`,
/// it is reused without touching the network. Otherwise a fresh shallow fetch of
/// the single commit is performed and checked out at a detached HEAD. Fetched
/// source is NEVER committed — `target/` is gitignored.
fn ensure_repo_at_base(entry: &CorpusEntry, repo_root: &Path) -> Result<PathBuf, String> {
    let slug = entry.repo.replace('/', "__");
    let dir = repo_root
        .join("target")
        .join("bench-repos")
        .join(format!("{slug}@{}", entry.base_sha));

    // Cache hit: a populated checkout already exists.
    if dir.join(".git").is_dir() {
        return Ok(dir);
    }

    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;

    let url = format!("https://github.com/{}.git", entry.repo);

    run_git(&dir, &["init", "-q"])?;
    run_git(&dir, &["remote", "add", "origin", &url])
        // remote may already exist on a retry after a partial fetch.
        .or_else(|_| run_git(&dir, &["remote", "set-url", "origin", &url]))?;
    // Shallow-fetch only the one commit we need.
    run_git(
        &dir,
        &["fetch", "--depth", "1", "-q", "origin", &entry.base_sha],
    )?;
    run_git(&dir, &["checkout", "-q", "-f", &entry.base_sha])?;

    Ok(dir)
}

/// Run a git subcommand in `dir`, mapping a non-zero exit to a descriptive error.
fn run_git(dir: &Path, args: &[&str]) -> Result<(), String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "git not found on PATH".to_string()
            } else {
                format!("failed to spawn git {args:?}: {e}")
            }
        })?;
    if !out.status.success() {
        return Err(format!(
            "git {args:?} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Fetch the PR's changed files WITH their status (added/modified/removed/
/// renamed) so ground truth can exclude added files. Shells out to `gh api`.
fn fetch_changed_files_with_status(entry: &CorpusEntry) -> Result<Vec<ChangedFile>, String> {
    let endpoint = format!("repos/{}/pulls/{}/files", entry.repo, entry.pr);
    let output = Command::new("gh")
        .args([
            "api",
            &endpoint,
            "--paginate",
            "--jq",
            ".[] | {filename, status, previous_filename}",
        ])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "gh CLI not found on PATH".to_string()
            } else {
                format!("failed to spawn gh: {e}")
            }
        })?;

    if !output.status.success() {
        return Err(format!(
            "gh api {endpoint} exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    // `--jq '.[] | {...}'` emits one JSON object per line (JSON Lines).
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files = Vec::new();
    for line in stdout.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let f: ChangedFile = serde_json::from_str(line)
            .map_err(|e| format!("parse gh file record '{line}': {e}"))?;
        files.push(f);
    }
    if files.is_empty() {
        return Err(format!(
            "gh api returned no files for {}/{}",
            entry.repo, entry.pr
        ));
    }
    Ok(files)
}
