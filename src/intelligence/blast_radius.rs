use crate::index::graph::DependencyGraph;
use crate::intelligence::test_map::TestFileRef;
use crate::schema::EdgeType;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};

// --------------------------------------------------------------------------
// Public types
// --------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct BlastRadiusResult {
    pub changed_files: Vec<String>,
    pub total_affected: usize,
    pub categories: BlastRadiusCategories,
    pub risk_summary: RiskSummary,
}

#[derive(Debug, Serialize)]
pub struct BlastRadiusCategories {
    pub direct_dependents: Vec<AffectedFile>,
    pub transitive_dependents: Vec<AffectedFile>,
    pub test_files: Vec<AffectedFile>,
    pub schema_dependents: Vec<AffectedFile>,
}

#[derive(Debug, Serialize)]
pub struct AffectedFile {
    pub path: String,
    pub edge_type: String,
    pub hops: usize,
    pub risk: f64,
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RiskSummary {
    pub high: usize,
    pub medium: usize,
    pub low: usize,
}

// --------------------------------------------------------------------------
// Risk scoring
// --------------------------------------------------------------------------

/// Compute the **blast impact** score for a file affected by a structural change.
///
/// This is the per-file score used when building blast-radius results.  It
/// models how much a *change propagation* hurts a particular dependent file.
///
/// Formula: `clamp(hop_decay × edge_weight × file_pagerank × untested_boost, 0.0, 1.0)`
///
/// - `hops` = 1 for direct dependents (seeds are NOT included in results)
/// - `file_pagerank` should already be in `[0.0, 1.0]`
/// - `untested_boost` = 1.2 when the file has no test coverage (untested = riskier),
///   and 1.0 when it does have coverage (no boost). Can push the raw score above 1.0
///   before the final clamp.
///
/// # Distinction from `risk::compute_risk_ranking`
///
/// This function answers: "how badly does *this dependent file* get hurt when
/// the changed files are modified?" — a structural propagation score.
///
/// `risk::compute_risk_ranking` answers a different question: "how risky is
/// *this file as a source of future bugs?*" — using churn rate, blast-radius
/// size, and test-coverage absence as an activity/health signal.
///
/// Both are valid risk signals; they measure different dimensions and are used
/// in separate contexts.  Do not conflate them.
pub fn compute_blast_impact(
    hops: usize,
    edge_type: &EdgeType,
    file_pagerank: f64,
    has_test_coverage: bool,
) -> f64 {
    let hop_decay = 1.0 / (hops as f64 + 1.0);

    let edge_weight = match edge_type {
        EdgeType::Import | EdgeType::ForeignKey | EdgeType::OrmModel => 1.0,
        EdgeType::EmbeddedSql | EdgeType::ViewReference | EdgeType::FunctionReference => 0.8,
        EdgeType::TriggerTarget | EdgeType::IndexTarget => 0.6,
        EdgeType::MigrationSequence => 0.5,
        EdgeType::CrossLanguage(_) => 0.5,
    };

    // Files without test coverage are 1.2× riskier than tested files.
    let untested_boost = if has_test_coverage { 1.0 } else { 1.2 };

    let raw = hop_decay * edge_weight * file_pagerank * untested_boost;
    raw.clamp(0.0, 1.0)
}

// --------------------------------------------------------------------------
// Categorization helpers
// --------------------------------------------------------------------------

/// Returns true when `path` looks like a test file based on path components.
/// Checks for `test`, `spec`, `__tests__` anywhere in the path.
pub(crate) fn is_test_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    let parts: Vec<&str> = lower.split('/').collect();
    for part in &parts[..parts.len().saturating_sub(1)] {
        if matches!(*part, "tests" | "test" | "spec" | "__tests__") {
            return true;
        }
    }
    // Also catch filename-level markers like foo_test.rs, test_foo.py, etc.
    if let Some(filename) = parts.last() {
        if filename.contains("_test.")
            || filename.contains("_spec.")
            || filename.starts_with("test_")
            || filename.ends_with("test")
            || filename.ends_with("spec")
            || filename.contains(".test.")
            || filename.contains(".spec.")
        {
            return true;
        }
    }
    false
}

/// Returns `true` when the edge type is a schema-layer relationship.
fn is_schema_edge(edge_type: &EdgeType) -> bool {
    matches!(
        edge_type,
        EdgeType::ForeignKey
            | EdgeType::EmbeddedSql
            | EdgeType::OrmModel
            | EdgeType::ViewReference
            | EdgeType::TriggerTarget
            | EdgeType::IndexTarget
    )
}

