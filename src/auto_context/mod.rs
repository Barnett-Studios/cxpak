pub mod briefing;
pub mod diff;
pub mod noise;

use crate::auto_context::noise::{FilteredFile, ScoredFileEntry};
use crate::index::CodebaseIndex;
use crate::intelligence::architecture::ArchitectureMap;
use crate::intelligence::co_change::CoChangeEdge;
use crate::intelligence::health::HealthScore;
use crate::intelligence::recent_changes::RecentChange;
use crate::intelligence::risk::RiskEntry;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct AutoContextOpts {
    pub tokens: usize,
    pub focus: Option<String>,
    pub include_tests: bool,
    pub include_blast_radius: bool,
}

#[derive(Debug, Serialize)]
pub struct AutoContextResult {
    pub task: String,
    pub dna: String,
    pub budget: crate::auto_context::briefing::BudgetSummary,
    pub sections: crate::auto_context::briefing::PackedSections,
    pub filtered_out: Vec<FilteredFile>,
    // v1.2.0 compound intelligence
    pub health: HealthScore,
    pub risks: Vec<RiskEntry>,
    pub architecture: ArchitectureMap,
    pub co_changes: Vec<CoChangeEdge>,
    pub recent_changes: Vec<RecentChange>,
}

// ---------------------------------------------------------------------------
// Orchestration pipeline
// ---------------------------------------------------------------------------

