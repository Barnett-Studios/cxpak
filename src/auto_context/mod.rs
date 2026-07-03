pub mod briefing;
pub mod diff;
pub mod efficiency;
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
    pub mode: String, // "full" (default) or "briefing"
    /// Opt-in cost model name; when `Some`, the efficiency report includes a USD estimate.
    pub cost_model: Option<String>,
}

/// Version of the `auto_context` output contract (ADR-0169). Bump on a breaking
/// change to the serialized shape; additive fields do not require a bump.
pub const FORMAT_VERSION: &str = "2.3";

#[derive(Debug, Serialize)]
pub struct AutoContextResult {
    /// Version of this output contract (see [`FORMAT_VERSION`]).
    pub format_version: &'static str,
    pub task: String,
    pub dna: String,
    pub budget: crate::auto_context::briefing::BudgetSummary,
    pub sections: crate::auto_context::briefing::PackedSections,
    pub filtered_out: Vec<FilteredFile>,
    // v2.3.0 efficiency / cost reporting (ADR-0168)
    pub efficiency: crate::auto_context::efficiency::EfficiencyReport,
    // v1.2.0 compound intelligence
    pub health: HealthScore,
    pub risks: Vec<RiskEntry>,
    pub architecture: ArchitectureMap,
    pub co_changes: Vec<CoChangeEdge>,
    pub recent_changes: Vec<RecentChange>,
    // v1.4.0 prediction
    pub predictions: Option<crate::intelligence::predict::PredictionResult>,
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
    // Ships with the inert (pre-D1) ranking; the D1 semantic upgrade is gated on
    // the D2 recall A/B (ADR-0184). `auto_context_with_mode` exposes the control.
    auto_context_with_mode(task, index, opts, crate::relevance::DEFAULT_RELEVANCE_MODE)
}