// --------------------------------------------------------------------------
// BFS state tracking
// --------------------------------------------------------------------------

/// Per-file best result found so far during BFS.
struct BestEntry {
    hops: usize,
    risk: f64,
    edge_type: EdgeType,
}

// --------------------------------------------------------------------------
// Main algorithm
// --------------------------------------------------------------------------

/// Compute the blast radius for a set of changed files.
///
/// Performs a BFS on the **reverse** dependency graph (i.e. follows
/// "who depends on me?" edges), starting from each `changed_files` seed.
///
/// Parameters:
/// - `changed_files` — file paths that are being changed (seeds; NOT included in results)
/// - `graph` — the typed dependency graph (uses `reverse_edges`)
/// - `pagerank` — normalised 0–1 PageRank scores keyed by path
/// - `test_map` — source → test file refs mapping (for test categorisation)
/// - `depth` — maximum BFS hops to follow
/// - `focus` — optional path prefix; only paths matching this prefix appear in results
pub fn compute_blast_radius(
    changed_files: &[&str],
    graph: &DependencyGraph,
    pagerank: &HashMap<String, f64>,
    test_map: &HashMap<String, Vec<TestFileRef>>,
    depth: usize,
    focus: Option<&str>,
) -> BlastRadiusResult {
    let changed_set: HashSet<&str> = changed_files.iter().copied().collect();

    // Build the set of test files that are covered by the changed files
    // (used for the `test_files` category).  A file qualifies only when
    // BOTH conditions hold:
    //   1. Its path matches test patterns
    //   2. It appears in `test_map` as a test for at least one changed file
    let mut covered_test_files: HashSet<String> = HashSet::new();
    for &changed in changed_files {
        if let Some(refs) = test_map.get(changed) {
            for tr in refs {
                covered_test_files.insert(tr.path.clone());
            }
        }
    }

    // BFS on reverse edges.
    // Queue entries: (file_path, hops, incoming_edge_type)
    let mut queue: VecDeque<(String, usize, EdgeType)> = VecDeque::new();
    let mut visited: HashSet<String> = HashSet::new();

    // Seed the queue with direct dependents of every changed file (hops = 1).
    for &seed in changed_files {
        visited.insert(seed.to_string());
        for edge in graph.dependents(seed) {
            // `edge.target` here is the importer (the file that imports `seed`).
            let dep_path = &edge.target;
            if !visited.contains(dep_path.as_str()) && !changed_set.contains(dep_path.as_str()) {
                queue.push_back((dep_path.clone(), 1, edge.edge_type.clone()));
            }
        }
    }

    // `best`: for each file, track the highest-risk (hops, edge_type) combination.
    let mut best: HashMap<String, BestEntry> = HashMap::new();

    while let Some((path, hops, edge_type)) = queue.pop_front() {
        if hops > depth {
            continue;
        }

        let file_pagerank = pagerank.get(&path).copied().unwrap_or(0.0);
        let has_test = covered_test_files.contains(&path)
            || test_map
                .values()
                .any(|refs| refs.iter().any(|r| r.path == path));
        let risk = compute_blast_impact(hops, &edge_type, file_pagerank, has_test);

        // Update best entry: keep highest risk. If equal risk, prefer fewer hops.
        let update = match best.get(&path) {
            None => true,
            Some(prev) => risk > prev.risk || (risk == prev.risk && hops < prev.hops),
        };
        if update {
            best.insert(
                path.clone(),
                BestEntry {
                    hops,
                    risk,
                    edge_type: edge_type.clone(),
                },
            );
        }

        // Continue BFS only if we haven't already visited this node.
        if !visited.insert(path.clone()) {
            continue;
        }

        // Expand further hops (if within depth limit).
        if hops < depth {
            for next_edge in graph.dependents(&path) {
                let next_path = &next_edge.target;
                if !visited.contains(next_path.as_str())
                    && !changed_set.contains(next_path.as_str())
                {
                    queue.push_back((next_path.clone(), hops + 1, next_edge.edge_type.clone()));
                }
            }
        }
    }

    // ---------- Categorise results ----------

    let mut direct_dependents: Vec<AffectedFile> = Vec::new();
    let mut transitive_dependents: Vec<AffectedFile> = Vec::new();
    let mut test_files: Vec<AffectedFile> = Vec::new();
    let mut schema_dependents: Vec<AffectedFile> = Vec::new();

    let mut risk_high = 0usize;
    let mut risk_medium = 0usize;
    let mut risk_low = 0usize;

    for (path, entry) in &best {
        // Apply focus filter.
        if let Some(f) = focus {
            if !path.starts_with(f) {
                continue;
            }
        }

        // Tally risk.
        if entry.risk >= 0.7 {
            risk_high += 1;
        } else if entry.risk >= 0.3 {
            risk_medium += 1;
        } else {
            risk_low += 1;
        }

        let edge_label = edge_type_label(&entry.edge_type);

        // Priority 1: test_files — BOTH conditions required:
        //   a) path matches test patterns
        //   b) the file is in `covered_test_files` (i.e. it's a test for a changed file)
        if is_test_path(path) && covered_test_files.contains(path) {
            test_files.push(AffectedFile {
                path: path.clone(),
                edge_type: edge_label,
                hops: entry.hops,
                risk: entry.risk,
                note: Some("directly tests changed file".to_string()),
            });
        } else if entry.hops == 1 {
            // Priority 2: direct_dependents
            direct_dependents.push(AffectedFile {
                path: path.clone(),
                edge_type: edge_label,
                hops: entry.hops,
                risk: entry.risk,
                note: None,
            });
        } else if is_schema_edge(&entry.edge_type) {
            // Priority 3: schema_dependents (hops >= 2, schema edge)
            schema_dependents.push(AffectedFile {
                path: path.clone(),
                edge_type: edge_label,
                hops: entry.hops,
                risk: entry.risk,
                note: None,
            });
        } else {
            // Priority 4: transitive_dependents (hops >= 2, non-schema edge)
            transitive_dependents.push(AffectedFile {
                path: path.clone(),
                edge_type: edge_label,
                hops: entry.hops,
                risk: entry.risk,
                note: None,
            });
        }
    }

    // Sort each category by risk descending for stable, useful output.
    let sort_by_risk = |a: &AffectedFile, b: &AffectedFile| {
        b.risk
            .partial_cmp(&a.risk)
            .unwrap_or(std::cmp::Ordering::Equal)
    };
    direct_dependents.sort_by(sort_by_risk);
    transitive_dependents.sort_by(sort_by_risk);
    test_files.sort_by(sort_by_risk);
    schema_dependents.sort_by(sort_by_risk);

    let total_affected = direct_dependents.len()
        + transitive_dependents.len()
        + test_files.len()
        + schema_dependents.len();

    BlastRadiusResult {
        changed_files: changed_files.iter().map(|s| s.to_string()).collect(),
        total_affected,
        categories: BlastRadiusCategories {
            direct_dependents,
            transitive_dependents,
            test_files,
            schema_dependents,
        },
        risk_summary: RiskSummary {
            high: risk_high,
            medium: risk_medium,
            low: risk_low,
        },
    }
}