/// Run the full auto-context pipeline for `task` against `index`.
///
/// Steps (in order):
/// 1. Query expansion
/// 2. Multi-signal relevance scoring
/// 3. Seed selection + 1-hop fan-out
/// 4. Convert to `ScoredFileEntry` + noise filtering
/// 5. Optional focus path filter
/// 6. Resolve file content for target section
/// 7. Resolve test files (when `opts.include_tests`)
/// 8. Optional blast-radius computation (top-5 files)
/// 9. Optional API surface extraction
/// 10. Fill-then-overflow budget allocation via `briefing::allocate_and_pack`
pub fn auto_context(
    task: &str,
    index: &CodebaseIndex,
    opts: &AutoContextOpts,
) -> AutoContextResult {
    // Step 0: DNA section — render convention profile, deduct from budget.
    let counter = crate::budget::counter::TokenCounter::new();
    let (effective_dna, dna_token_cost) = if opts.tokens < 2000 {
        (String::new(), 0)
    } else if opts.tokens < 5000 {
        let compact = crate::conventions::render::render_compact_dna(&index.conventions);
        let cost = counter.count(&compact);
        (compact, cost)
    } else {
        let dna = crate::conventions::render::render_dna_section(&index.conventions);
        let cost = counter.count(&dna);
        (dna, cost)
    };
    let remaining_budget = opts.tokens.saturating_sub(dna_token_cost);

    // Step 1: Query expansion
    let expanded = crate::context_quality::expansion::expand_query(task, &index.domains);

    // Step 2: Relevance scoring — select weights based on index capabilities.
    let scorer = crate::relevance::MultiSignalScorer::new_for_index(index).with_expansion(expanded);
    let all_scored = scorer.score_all(task, index);

    // Step 3: Seed selection + fan-out via prebuilt graph.
    let seeds = crate::relevance::seed::select_seeds_with_graph(
        &all_scored,
        index,
        crate::relevance::seed::SEED_THRESHOLD,
        50,
        Some(&index.graph),
    );

    // Step 4: Convert to ScoredFileEntry and run noise filter.
    let candidates: Vec<ScoredFileEntry> = seeds
        .iter()
        .map(|s| ScoredFileEntry {
            path: s.path.clone(),
            score: s.score,
            token_count: s.token_count,
        })
        .collect();
    let filtered = crate::auto_context::noise::filter_noise(candidates, index, &index.pagerank);

    // Step 5: Optional focus filter — keep only files under the focus prefix.
    let mut kept = filtered.kept;
    if let Some(ref focus) = opts.focus {
        kept.retain(|f| f.path.starts_with(focus.as_str()));
    }

    // Step 6: Resolve target file content from the index.
    let target_files: Vec<(String, f64, String)> = kept
        .iter()
        .filter_map(|entry| {
            index
                .files
                .iter()
                .find(|f| f.relative_path == entry.path)
                .map(|f| (f.relative_path.clone(), entry.score, f.content.clone()))
        })
        .collect();

    // Step 7: Resolve test files.
    let test_files: Vec<(String, String)> = if opts.include_tests {
        let mut tests: Vec<(String, String)> = Vec::new();
        for entry in &kept {
            if let Some(test_refs) = index.test_map.get(&entry.path) {
                for tr in test_refs {
                    if let Some(f) = index.files.iter().find(|f| f.relative_path == tr.path) {
                        tests.push((f.relative_path.clone(), f.content.clone()));
                    }
                }
            }
        }
        // Stable sort + dedup so the output is deterministic.
        tests.sort_by(|a, b| a.0.cmp(&b.0));
        tests.dedup_by(|a, b| a.0 == b.0);
        tests
    } else {
        vec![]
    };

    // Step 8: Optional blast radius (top 5 target files).
    let blast_json = if opts.include_blast_radius && !kept.is_empty() {
        let top_paths: Vec<&str> = kept.iter().take(5).map(|f| f.path.as_str()).collect();
        let result = crate::intelligence::blast_radius::compute_blast_radius(
            &top_paths,
            &index.graph,
            &index.pagerank,
            &index.test_map,
            3,
            opts.focus.as_deref(),
        );
        serde_json::to_value(&result).ok()
    } else {
        None
    };

    // Step 9: API surface extraction.
    let api_json = {
        let api = crate::intelligence::api_surface::extract_api_surface(
            index,
            opts.focus.as_deref(),
            "all",
            0,
        );
        serde_json::to_value(&api).ok()
    };

    // Step 10: Pack with budget allocation (budget minus DNA tokens).
    let packed = crate::auto_context::briefing::allocate_and_pack(
        target_files,
        test_files,
        None,
        api_json,
        blast_json,
        remaining_budget,
    );

    // Compound intelligence (computed after packing to avoid double-borrowing index)
    let health = crate::intelligence::health::compute_health(index);
    let all_risks = crate::intelligence::risk::compute_risk_ranking(index);
    let risks: Vec<RiskEntry> = all_risks.into_iter().take(10).collect();
    let architecture = crate::intelligence::architecture::build_architecture_map(index, 2);
    let co_changes = index.co_changes.clone();
    let recent_changes = crate::intelligence::recent_changes::compute_recent_changes(index);

    AutoContextResult {
        task: task.to_string(),
        dna: effective_dna,
        budget: packed.budget,
        sections: packed.sections,
        filtered_out: filtered.filtered_out,
        health,
        risks,
        architecture,
        co_changes,
        recent_changes,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    /// Build a minimal `CodebaseIndex` from in-memory content slices.
    fn make_index(paths: &[(&str, &str)]) -> (CodebaseIndex, tempfile::TempDir) {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let files: Vec<ScannedFile> = paths
            .iter()
            .map(|(rel, content)| {
                // Use flat filenames under the temp dir to avoid directory
                // creation complexity.
                let safe = rel.replace('/', "_");
                let abs = dir.path().join(&safe);
                std::fs::write(&abs, content).unwrap();
                ScannedFile {
                    relative_path: rel.to_string(),
                    absolute_path: abs,
                    language: Some("rust".into()),
                    size_bytes: content.len() as u64,
                }
            })
            .collect();

        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        (index, dir)
    }

    fn default_opts(tokens: usize) -> AutoContextOpts {
        AutoContextOpts {
            tokens,
            focus: None,
            include_tests: false,
            include_blast_radius: false,
        }
    }

    // -----------------------------------------------------------------------
    // test_auto_context_happy_path
    // -----------------------------------------------------------------------
    #[test]
    fn test_auto_context_happy_path() {
        let (index, _dir) = make_index(&[
            (
                "src/auth.rs",
                "pub fn authenticate(user: &str) -> bool { true }",
            ),
            (
                "src/session.rs",
                "pub fn create_session(id: u64) -> Session { todo!() }",
            ),
        ]);
        let opts = default_opts(50_000);
        let result = auto_context("user authentication login", &index, &opts);

        assert_eq!(result.task, "user authentication login");
        assert_eq!(
            result.budget.used + result.budget.remaining,
            result.budget.total
        );
        // Budget total is opts.tokens minus DNA token cost.
        assert!(result.budget.total <= 50_000);
        // Full mode: packed file content must always be Some.
        for file in &result.sections.target_files.files {
            assert!(
                file.content.is_some(),
                "full-mode file content must be Some"
            );
        }
        assert!(
            result.health.composite >= 0.0 && result.health.composite <= 10.0,
            "health composite out of range: {}",
            result.health.composite
        );
        assert!(result.risks.len() <= 10, "risks should be capped at 10");
    }

    // -----------------------------------------------------------------------
    // test_auto_context_empty_repo
    // -----------------------------------------------------------------------
    #[test]
    fn test_auto_context_empty_repo() {
        let (index, _dir) = make_index(&[]);
        let opts = default_opts(10_000);
        let result = auto_context("anything", &index, &opts);

        // No files → no packed content in the file sections.
        assert!(result.sections.target_files.files.is_empty());
        assert!(result.sections.test_files.files.is_empty());
        assert_eq!(result.sections.target_files.tokens, 0);
        assert_eq!(result.sections.test_files.tokens, 0);
        // Budget invariant always holds regardless of API surface overhead.
        assert_eq!(
            result.budget.used + result.budget.remaining,
            result.budget.total
        );
    }

    // -----------------------------------------------------------------------
    // test_auto_context_focus_param
    // -----------------------------------------------------------------------
    #[test]
    fn test_auto_context_focus_param() {
        let (index, _dir) = make_index(&[
            ("src/api/handler.rs", "pub fn handle() {}"),
            ("src/db/query.rs", "pub fn run_query() {}"),
        ]);
        let opts = AutoContextOpts {
            tokens: 50_000,
            focus: Some("src/api/".to_string()),
            include_tests: false,
            include_blast_radius: false,
        };
        let result = auto_context("handler", &index, &opts);

        // Every packed target must be under the focus prefix.
        for file in &result.sections.target_files.files {
            assert!(
                file.path.starts_with("src/api/"),
                "file {} is outside the focus prefix",
                file.path
            );
        }
    }

    // -----------------------------------------------------------------------
    // test_auto_context_noise_filtered
    // -----------------------------------------------------------------------
    /// Verify that files under vendor/ are excluded by the noise filter even
    /// when they score highly for the task query.  We make the vendor file
    /// intentionally relevant to the query so that it passes the relevance
    /// threshold and reaches the noise-filter stage where it should be
    /// blocked by the path-pattern blocklist.
    #[test]
    fn test_auto_context_noise_filtered() {
        let (index, _dir) = make_index(&[
            // Matches query strongly (path + symbols), but is in vendor/ → blocklisted.
            (
                "vendor/auth.rs",
                "pub fn authenticate(user: &str) -> bool { check_credentials(user) }",
            ),
            (
                "src/auth.rs",
                "pub fn authenticate(user: &str) -> bool { true }",
            ),
        ]);
        let opts = default_opts(50_000);
        let result = auto_context("authenticate user credentials", &index, &opts);

        // vendor/auth.rs must not appear in packed target files.
        let vendor_packed = result
            .sections
            .target_files
            .files
            .iter()
            .any(|f| f.path == "vendor/auth.rs");
        assert!(
            !vendor_packed,
            "vendor/auth.rs must not appear in packed files"
        );

        // vendor/auth.rs should either have been filtered by the noise filter
        // (appears in filtered_out) or never reached seed selection due to a
        // low relevance score — either way it must not be in the output.
        // If it did reach the noise filter, verify it appears in filtered_out.
        let in_filtered_out = result
            .filtered_out
            .iter()
            .any(|f| f.path == "vendor/auth.rs");
        if in_filtered_out {
            let reason = result
                .filtered_out
                .iter()
                .find(|f| f.path == "vendor/auth.rs")
                .map(|f| f.reason.as_str())
                .unwrap_or("");
            assert!(
                reason.starts_with("blocklist:"),
                "vendor/ file should have blocklist reason, got: {reason}"
            );
        }
        // Whether filtered by noise or never selected, it must not be packed.
        assert!(
            !vendor_packed,
            "vendor/auth.rs must never appear in packed target files"
        );
    }

    // -----------------------------------------------------------------------
    // test_auto_context_budget_summary
    // -----------------------------------------------------------------------
    #[test]
    fn test_auto_context_budget_summary() {
        let (index, _dir) = make_index(&[
            ("src/a.rs", "pub fn alpha() {}"),
            ("src/b.rs", "pub fn beta() {}"),
        ]);
        let opts = default_opts(50_000);
        let result = auto_context("alpha beta", &index, &opts);

        assert_eq!(
            result.budget.used + result.budget.remaining,
            result.budget.total,
            "budget invariant: used + remaining = total"
        );
    }

    // -----------------------------------------------------------------------
    // test_auto_context_no_tests_flag
    // -----------------------------------------------------------------------
    #[test]
    fn test_auto_context_no_tests_flag() {
        let (index, _dir) = make_index(&[
            ("src/handler.rs", "pub fn handle_request() {}"),
            (
                "tests/handler_test.rs",
                "fn test_handle() { assert!(true); }",
            ),
        ]);
        let opts = AutoContextOpts {
            tokens: 50_000,
            focus: None,
            include_tests: false,
            include_blast_radius: false,
        };
        let result = auto_context("handle request", &index, &opts);

        assert_eq!(
            result.sections.test_files.count, 0,
            "include_tests=false should produce no test_files"
        );
        assert!(result.sections.test_files.files.is_empty());
    }
}