/// Like [`auto_context`], but with an explicit [`RelevanceMode`] — the
/// product-level half of the D1 A/B control (ADR-0184). `Inert` reproduces the
/// pre-D1 ranking byte-for-byte; `Active` enables RRF fusion. The D2 recall
/// harness calls this with both modes over a SINGLE index build so the semantic
/// upgrade is measurable index-once (the C2 reaping lesson).
///
/// [`RelevanceMode`]: crate::relevance::RelevanceMode
pub fn auto_context_with_mode(
    task: &str,
    index: &CodebaseIndex,
    opts: &AutoContextOpts,
    mode: crate::relevance::RelevanceMode,
) -> AutoContextResult {
    // Step 0: DNA section — render convention profile, deduct from budget.
    // If the DNA cost meets or exceeds the total budget, fall back to empty
    // DNA so that all budget tiers can still produce content sections.
    let counter = crate::budget::counter::TokenCounter::new();
    let (effective_dna, dna_token_cost) = if opts.tokens < 2000 {
        (String::new(), 0)
    } else if opts.tokens < 5000 {
        let compact = crate::conventions::render::render_compact_dna(&index.conventions);
        let cost = counter.count(&compact);
        if cost >= opts.tokens {
            (String::new(), 0)
        } else {
            (compact, cost)
        }
    } else {
        let dna = crate::conventions::render::render_dna_section(&index.conventions);
        let cost = counter.count(&dna);
        if cost >= opts.tokens {
            (String::new(), 0)
        } else {
            (dna, cost)
        }
    };
    let remaining_budget = opts.tokens.saturating_sub(dna_token_cost);

    // Step 1: Query expansion
    let expanded = crate::context_quality::expansion::expand_query(task, &index.domains);

    // Step 2: Relevance scoring — select weights based on index capabilities.
    let scorer = crate::relevance::MultiSignalScorer::new_for_index(index)
        .with_expansion(expanded)
        .with_mode(mode);
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
    //
    // Two sources of tests are combined:
    // 1. Separate test files referenced by `index.test_map` (the common case
    //    for languages with dedicated test directories).
    // 2. Seed files that *are their own tests* — i.e. files containing inline
    //    test blocks (e.g. Rust `#[cfg(test)]`, Python `class Test…`, etc.).
    //    These files are already in `target_files`, but we also list them in
    //    the `test_files` section so consumers know inline tests are present.
    let test_files: Vec<(String, String)> = if opts.include_tests {
        let mut tests: Vec<(String, String)> = Vec::new();
        for entry in &kept {
            // Source 1: explicitly mapped test files.
            if let Some(test_refs) = index.test_map.get(&entry.path) {
                for tr in test_refs {
                    if let Some(f) = index.files.iter().find(|f| f.relative_path == tr.path) {
                        tests.push((f.relative_path.clone(), f.content.clone()));
                    }
                }
            }
            // Source 2: seed file contains inline tests (e.g. Rust #[cfg(test)]).
            // Include the file itself so the test section is populated for repos
            // that co-locate tests with source instead of using separate files.
            if let Some(f) = index.files.iter().find(|f| f.relative_path == entry.path) {
                if crate::intelligence::health::has_inline_tests(f) {
                    tests.push((f.relative_path.clone(), f.content.clone()));
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

    // Step 9.5 (v1.5.0): serialize cross-language edges, optionally filtered
    // by focus prefix. Edges where either endpoint falls within the focus
    // scope are kept.
    let cross_lang_json = {
        let filtered: Vec<&crate::intelligence::cross_lang::CrossLangEdge> = if let Some(prefix) =
            opts.focus.as_deref()
        {
            index
                .cross_lang_edges
                .iter()
                .filter(|e| e.source_file.starts_with(prefix) || e.target_file.starts_with(prefix))
                .collect()
        } else {
            index.cross_lang_edges.iter().collect()
        };
        if filtered.is_empty() {
            None
        } else {
            serde_json::to_value(&filtered).ok()
        }
    };

    // Step 10: Pack with budget allocation (budget minus DNA tokens).
    let briefing_mode = opts.mode == "briefing";
    let packed = crate::auto_context::briefing::allocate_and_pack_with_cross_lang(
        target_files,
        test_files,
        None,
        api_json,
        blast_json,
        cross_lang_json,
        remaining_budget,
        briefing_mode,
    );

    // Compound intelligence (computed after packing to avoid double-borrowing index)
    let health = crate::intelligence::health::compute_health(index);
    let all_risks = crate::intelligence::risk::compute_risk_ranking(index);
    let risks: Vec<RiskEntry> = all_risks.into_iter().take(10).collect();
    let architecture = crate::intelligence::architecture::build_architecture_map(index, 2);
    let co_changes = index.co_changes.clone();
    let recent_changes = crate::intelligence::recent_changes::compute_recent_changes(index);

    // v1.4.0: detect file mentions in task and compute predictions
    let predictions = {
        let ext_re =
            regex::Regex::new(r"\b[\w/.-]+\.(?:rs|ts|js|py|go|java|rb|c|cpp|h|cs|swift|kt)\b").ok();
        let slash_re = regex::Regex::new(r"\b(?:src|lib|tests?|spec|app|pkg)/[\w/.-]+\b").ok();

        let mut mentions: Vec<&str> = Vec::new();
        if let Some(re) = &ext_re {
            for m in re.find_iter(task) {
                mentions.push(m.as_str());
            }
        }
        if let Some(re) = &slash_re {
            for m in re.find_iter(task) {
                if !mentions.contains(&m.as_str()) {
                    mentions.push(m.as_str());
                }
            }
        }
        mentions.retain(|p| index.files.iter().any(|f| f.relative_path == *p));

        if !mentions.is_empty() {
            Some(crate::intelligence::predict::predict(
                &mentions,
                &index.graph,
                &index.pagerank,
                &index.co_changes,
                &index.test_map,
                3,
            ))
        } else {
            None
        }
    };

    // Efficiency report (ADR-0168) — computed from data in scope, before `packed`
    // and `filtered` are moved into the result. `kept` is the relevant set.
    let selected_tokens = packed.sections.target_files.tokens
        + packed.sections.test_files.tokens
        + packed.sections.schema_context.tokens;
    let filtered_tokens: usize = filtered.filtered_out.iter().map(|f| f.tokens).sum();
    // A relevant candidate counts as "covered" if it was packed into ANY file
    // section — target, test, or schema — not just target_files (else a kept file
    // packed as a test would be miscounted as excluded, skewing coverage + advisory).
    let included: std::collections::HashSet<&str> = packed
        .sections
        .target_files
        .files
        .iter()
        .chain(packed.sections.test_files.files.iter())
        .chain(packed.sections.schema_context.files.iter())
        .map(|f| f.path.as_str())
        .collect();
    let relevant_total = kept.len();
    let relevant_covered = kept
        .iter()
        .filter(|e| included.contains(e.path.as_str()))
        .count();
    let marginal_included_score = kept
        .iter()
        .filter(|e| included.contains(e.path.as_str()))
        .map(|e| e.score)
        .fold(None, |acc: Option<f64>, s| {
            Some(acc.map_or(s, |a| a.min(s)))
        });
    let marginal_excluded_score = kept
        .iter()
        .filter(|e| !included.contains(e.path.as_str()))
        .map(|e| e.score)
        .fold(None, |acc: Option<f64>, s| {
            Some(acc.map_or(s, |a| a.max(s)))
        });
    let efficiency = crate::auto_context::efficiency::compute_efficiency(
        crate::auto_context::efficiency::EfficiencyInputs {
            repo_tokens: index.total_tokens,
            selected_tokens,
            budget_total: packed.budget.total,
            budget_used: packed.budget.used,
            filtered_tokens,
            relevant_total,
            relevant_covered,
            marginal_included_score,
            marginal_excluded_score,
            cost_model: opts.cost_model.clone(),
        },
    );

    AutoContextResult {
        format_version: FORMAT_VERSION,
        task: task.to_string(),
        dna: effective_dna,
        budget: packed.budget,
        sections: packed.sections,
        filtered_out: filtered.filtered_out,
        efficiency,
        health,
        risks,
        architecture,
        co_changes,
        recent_changes,
        predictions,
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
            cost_model: None,
            tokens,
            focus: None,
            include_tests: false,
            include_blast_radius: false,
            mode: "full".to_string(),
        }
    }

    #[test]
    fn opts_default_has_no_cost_model() {
        // Default opts must not request a cost estimate (cost is strictly opt-in).
        let opts = default_opts(10_000);
        assert!(opts.cost_model.is_none());
    }

    #[test]
    fn auto_context_default_equals_inert_mode() {
        // The plain `auto_context` entrypoint must delegate to the inert mode —
        // the D1 upgrade ships gated (ADR-0184), so the default product surface
        // is byte-identical to the pre-D1 ranking. Assert the selected file sets
        // match exactly.
        let (index, _dir) = make_index(&[
            ("src/api.rs", "pub fn handle_request() {}"),
            ("src/config.rs", "pub struct Config {}"),
        ]);
        let opts = default_opts(10_000);
        let default = auto_context("handle request", &index, &opts);
        let inert = auto_context_with_mode(
            "handle request",
            &index,
            &opts,
            crate::relevance::RelevanceMode::Inert,
        );
        let paths = |r: &AutoContextResult| -> Vec<String> {
            r.sections
                .target_files
                .files
                .iter()
                .map(|f| f.path.clone())
                .collect()
        };
        assert_eq!(paths(&default), paths(&inert));
    }

    #[test]
    fn auto_context_active_mode_runs_and_produces_valid_result() {
        // The Active (RRF) mode must run end-to-end and produce a well-formed
        // result — the measurable half of the D1 A/B control.
        let (index, _dir) = make_index(&[
            ("src/api.rs", "pub fn handle_request() {}"),
            ("src/config.rs", "pub struct Config {}"),
        ]);
        let opts = default_opts(10_000);
        let active = auto_context_with_mode(
            "handle request",
            &index,
            &opts,
            crate::relevance::RelevanceMode::Active,
        );
        assert_eq!(active.efficiency.repo_tokens, index.total_tokens);
        assert!(active.efficiency.selected_tokens <= active.efficiency.repo_tokens);
    }

    #[test]
    fn auto_context_result_has_efficiency_block() {
        let (index, _dir) =
            make_index(&[("src/a.rs", "pub fn a() {}"), ("src/b.rs", "pub fn b() {}")]);
        let opts = default_opts(10_000);
        let result = auto_context("explain a", &index, &opts);

        assert_eq!(result.efficiency.repo_tokens, index.total_tokens);
        assert!(result.efficiency.selected_tokens <= result.efficiency.repo_tokens);
        assert!(
            result.efficiency.relevant_coverage >= 0.0
                && result.efficiency.relevant_coverage <= 1.0
        );
        assert!(result.efficiency.relevant_covered <= result.efficiency.relevant_total);
        assert!(
            result.efficiency.absolute_coverage >= 0.0
                && result.efficiency.absolute_coverage <= 1.0
        );
        // marginal_included is Some when at least one relevant file was packed
        if result.efficiency.relevant_covered > 0 {
            assert!(result.efficiency.marginal_included_score.is_some());
        }
        assert!(result.efficiency.cost_estimate.is_none()); // default opts
    }

    #[test]
    fn result_advertises_format_version() {
        let (index, _dir) = make_index(&[("src/a.rs", "pub fn a() {}")]);
        let result = auto_context("a", &index, &default_opts(10_000));
        assert_eq!(result.format_version, FORMAT_VERSION);
        assert_eq!(FORMAT_VERSION, "2.3");
    }

    #[test]
    fn schema_documents_every_serialized_field() {
        // Drift guard (ADR-0169): a real serialized AutoContextResult must have
        // every top-level key AND every `sections` key documented in the published
        // schema. Adding a struct field without updating the schema fails here.
        let (index, _dir) = make_index(&[("src/a.rs", "pub fn a() {}")]);
        let result = auto_context("a", &index, &default_opts(10_000));
        let json = serde_json::to_value(&result).unwrap();
        let schema = crate::commands::schema::auto_context_schema();

        let props = schema["properties"].as_object().unwrap();
        for key in json.as_object().unwrap().keys() {
            assert!(
                props.contains_key(key),
                "schema is missing documented top-level field: {key}"
            );
        }
        let sect_props = schema["properties"]["sections"]["properties"]
            .as_object()
            .unwrap();
        for key in json["sections"].as_object().unwrap().keys() {
            assert!(
                sect_props.contains_key(key),
                "schema sections is missing documented field: {key}"
            );
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

    /// Build an index with per-file language for the cross-language test.
    fn make_index_with_langs(paths: &[(&str, &str, &str)]) -> (CodebaseIndex, tempfile::TempDir) {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let files: Vec<ScannedFile> = paths
            .iter()
            .map(|(rel, lang, content)| {
                let abs = dir.path().join(rel);
                std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
                std::fs::write(&abs, content).unwrap();
                ScannedFile {
                    relative_path: (*rel).into(),
                    absolute_path: abs,
                    language: Some((*lang).into()),
                    size_bytes: content.len() as u64,
                }
            })
            .collect();
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        (index, dir)
    }

    #[test]
    fn test_auto_context_includes_cross_lang() {
        let (index, _dir) = make_index_with_langs(&[
            (
                "frontend/api.ts",
                "typescript",
                r#"async function loadUsers() { return fetch("/api/users"); }"#,
            ),
            (
                "backend/users.py",
                "python",
                "from fastapi import FastAPI\n@app.get(\"/api/users\")\ndef get_users():\n    return []\n",
            ),
        ]);
        assert!(
            !index.cross_lang_edges.is_empty(),
            "fixture must detect cross-lang edges"
        );

        let opts = default_opts(10_000);
        let result = auto_context("add error handling to the API", &index, &opts);
        assert!(
            result.sections.cross_language_edges.is_some(),
            "expected cross_language_edges section to be populated"
        );
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
            cost_model: None,
            tokens: 50_000,
            focus: Some("src/api/".to_string()),
            include_tests: false,
            include_blast_radius: false,
            mode: "full".to_string(),
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
    // test_auto_context_briefing_mode_content_is_none
    // -----------------------------------------------------------------------
    #[test]
    fn test_auto_context_briefing_mode_content_is_none() {
        let (index, _dir) = make_index(&[(
            "src/auth.rs",
            "pub fn authenticate(user: &str) -> bool { true }",
        )]);
        let opts = AutoContextOpts {
            cost_model: None,
            tokens: 50_000,
            focus: None,
            include_tests: false,
            include_blast_radius: false,
            mode: "briefing".to_string(),
        };
        let result = auto_context("authenticate", &index, &opts);
        for file in &result.sections.target_files.files {
            assert!(
                file.content.is_none(),
                "briefing mode must set content to None, got Some for {}",
                file.path
            );
        }
    }

    // -----------------------------------------------------------------------
    // test_auto_context_full_mode_content_is_some
    // -----------------------------------------------------------------------
    #[test]
    fn test_auto_context_full_mode_content_is_some() {
        let (index, _dir) = make_index(&[(
            "src/auth.rs",
            "pub fn authenticate(user: &str) -> bool { true }",
        )]);
        let opts = default_opts(50_000);
        let result = auto_context("authenticate", &index, &opts);
        for file in &result.sections.target_files.files {
            assert!(
                file.content.is_some(),
                "full mode must set content to Some, got None for {}",
                file.path
            );
        }
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
            cost_model: None,
            tokens: 50_000,
            focus: None,
            include_tests: false,
            include_blast_radius: false,
            mode: "full".to_string(),
        };
        let result = auto_context("handle request", &index, &opts);

        assert_eq!(
            result.sections.test_files.count, 0,
            "include_tests=false should produce no test_files"
        );
        assert!(result.sections.test_files.files.is_empty());
    }

    // -----------------------------------------------------------------------
    // DNA section budget branches
    // -----------------------------------------------------------------------

    #[test]
    fn test_auto_context_tiny_budget_skips_dna() {
        // tokens < 2000 → DNA section is skipped entirely
        let (index, _dir) = make_index(&[("src/auth.rs", "pub fn authenticate() {}")]);
        let opts = default_opts(1_500);
        let result = auto_context("authenticate", &index, &opts);
        // dna must be empty when tokens < 2000
        assert!(
            result.dna.is_empty(),
            "DNA must be empty for tokens < 2000, got {} chars",
            result.dna.len()
        );
        // Budget invariant
        assert_eq!(
            result.budget.used + result.budget.remaining,
            result.budget.total
        );
    }

    #[test]
    fn test_auto_context_compact_dna_for_medium_budget() {
        // 2000 <= tokens < 5000 → compact DNA rendered
        let (index, _dir) = make_index(&[("src/auth.rs", "pub fn authenticate() {}")]);
        let opts = default_opts(3_000);
        let result = auto_context("authenticate", &index, &opts);
        // Compact DNA may be empty if no convention data, but the branch is exercised.
        // Budget invariant must hold.
        assert_eq!(
            result.budget.used + result.budget.remaining,
            result.budget.total
        );
    }

    // -----------------------------------------------------------------------
    // include_tests=true path
    // -----------------------------------------------------------------------

    #[test]
    fn test_auto_context_include_tests_with_test_map() {
        let (mut index, _dir) = make_index(&[
            ("src/handler.rs", "pub fn handle_request() {}"),
            (
                "tests/handler_test.rs",
                "fn test_handle() { assert!(true); }",
            ),
        ]);
        // Inject a test_map entry so the include_tests branch resolves a test file.
        use crate::intelligence::test_map::{TestConfidence, TestFileRef};
        index.test_map.insert(
            "src/handler.rs".to_string(),
            vec![TestFileRef {
                path: "tests/handler_test.rs".to_string(),
                confidence: TestConfidence::NameMatch,
            }],
        );

        let opts = AutoContextOpts {
            cost_model: None,
            tokens: 50_000,
            focus: None,
            include_tests: true,
            include_blast_radius: false,
            mode: "full".to_string(),
        };
        let result = auto_context("handle request", &index, &opts);
        // The test resolver only emits a test if the seed file ("src/handler.rs") was kept.
        // We just verify the include_tests branch executed without crashing and the budget
        // invariant still holds.
        assert_eq!(
            result.budget.used + result.budget.remaining,
            result.budget.total
        );
    }

    // -----------------------------------------------------------------------
    // include_tests: inline tests detection (no separate test file)
    // -----------------------------------------------------------------------

    #[test]
    fn test_auto_context_include_tests_inline_tests_detected() {
        // A Rust file with an inline #[cfg(test)] block has no entry in the
        // test_map, yet include_tests=true should still surface it as a test
        // file because has_inline_tests() returns true for it.
        //
        // We construct a two-file index where one file is the implementation
        // and the other carries inline tests.  We then manually verify that
        // the inline-test source appears in the test_files section.  Because
        // the auto_context pipeline must first select the file as a seed, we
        // use a task string that directly matches the file's content and path.
        let rust_with_inline_tests = concat!(
            "pub fn authenticate(user: &str) -> bool { !user.is_empty() }\n",
            "\n",
            "#[cfg(test)]\n",
            "mod tests {\n",
            "    use super::*;\n",
            "    #[test]\n",
            "    fn test_authenticate() { assert!(authenticate(\"alice\")); }\n",
            "}\n",
        );
        // Build the index.  The helper writes content to disk so that
        // CodebaseIndex::build can read it into IndexedFile::content.
        let (index, _dir) = make_index(&[("src/auth.rs", rust_with_inline_tests)]);

        // Verify the index actually stored the inline content so has_inline_tests works.
        let stored_content = index
            .files
            .iter()
            .find(|f| f.relative_path == "src/auth.rs")
            .map(|f| f.content.as_str())
            .unwrap_or("");
        assert!(
            stored_content.contains("#[cfg(test)]"),
            "IndexedFile content must contain inline test block; got: {stored_content:?}"
        );

        let opts = AutoContextOpts {
            cost_model: None,
            tokens: 50_000,
            focus: None,
            include_tests: true,
            include_blast_radius: false,
            mode: "full".to_string(),
        };
        let result = auto_context("user authentication", &index, &opts);

        // The file must appear in test_files because it carries inline tests,
        // provided it was selected as a seed (which it should be given the task).
        // We check that IF the file was kept as a seed it also appears in test_files.
        let in_target = result
            .sections
            .target_files
            .files
            .iter()
            .any(|f| f.path == "src/auth.rs");
        let in_tests = result
            .sections
            .test_files
            .files
            .iter()
            .any(|f| f.path == "src/auth.rs");

        if in_target {
            assert!(
                in_tests,
                "src/auth.rs was selected as a target and has #[cfg(test)]; \
                 it must also appear in test_files"
            );
        }
        // Budget invariant always holds.
        assert_eq!(
            result.budget.used + result.budget.remaining,
            result.budget.total
        );
    }

    // -----------------------------------------------------------------------
    // include_blast_radius=true path
    // -----------------------------------------------------------------------

    #[test]
    fn test_auto_context_include_blast_radius() {
        let (index, _dir) = make_index(&[
            ("src/auth.rs", "pub fn authenticate() {}"),
            ("src/session.rs", "pub fn make_session() {}"),
        ]);
        let opts = AutoContextOpts {
            cost_model: None,
            tokens: 50_000,
            focus: None,
            include_tests: false,
            include_blast_radius: true,
            mode: "full".to_string(),
        };
        let result = auto_context("authenticate", &index, &opts);
        // The branch was exercised — we just need budget invariance.
        assert_eq!(
            result.budget.used + result.budget.remaining,
            result.budget.total
        );
    }

    // -----------------------------------------------------------------------
    // predictions: file mention in task
    // -----------------------------------------------------------------------

    #[test]
    fn test_auto_context_predictions_for_file_mention() {
        // Mentioning a file path with a recognized extension in the task should
        // populate the predictions field.
        let (index, _dir) = make_index(&[
            ("src/auth.rs", "pub fn authenticate() {}"),
            ("src/session.rs", "pub fn make_session() {}"),
        ]);
        let opts = default_opts(50_000);
        let result = auto_context("please update src/auth.rs to fix the bug", &index, &opts);
        assert!(
            result.predictions.is_some(),
            "predictions should be Some when a file is mentioned"
        );
    }

    #[test]
    fn test_auto_context_no_predictions_without_mentions() {
        let (index, _dir) = make_index(&[("src/auth.rs", "pub fn authenticate() {}")]);
        let opts = default_opts(50_000);
        let result = auto_context("just a generic question", &index, &opts);
        assert!(
            result.predictions.is_none(),
            "predictions should be None when no file is mentioned"
        );
    }
}