fn edge_type_label(et: &EdgeType) -> String {
    match et {
        EdgeType::Import => "import".to_string(),
        EdgeType::ForeignKey => "foreign_key".to_string(),
        EdgeType::ViewReference => "view_reference".to_string(),
        EdgeType::TriggerTarget => "trigger_target".to_string(),
        EdgeType::IndexTarget => "index_target".to_string(),
        EdgeType::FunctionReference => "function_reference".to_string(),
        EdgeType::EmbeddedSql => "embedded_sql".to_string(),
        EdgeType::OrmModel => "orm_model".to_string(),
        EdgeType::MigrationSequence => "migration_sequence".to_string(),
        EdgeType::CrossLanguage(bt) => format!("cross_language:{bt:?}"),
    }
}

// --------------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pagerank(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn make_test_map_from(entries: &[(&str, &str)]) -> HashMap<String, Vec<TestFileRef>> {
        use crate::intelligence::test_map::{TestConfidence, TestFileRef};
        let mut map: HashMap<String, Vec<TestFileRef>> = HashMap::new();
        for (src, test) in entries {
            map.entry(src.to_string()).or_default().push(TestFileRef {
                path: test.to_string(),
                confidence: TestConfidence::NameMatch,
            });
        }
        map
    }

    // -----------------------------------------------------------------------
    // compute_blast_impact tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_risk_hop_decay() {
        // hop=1 → decay = 1/2 = 0.5, hop=2 → decay = 1/3 ≈ 0.333
        let r1 = compute_blast_impact(1, &EdgeType::Import, 1.0, true);
        let r2 = compute_blast_impact(2, &EdgeType::Import, 1.0, true);
        assert!(
            r1 > r2,
            "direct dependent (hops=1) should have higher risk than transitive (hops=2)"
        );
        // hop=1 with full pagerank: 0.5 * 1.0 * 1.0 * 1.0 = 0.5
        assert!((r1 - 0.5).abs() < 1e-9, "expected 0.5, got {r1}");
    }

    #[test]
    fn test_risk_edge_weight() {
        // Import (1.0) vs MigrationSequence (0.5) at same hops and pagerank
        let r_import = compute_blast_impact(1, &EdgeType::Import, 1.0, true);
        let r_migration = compute_blast_impact(1, &EdgeType::MigrationSequence, 1.0, true);
        assert!(
            r_import > r_migration,
            "Import edge should score higher than MigrationSequence"
        );
    }

    #[test]
    fn test_risk_untested_penalty() {
        // Untested file should score higher (penalty 1.2 > 1.0)
        let r_tested = compute_blast_impact(1, &EdgeType::Import, 0.5, true);
        let r_untested = compute_blast_impact(1, &EdgeType::Import, 0.5, false);
        assert!(
            r_untested > r_tested,
            "untested file should have higher risk than tested file"
        );
    }

    #[test]
    fn test_risk_clamped_to_one() {
        // Even with max penalty, result must not exceed 1.0
        // hops=1, Import, pagerank=1.0, untested → raw = 0.5 * 1.0 * 1.0 * 1.2 = 0.6
        let risk = compute_blast_impact(1, &EdgeType::Import, 1.0, false);
        assert!(risk <= 1.0, "risk must not exceed 1.0");
        assert!(
            risk > 0.5,
            "direct dependent with high pagerank should be high risk"
        );
    }

    #[test]
    fn test_risk_clamped_at_zero() {
        // Pagerank 0.0 → risk is 0.0 regardless
        let risk = compute_blast_impact(1, &EdgeType::Import, 0.0, false);
        assert_eq!(risk, 0.0);
    }

    #[test]
    fn test_risk_all_edge_weights() {
        let pr = 1.0;
        // Import, ForeignKey, OrmModel → weight 1.0
        let r_import = compute_blast_impact(1, &EdgeType::Import, pr, true);
        let r_fk = compute_blast_impact(1, &EdgeType::ForeignKey, pr, true);
        let r_orm = compute_blast_impact(1, &EdgeType::OrmModel, pr, true);
        assert!((r_import - r_fk).abs() < 1e-9);
        assert!((r_import - r_orm).abs() < 1e-9);

        // EmbeddedSql, ViewReference, FunctionReference → weight 0.8
        let r_sql = compute_blast_impact(1, &EdgeType::EmbeddedSql, pr, true);
        let r_view = compute_blast_impact(1, &EdgeType::ViewReference, pr, true);
        let r_fn = compute_blast_impact(1, &EdgeType::FunctionReference, pr, true);
        assert!((r_sql - r_view).abs() < 1e-9);
        assert!((r_sql - r_fn).abs() < 1e-9);
        assert!(r_import > r_sql);

        // TriggerTarget, IndexTarget → weight 0.6
        let r_trig = compute_blast_impact(1, &EdgeType::TriggerTarget, pr, true);
        let r_idx = compute_blast_impact(1, &EdgeType::IndexTarget, pr, true);
        assert!((r_trig - r_idx).abs() < 1e-9);
        assert!(r_sql > r_trig);

        // MigrationSequence → weight 0.5
        let r_mig = compute_blast_impact(1, &EdgeType::MigrationSequence, pr, true);
        assert!(r_trig > r_mig);
    }

    // -----------------------------------------------------------------------
    // compute_blast_radius tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_blast_radius_empty() {
        let graph = DependencyGraph::new();
        let pagerank = HashMap::new();
        let test_map = HashMap::new();
        let result = compute_blast_radius(&[], &graph, &pagerank, &test_map, 3, None);
        assert_eq!(result.total_affected, 0);
        assert!(result.categories.direct_dependents.is_empty());
        assert!(result.categories.transitive_dependents.is_empty());
        assert!(result.categories.test_files.is_empty());
        assert!(result.categories.schema_dependents.is_empty());
    }

    #[test]
    fn test_blast_radius_direct_dependent() {
        // A imports B; changing B → A is a direct dependent (hops=1)
        let mut graph = DependencyGraph::new();
        graph.add_edge("src/a.rs", "src/b.rs", EdgeType::Import);

        let pagerank = make_pagerank(&[("src/a.rs", 0.8), ("src/b.rs", 0.5)]);
        let test_map = HashMap::new();

        let result = compute_blast_radius(&["src/b.rs"], &graph, &pagerank, &test_map, 3, None);

        assert_eq!(result.total_affected, 1);
        assert_eq!(result.categories.direct_dependents.len(), 1);
        let dep = &result.categories.direct_dependents[0];
        assert_eq!(dep.path, "src/a.rs");
        assert_eq!(dep.hops, 1);
        assert_eq!(dep.edge_type, "import");
        // risk = 1/(1+1) * 1.0 * 0.8 * 1.2 = 0.5 * 0.8 * 1.2 = 0.48
        assert!((dep.risk - 0.48).abs() < 1e-9);
    }

    #[test]
    fn test_blast_radius_transitive() {
        // Chain: A → B → C; changing C → B (hops=1), A (hops=2)
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "c.rs", EdgeType::Import);

        let pagerank = make_pagerank(&[("a.rs", 0.9), ("b.rs", 0.6), ("c.rs", 0.3)]);
        let test_map = HashMap::new();

        let result = compute_blast_radius(&["c.rs"], &graph, &pagerank, &test_map, 3, None);

        assert_eq!(result.total_affected, 2);
        let direct: HashSet<&str> = result
            .categories
            .direct_dependents
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        let transitive: HashSet<&str> = result
            .categories
            .transitive_dependents
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert!(direct.contains("b.rs"), "b.rs should be a direct dependent");
        assert!(
            transitive.contains("a.rs"),
            "a.rs should be a transitive dependent"
        );
    }

    #[test]
    fn test_blast_radius_depth_limit() {
        // Chain of 5: a→b→c→d→e; changing e with depth=2
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "c.rs", EdgeType::Import);
        graph.add_edge("c.rs", "d.rs", EdgeType::Import);
        graph.add_edge("d.rs", "e.rs", EdgeType::Import);

        let pagerank = make_pagerank(&[
            ("a.rs", 1.0),
            ("b.rs", 0.8),
            ("c.rs", 0.6),
            ("d.rs", 0.4),
            ("e.rs", 0.2),
        ]);
        let test_map = HashMap::new();

        let result = compute_blast_radius(&["e.rs"], &graph, &pagerank, &test_map, 2, None);

        // depth=2: d (hops=1), c (hops=2); a and b should not appear
        let all_paths: HashSet<&str> = result
            .categories
            .direct_dependents
            .iter()
            .chain(result.categories.transitive_dependents.iter())
            .chain(result.categories.test_files.iter())
            .chain(result.categories.schema_dependents.iter())
            .map(|f| f.path.as_str())
            .collect();

        assert!(
            all_paths.contains("d.rs"),
            "d.rs should be reachable at depth=2"
        );
        assert!(
            all_paths.contains("c.rs"),
            "c.rs should be reachable at depth=2"
        );
        assert!(!all_paths.contains("b.rs"), "b.rs is beyond depth limit");
        assert!(!all_paths.contains("a.rs"), "a.rs is beyond depth limit");
    }

    #[test]
    fn test_blast_radius_focus_filter() {
        // Three dependents; only two match the focus prefix "src/"
        let mut graph = DependencyGraph::new();
        graph.add_edge("src/a.rs", "core.rs", EdgeType::Import);
        graph.add_edge("src/b.rs", "core.rs", EdgeType::Import);
        graph.add_edge("vendor/c.rs", "core.rs", EdgeType::Import);

        let pagerank = make_pagerank(&[("src/a.rs", 0.5), ("src/b.rs", 0.5), ("vendor/c.rs", 0.5)]);
        let test_map = HashMap::new();

        let result =
            compute_blast_radius(&["core.rs"], &graph, &pagerank, &test_map, 3, Some("src/"));

        let all_paths: HashSet<&str> = result
            .categories
            .direct_dependents
            .iter()
            .map(|f| f.path.as_str())
            .collect();

        assert!(all_paths.contains("src/a.rs"));
        assert!(all_paths.contains("src/b.rs"));
        assert!(
            !all_paths.contains("vendor/c.rs"),
            "vendor/ should be excluded by focus"
        );
        assert_eq!(result.total_affected, 2);
    }

    #[test]
    fn test_blast_radius_test_files_categorized() {
        // tests/auth_test.rs depends on src/auth.rs AND is in test_map for auth.rs
        // → categorized as test_files (priority 1, not direct_dependents)
        //
        // tests/unrelated_test.rs also depends on src/auth.rs but is NOT in test_map
        // → falls through to direct_dependents (path matches test pattern but condition 2 fails)
        let mut graph = DependencyGraph::new();
        graph.add_edge("tests/auth_test.rs", "src/auth.rs", EdgeType::Import);
        graph.add_edge("tests/unrelated_test.rs", "src/auth.rs", EdgeType::Import);

        let pagerank = make_pagerank(&[
            ("tests/auth_test.rs", 0.6),
            ("tests/unrelated_test.rs", 0.6),
        ]);

        // Only auth_test.rs is in the test_map for src/auth.rs
        let test_map = make_test_map_from(&[("src/auth.rs", "tests/auth_test.rs")]);

        let result = compute_blast_radius(&["src/auth.rs"], &graph, &pagerank, &test_map, 3, None);

        let test_file_paths: HashSet<&str> = result
            .categories
            .test_files
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        let direct_paths: HashSet<&str> = result
            .categories
            .direct_dependents
            .iter()
            .map(|f| f.path.as_str())
            .collect();

        assert!(
            test_file_paths.contains("tests/auth_test.rs"),
            "auth_test.rs should be in test_files (both conditions met)"
        );
        assert!(
            direct_paths.contains("tests/unrelated_test.rs"),
            "unrelated_test.rs should be in direct_dependents (only condition 1 met)"
        );
        assert!(
            !direct_paths.contains("tests/auth_test.rs"),
            "auth_test.rs must NOT be in direct_dependents"
        );
    }

    #[test]
    fn test_blast_radius_circular_no_panic() {
        // Circular dependency: A → B → C → A; changing A should not infinite-loop
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "c.rs", EdgeType::Import);
        graph.add_edge("c.rs", "a.rs", EdgeType::Import);

        let pagerank = make_pagerank(&[("a.rs", 0.5), ("b.rs", 0.5), ("c.rs", 0.5)]);
        let test_map = HashMap::new();

        // Must not panic or infinite-loop
        let result = compute_blast_radius(&["a.rs"], &graph, &pagerank, &test_map, 10, None);

        // b and c are affected; a is the seed (excluded from results)
        let all_paths: HashSet<&str> = result
            .categories
            .direct_dependents
            .iter()
            .chain(result.categories.transitive_dependents.iter())
            .map(|f| f.path.as_str())
            .collect();

        assert!(
            !all_paths.contains("a.rs"),
            "seed must not appear in results"
        );
        assert_eq!(result.total_affected, 2);
    }

    #[test]
    fn test_blast_radius_multiple_changed_files() {
        // Two changed files; dependents are the union (deduplicated)
        let mut graph = DependencyGraph::new();
        graph.add_edge("x.rs", "a.rs", EdgeType::Import);
        graph.add_edge("x.rs", "b.rs", EdgeType::Import);
        graph.add_edge("y.rs", "b.rs", EdgeType::Import);

        let pagerank = make_pagerank(&[("x.rs", 0.8), ("y.rs", 0.6)]);
        let test_map = HashMap::new();

        let result = compute_blast_radius(&["a.rs", "b.rs"], &graph, &pagerank, &test_map, 3, None);

        let all_paths: HashSet<&str> = result
            .categories
            .direct_dependents
            .iter()
            .map(|f| f.path.as_str())
            .collect();

        // x.rs imports a.rs, x.rs also imports b.rs, y.rs imports b.rs
        assert!(all_paths.contains("x.rs"), "x.rs should be affected");
        assert!(all_paths.contains("y.rs"), "y.rs should be affected");
        // x.rs should appear only once (deduplication)
        let x_count = result
            .categories
            .direct_dependents
            .iter()
            .filter(|f| f.path == "x.rs")
            .count();
        assert_eq!(x_count, 1, "x.rs must appear exactly once");
    }

    #[test]
    fn test_blast_radius_schema_dependents() {
        // A file connected via ForeignKey at hops=2 → schema_dependents
        let mut graph = DependencyGraph::new();
        // direct dep via Import at hops=1
        graph.add_edge("api.rs", "models.rs", EdgeType::Import);
        // schema dep via ForeignKey at hops=2
        graph.add_edge("report.rs", "api.rs", EdgeType::ForeignKey);

        let pagerank = make_pagerank(&[("api.rs", 0.7), ("report.rs", 0.5)]);
        let test_map = HashMap::new();

        let result = compute_blast_radius(&["models.rs"], &graph, &pagerank, &test_map, 3, None);

        let schema_paths: HashSet<&str> = result
            .categories
            .schema_dependents
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert!(
            schema_paths.contains("report.rs"),
            "report.rs should be in schema_dependents"
        );
    }

    #[test]
    fn test_blast_radius_risk_summary_counts() {
        // Verify that risk_summary counts sum to total_affected
        let mut graph = DependencyGraph::new();
        // high risk: pagerank=1.0, hops=1 → 0.5 * 1.0 * 1.0 * 1.2 = 0.6 (medium)
        // Use enough pagerank to push into different buckets:
        // hops=1, pagerank=1.0, untested → 0.6 (medium)
        // hops=1, pagerank=1.0, tested → 0.5 (medium)
        graph.add_edge("dep1.rs", "src.rs", EdgeType::Import);
        graph.add_edge("dep2.rs", "src.rs", EdgeType::Import);

        let pagerank = make_pagerank(&[("dep1.rs", 1.0), ("dep2.rs", 0.1)]);
        let test_map = HashMap::new();

        let result = compute_blast_radius(&["src.rs"], &graph, &pagerank, &test_map, 3, None);

        let summary = &result.risk_summary;
        let counted = summary.high + summary.medium + summary.low;
        assert_eq!(
            counted, result.total_affected,
            "risk_summary counts must sum to total_affected"
        );
    }

    #[test]
    fn test_blast_radius_multiple_edges_highest_risk_wins() {
        // Same target reachable via two paths — best risk is kept.
        // A → C (Import, hops=1) and B → C (Import, hops=1)
        // But we only change C; both A and B are direct dependents.
        // Now add a second path: D → A (Import, hops=1); D is hops=2 from C.
        // Also D → C directly (Import, hops=1).
        // D should appear at hops=1 (best hop).
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "c.rs", EdgeType::Import);
        graph.add_edge("d.rs", "a.rs", EdgeType::Import);
        graph.add_edge("d.rs", "c.rs", EdgeType::ForeignKey); // also direct, schema edge

        let pagerank = make_pagerank(&[("a.rs", 0.5), ("d.rs", 0.8)]);
        let test_map = HashMap::new();

        let result = compute_blast_radius(&["c.rs"], &graph, &pagerank, &test_map, 3, None);

        // d.rs is reachable via hops=1 (Import/ForeignKey from c.rs) AND hops=2 (via a.rs).
        // Best hop for d.rs is 1.  The highest-risk edge at hops=1 is ForeignKey (weight 1.0)
        // OR Import (weight 1.0) — same weight, so either is fine.
        let d_entry = result
            .categories
            .direct_dependents
            .iter()
            .chain(result.categories.schema_dependents.iter())
            .find(|f| f.path == "d.rs")
            .expect("d.rs must appear in results");
        assert_eq!(d_entry.hops, 1, "d.rs should appear at hops=1 (best path)");
    }

    #[test]
    fn test_blast_radius_changed_files_not_in_results() {
        // The seeds themselves must never appear in any category
        let mut graph = DependencyGraph::new();
        // self-referencing import (unusual but should not cause seed to appear)
        graph.add_edge("a.rs", "a.rs", EdgeType::Import);

        let pagerank = make_pagerank(&[("a.rs", 1.0)]);
        let test_map = HashMap::new();

        let result = compute_blast_radius(&["a.rs"], &graph, &pagerank, &test_map, 3, None);

        let all_paths: HashSet<&str> = result
            .categories
            .direct_dependents
            .iter()
            .chain(result.categories.transitive_dependents.iter())
            .chain(result.categories.test_files.iter())
            .chain(result.categories.schema_dependents.iter())
            .map(|f| f.path.as_str())
            .collect();

        assert!(
            !all_paths.contains("a.rs"),
            "changed files must not appear in blast radius results"
        );
    }
}
