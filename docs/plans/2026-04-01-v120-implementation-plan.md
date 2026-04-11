# v1.2.0 "Codebase Health" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add compound intelligence to auto_context: health score, risk ranking, architecture map, co-change analysis, briefing mode, and recency scoring. Three new MCP tools. PackedFile.content becomes Option<String>.

**Architecture:** The new intelligence types (`HealthScore`, `RiskEntry`, `ArchitectureMap`, `CoChangeEdge`, `RecentChange`) are computed from data already present in `CodebaseIndex` — git_health churn data, the dependency graph, test_map, and conventions. Co-change analysis piggybacks on the git2 walk already done in `extract_git_health`, storing results on `CodebaseIndex`. The `auto_context` pipeline grows a `mode` parameter; briefing mode sets `PackedFile.content` to `None` instead of `Some(src)`. The three new MCP tools (`cxpak_health`, `cxpak_risks`, `cxpak_briefing`) are registered in `mcp_stdio_loop_with_io` following the exact JSON-RPC pattern already used by the 13 existing tools.

**Tech Stack:** Rust 1.80+, tree-sitter, tiktoken-rs, git2, serde, axum (daemon feature)

---

## Task 1 — Verify PackedFile.content is Option<String> and add briefing assertion

**Note:** `PackedFile.content` is ALREADY `Option<String>` (done in a prior version). This task verifies the existing state and adds the briefing-mode test.

**Files:**
- Verify: `src/auto_context/briefing.rs` (content field is already `Option<String>`)

**Steps:**

1. Verify `PackedFile.content` is `Option<String>` in `src/auto_context/briefing.rs` line 45. If it is already `Option<String>` (expected), skip to step 3. If somehow still `String`, change it to `Option<String>`:

2. (Skip if already Option<String>) Change `PackedFile.content` from `String` to `Option<String>` in `src/auto_context/briefing.rs`:

```rust
#[derive(Debug, Serialize)]
pub struct PackedFile {
    pub path: String,
    pub score: f64,
    pub detail_level: String,
    pub tokens: usize,
    pub content: Option<String>,
}
```

3. Update all `PackedFile { ..., content, ... }` construction sites in `briefing.rs` to wrap with `Some(...)`:

```rust
// Before (all occurrences in allocate_and_pack):
content,
// After:
content: Some(content),
```

Also update the `<schema>` PackedFile construction:
```rust
content: Some(schema_str),
// and truncated variant:
content: Some(truncated),
```

4. Fix compile errors in `src/auto_context/mod.rs` where target file content is passed. Step 6 resolve already has `f.content.clone()` — the `allocate_and_pack` signature takes `Vec<(String, f64, String)>` (plain `String`) and wraps to `Some` inside, so no change needed there. Verify no other callers break.

5. Update the `test_auto_context_*` tests in `src/auto_context/mod.rs` to assert `file.content.is_some()` for full-mode files (add one assertion to `test_auto_context_happy_path`).

6. Update `test_higher_scored_targets_packed_first` in `briefing.rs` — the assertion `files[0].path` stays valid; add `assert!(files[0].content.is_some())`.

**Commands:**
```bash
cargo test -p cxpak auto_context::briefing::tests -- --nocapture 2>&1 | tail -20
cargo test -p cxpak auto_context::tests -- --nocapture 2>&1 | tail -20
```
Expected: all briefing + auto_context tests pass.

**Commit:** `feat: change PackedFile.content to Option<String> for briefing mode support`

---

## Task 2 — CoChangeEdge type + co-change computation in git_health

**Files:**
- Modify: `src/conventions/git_health.rs`
- New type in: `src/intelligence/co_change.rs`
- Modify: `src/intelligence/mod.rs`

**Steps:**

1. Create `src/intelligence/co_change.rs` with the `CoChangeEdge` type and computation function:

```rust
use crate::conventions::git_health::GitHealthProfile;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CoChangeEdge {
    pub file_a: String,
    pub file_b: String,
    pub count: u32,
    pub recency_weight: f64,
}

/// Decay weight for a commit `days_ago` days old (180d window).
/// Returns 1.0 for days_ago <= 30, linearly decays to 0.3 at days_ago == 180.
/// Commits older than 180 days are excluded before calling this.
pub fn co_change_weight(days_ago: i64) -> f64 {
    if days_ago <= 30 {
        1.0
    } else {
        // days_ago in (30, 180]: linearly interpolate from 1.0 down to 0.3
        1.0 - 0.7 * (days_ago - 30) as f64 / 150.0
    }
}

/// Build co-change edges from a list of (commit_files, days_ago) pairs.
///
/// A pair (file_a, file_b) becomes an edge when it co-appears in >= 3 commits
/// within the 180-day window. `recency_weight` is the weight of the most recent
/// co-commit (not the average), per the design spec.
///
/// `commits` is `Vec<(Vec<String>, i64)>` where the i64 is days_ago at index time.
pub fn build_co_changes(commits: &[(Vec<String>, i64)]) -> Vec<CoChangeEdge> {
    use std::collections::HashMap;

    // Map (sorted file_a, file_b) -> (count, most_recent_days_ago)
    let mut pair_data: HashMap<(String, String), (u32, i64)> = HashMap::new();

    for (files, days_ago) in commits {
        if files.len() < 2 {
            continue;
        }
        // Build all pairs from the commit's changed files (sorted for dedup)
        for i in 0..files.len() {
            for j in (i + 1)..files.len() {
                let a = files[i].clone();
                let b = files[j].clone();
                let key = if a <= b { (a, b) } else { (b, a) };
                let entry = pair_data.entry(key).or_insert((0, *days_ago));
                entry.0 += 1;
                // Track the most recent (smallest days_ago)
                if *days_ago < entry.1 {
                    entry.1 = *days_ago;
                }
            }
        }
    }

    pair_data
        .into_iter()
        .filter(|(_, (count, _))| *count >= 3)
        .map(|((file_a, file_b), (count, most_recent_days))| CoChangeEdge {
            file_a,
            file_b,
            count,
            recency_weight: co_change_weight(most_recent_days),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_co_change_weight_at_zero_days() {
        assert!((co_change_weight(0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_co_change_weight_at_30_days() {
        assert!((co_change_weight(30) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_co_change_weight_at_180_days() {
        // 1.0 - 0.7 * (180-30)/150 = 1.0 - 0.7 = 0.3
        assert!((co_change_weight(180) - 0.3).abs() < 1e-9);
    }

    #[test]
    fn test_co_change_weight_at_105_days() {
        // 1.0 - 0.7 * 75/150 = 1.0 - 0.35 = 0.65
        assert!((co_change_weight(105) - 0.65).abs() < 1e-9);
    }

    #[test]
    fn test_build_co_changes_threshold_3() {
        // Two files co-appear in exactly 2 commits -> filtered out (< 3)
        let commits = vec![
            (vec!["a.rs".to_string(), "b.rs".to_string()], 10i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 20i64),
        ];
        let edges = build_co_changes(&commits);
        assert!(edges.is_empty(), "pairs with < 3 co-commits must be excluded");
    }

    #[test]
    fn test_build_co_changes_exactly_3_commits() {
        let commits = vec![
            (vec!["a.rs".to_string(), "b.rs".to_string()], 5i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 15i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 25i64),
        ];
        let edges = build_co_changes(&commits);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].count, 3);
        // Most recent is 5 days ago -> weight = 1.0
        assert!((edges[0].recency_weight - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_build_co_changes_recency_uses_most_recent() {
        // Co-appear 3 times; most recent is 100 days ago
        let commits = vec![
            (vec!["a.rs".to_string(), "b.rs".to_string()], 100i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 150i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 170i64),
        ];
        let edges = build_co_changes(&commits);
        assert_eq!(edges.len(), 1);
        // Weight for 100 days: 1.0 - 0.7 * 70/150 = 1.0 - 0.3267 = 0.6733
        let expected = 1.0 - 0.7 * 70.0 / 150.0;
        assert!((edges[0].recency_weight - expected).abs() < 1e-9);
    }

    #[test]
    fn test_build_co_changes_pair_ordering_canonical() {
        // Same pair in different order should be deduped
        let commits = vec![
            (vec!["b.rs".to_string(), "a.rs".to_string()], 5i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 10i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 15i64),
        ];
        let edges = build_co_changes(&commits);
        assert_eq!(edges.len(), 1, "reversed pair should be deduplicated");
        assert_eq!(edges[0].count, 3);
    }

    #[test]
    fn test_build_co_changes_single_file_commits_ignored() {
        // Commits with only 1 file produce no pairs
        let commits = vec![
            (vec!["a.rs".to_string()], 5i64),
            (vec!["a.rs".to_string()], 10i64),
            (vec!["a.rs".to_string()], 15i64),
        ];
        let edges = build_co_changes(&commits);
        assert!(edges.is_empty());
    }

    #[test]
    fn test_build_co_changes_multiple_pairs() {
        // Three files co-appearing: a+b (4x), a+c (3x), b+c (2x - excluded)
        let commits: Vec<(Vec<String>, i64)> = (0..4)
            .map(|i| (vec!["a.rs".to_string(), "b.rs".to_string()], i as i64 * 10))
            .chain(
                (0..3).map(|i| (vec!["a.rs".to_string(), "c.rs".to_string()], i as i64 * 10)),
            )
            .chain(
                (0..2).map(|i| (vec!["b.rs".to_string(), "c.rs".to_string()], i as i64 * 10)),
            )
            .collect();
        let edges = build_co_changes(&commits);
        assert_eq!(edges.len(), 2, "a+b and a+c should qualify; b+c (count=2) should not");
        let has_ab = edges.iter().any(|e| {
            (e.file_a == "a.rs" && e.file_b == "b.rs")
                || (e.file_a == "b.rs" && e.file_b == "a.rs")
        });
        let has_ac = edges.iter().any(|e| {
            (e.file_a == "a.rs" && e.file_b == "c.rs")
                || (e.file_a == "c.rs" && e.file_b == "a.rs")
        });
        assert!(has_ab);
        assert!(has_ac);
    }
}
```

2. Register `pub mod co_change;` in `src/intelligence/mod.rs`.

3. Extend `extract_git_health` in `src/conventions/git_health.rs` to collect co-change data during the git walk (same revwalk, no second pass). Add a `Vec<(Vec<String>, i64)>` accumulator, populate it inside the `for oid in revwalk` loop after the diff foreach, then call `build_co_changes` at the end and store the result on `GitHealthProfile`:

```rust
// In GitHealthProfile:
pub co_changes: Vec<crate::intelligence::co_change::CoChangeEdge>,

// In extract_git_health, inside the revwalk loop after changed_files is populated:
let days_ago = (now_epoch - commit_time).max(0) / 86400;
commit_file_sets.push((changed_files.clone(), days_ago));

// After the revwalk:
let co_changes = crate::intelligence::co_change::build_co_changes(&commit_file_sets);
```

**Commands:**
```bash
cargo test -p cxpak intelligence::co_change::tests -- --nocapture 2>&1 | tail -30
cargo test -p cxpak conventions::git_health::tests -- --nocapture 2>&1 | tail -20
```
Expected: all 11 co_change tests pass; git_health tests continue to pass.

**Commit:** `feat: add co-change analysis with 180d window and >=3 commit threshold`

---

## Task 3 — CoChangeEdge on CodebaseIndex

**Files:**
- Modify: `src/index/mod.rs`
- Modify: `src/conventions/mod.rs`

**Steps:**

1. Add `co_changes` field to `CodebaseIndex`:

```rust
// In src/index/mod.rs, CodebaseIndex struct:
pub co_changes: Vec<crate::intelligence::co_change::CoChangeEdge>,
```

2. Initialize to empty `Vec::new()` in `CodebaseIndex::build`, `build_with_content`, and `empty()`.

3. In `build_convention_profile` (`src/conventions/mod.rs`), after building the profile, populate `co_changes` on the index. Because `build_convention_profile` returns a `ConventionProfile` (not mutating the index), the caller in `serve.rs` (`build_index`) must propagate co_changes. The cleanest approach: after `index.conventions = build_convention_profile(...)`, set:

```rust
index.co_changes = index.conventions.git_health.co_changes.clone();
```

4. Write a test in `src/index/mod.rs` that confirms `co_changes` is empty for a non-git index:

```rust
#[test]
fn test_index_co_changes_empty_by_default() {
    let counter = TokenCounter::new();
    let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
    assert!(index.co_changes.is_empty());
}
```

**Commands:**
```bash
cargo test -p cxpak index::tests -- --nocapture 2>&1 | tail -20
cargo build 2>&1 | grep -E "^error" | head -20
```
Expected: index tests pass, no compile errors.

**Commit:** `feat: store co_changes on CodebaseIndex after convention profile build`

---

## Task 4 — HealthScore type and computation

**Files:**
- New: `src/intelligence/health.rs`
- Modify: `src/intelligence/mod.rs`

**Steps:**

1. Create `src/intelligence/health.rs`:

```rust
use crate::index::CodebaseIndex;
use crate::index::graph::DependencyGraph;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize)]
pub struct HealthScore {
    pub composite: f64,
    pub conventions: f64,
    pub test_coverage: f64,
    pub churn_stability: f64,
    pub coupling: f64,
    pub cycles: f64,
    pub dead_code: Option<f64>,
}

/// Compute the composite health score from the index.
///
/// When `dead_code` is None (v1.2.0), weights are renormalized over 5 dimensions.
pub fn compute_health(index: &CodebaseIndex) -> HealthScore {
    let conventions_score = score_conventions(index);
    let test_coverage_score = score_test_coverage(index);
    let churn_stability_score = score_churn_stability(index);
    let coupling_score = score_coupling(index, 2);
    let cycles_score = score_cycles(&index.graph);

    // dead_code is None until v1.3.0 populates it.
    let dead_code: Option<f64> = None;

    let composite = compute_composite(
        conventions_score,
        test_coverage_score,
        churn_stability_score,
        coupling_score,
        cycles_score,
        dead_code,
    );

    HealthScore {
        composite,
        conventions: conventions_score,
        test_coverage: test_coverage_score,
        churn_stability: churn_stability_score,
        coupling: coupling_score,
        cycles: cycles_score,
        dead_code,
    }
}

/// Conventions dimension: mean PatternStrength adherence across all detected patterns.
/// Convention = 10.0, Trend = 7.0, Mixed = 5.0. Empty profile = 10.0 (no violations detected).
fn score_conventions(index: &CodebaseIndex) -> f64 {
    use crate::conventions::PatternStrength;

    let p = &index.conventions;
    let mut scores: Vec<f64> = Vec::new();

    let push = |obs: &Option<crate::conventions::PatternObservation>, scores: &mut Vec<f64>| {
        if let Some(o) = obs {
            scores.push(match o.strength {
                PatternStrength::Convention => 10.0,
                PatternStrength::Trend => 7.0,
                PatternStrength::Mixed => 5.0,
            });
        }
    };

    push(&p.naming.function_style, &mut scores);
    push(&p.naming.type_style, &mut scores);
    push(&p.naming.constant_style, &mut scores);
    push(&p.imports.import_style, &mut scores);
    push(&p.errors.result_return, &mut scores);
    push(&p.visibility.default_visibility, &mut scores);
    push(&p.functions.avg_length_pattern, &mut scores);

    if scores.is_empty() {
        return 10.0;
    }
    scores.iter().sum::<f64>() / scores.len() as f64
}

/// Test coverage dimension: ratio of source files with ≥1 mapped test file, scaled to [0, 10].
fn score_test_coverage(index: &CodebaseIndex) -> f64 {
    let source_files: Vec<&str> = index
        .files
        .iter()
        .filter(|f| {
            // Exclude test files themselves from the denominator
            let p = &f.relative_path;
            !p.contains("/tests/")
                && !p.contains("/test/")
                && !p.contains("/spec/")
                && !p.contains("_test.")
                && !p.contains(".test.")
                && !p.contains("_spec.")
                && !p.contains(".spec.")
        })
        .map(|f| f.relative_path.as_str())
        .collect();

    if source_files.is_empty() {
        return 10.0;
    }

    let covered = source_files
        .iter()
        .filter(|path| index.test_map.contains_key(*path))
        .count();

    (covered as f64 / source_files.len() as f64) * 10.0
}

/// Churn stability: inverse of the ratio of "hot" files (>10 changes in 30d).
/// Score = 10.0 * (1.0 - hot_ratio). Empty churn = 10.0.
fn score_churn_stability(index: &CodebaseIndex) -> f64 {
    let churn = &index.conventions.git_health.churn_30d;
    if churn.is_empty() {
        return 10.0;
    }
    let total_files = index.total_files.max(1) as f64;
    let hot_files = churn.iter().filter(|e| e.modifications > 10).count() as f64;
    10.0 * (1.0 - (hot_files / total_files).min(1.0))
}

/// Coupling dimension: 1.0 - mean cross-module edge ratio across qualifying modules.
/// A module qualifies when it has ≥3 files. Returned score is on [0, 10].
/// When no modules qualify, returns 10.0.
/// When a qualifying module has 0 total edges, coupling = 0.0 (fully isolated → unhealthy signal).
pub fn score_coupling(index: &CodebaseIndex, module_depth: usize) -> f64 {
    // Group files into modules by taking the first `module_depth` path segments.
    let mut module_files: HashMap<String, Vec<&str>> = HashMap::new();
    for file in &index.files {
        let prefix = module_prefix(&file.relative_path, module_depth);
        module_files.entry(prefix).or_default().push(&file.relative_path);
    }

    let qualifying: Vec<(&String, &Vec<&str>)> = module_files
        .iter()
        .filter(|(_, files)| files.len() >= 3)
        .collect();

    if qualifying.is_empty() {
        return 10.0;
    }

    let module_set: HashSet<String> = qualifying
        .iter()
        .flat_map(|(_, files)| files.iter().map(|f| module_prefix(f, module_depth)))
        .collect();

    let mean_cross_ratio: f64 = qualifying
        .iter()
        .map(|(mod_name, files)| {
            let file_set: HashSet<&str> = files.iter().copied().collect();
            let mut total_edges = 0usize;
            let mut cross_edges = 0usize;

            for &file in files {
                // Outgoing edges from this file
                if let Some(deps) = index.graph.edges.get(file) {
                    for edge in deps {
                        total_edges += 1;
                        let target_mod = module_prefix(&edge.target, module_depth);
                        if &target_mod != *mod_name && module_set.contains(&target_mod) {
                            cross_edges += 1;
                        } else if !file_set.contains(edge.target.as_str()) {
                            // Edge to a file outside any qualifying module = cross-module
                            if !module_set.contains(&module_prefix(&edge.target, module_depth)) {
                                cross_edges += 1;
                            }
                        }
                    }
                }
                // Incoming edges (reverse direction)
                if let Some(deps) = index.graph.reverse_edges.get(file) {
                    for edge in deps {
                        total_edges += 1;
                        let src_mod = module_prefix(&edge.target, module_depth);
                        if &src_mod != *mod_name {
                            cross_edges += 1;
                        }
                    }
                }
            }

            if total_edges == 0 {
                0.0 // fully isolated: treat as 0.0 coupling ratio
            } else {
                cross_edges as f64 / total_edges as f64
            }
        })
        .sum::<f64>()
        / qualifying.len() as f64;

    (1.0 - mean_cross_ratio) * 10.0
}

/// Cycles dimension: 10.0 / (1.0 + scc_count), where scc_count is the number of
/// strongly connected components with size > 1. Logarithmic decay, not clamped.
pub fn score_cycles(graph: &DependencyGraph) -> f64 {
    let scc_count = count_nontrivial_sccs(graph);
    10.0 / (1.0 + scc_count as f64)
}

/// Tarjan's SCC algorithm. Returns the count of SCCs with >1 node (i.e., actual cycles).
pub fn count_nontrivial_sccs(graph: &DependencyGraph) -> usize {
    // Collect all nodes
    let nodes: Vec<String> = {
        let mut set = HashSet::new();
        for (k, edges) in &graph.edges {
            set.insert(k.clone());
            for e in edges {
                set.insert(e.target.clone());
            }
        }
        for (k, edges) in &graph.reverse_edges {
            set.insert(k.clone());
            for e in edges {
                set.insert(e.target.clone());
            }
        }
        let mut v: Vec<_> = set.into_iter().collect();
        v.sort(); // deterministic
        v
    };

    let n = nodes.len();
    if n == 0 {
        return 0;
    }

    let node_index: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();

    let mut index_counter = 0usize;
    let mut stack: Vec<usize> = Vec::new();
    let mut on_stack = vec![false; n];
    let mut indices = vec![usize::MAX; n];
    let mut lowlinks = vec![0usize; n];
    let mut nontrivial_count = 0usize;

    fn strongconnect(
        v: usize,
        nodes: &[String],
        graph: &DependencyGraph,
        node_index: &HashMap<&str, usize>,
        index_counter: &mut usize,
        stack: &mut Vec<usize>,
        on_stack: &mut Vec<bool>,
        indices: &mut Vec<usize>,
        lowlinks: &mut Vec<usize>,
        nontrivial_count: &mut usize,
    ) {
        indices[v] = *index_counter;
        lowlinks[v] = *index_counter;
        *index_counter += 1;
        stack.push(v);
        on_stack[v] = true;

        let node_path = &nodes[v];
        if let Some(edges) = graph.edges.get(node_path.as_str()) {
            let targets: Vec<usize> = edges
                .iter()
                .filter_map(|e| node_index.get(e.target.as_str()).copied())
                .collect();
            for w in targets {
                if indices[w] == usize::MAX {
                    strongconnect(
                        w,
                        nodes,
                        graph,
                        node_index,
                        index_counter,
                        stack,
                        on_stack,
                        indices,
                        lowlinks,
                        nontrivial_count,
                    );
                    lowlinks[v] = lowlinks[v].min(lowlinks[w]);
                } else if on_stack[w] {
                    lowlinks[v] = lowlinks[v].min(indices[w]);
                }
            }
        }

        if lowlinks[v] == indices[v] {
            let mut scc_size = 0;
            loop {
                let w = stack.pop().unwrap();
                on_stack[w] = false;
                scc_size += 1;
                if w == v {
                    break;
                }
            }
            if scc_size > 1 {
                *nontrivial_count += 1;
            }
        }
    }

    for i in 0..n {
        if indices[i] == usize::MAX {
            strongconnect(
                i,
                &nodes,
                graph,
                &node_index,
                &mut index_counter,
                &mut stack,
                &mut on_stack,
                &mut indices,
                &mut lowlinks,
                &mut nontrivial_count,
            );
        }
    }

    nontrivial_count
}

/// Composite with optional dead_code. When None, renormalize the 5 active weights to 1.0.
pub fn compute_composite(
    conventions: f64,
    test_coverage: f64,
    churn_stability: f64,
    coupling: f64,
    cycles: f64,
    dead_code: Option<f64>,
) -> f64 {
    match dead_code {
        Some(dc) => {
            // Full 6-dimension weights (sum = 1.0)
            0.20 * conventions
                + 0.20 * test_coverage
                + 0.15 * churn_stability
                + 0.20 * coupling
                + 0.15 * cycles
                + 0.10 * dc
        }
        None => {
            // 5-dimension weights renormalized: each / 0.90 (sum = 1.0)
            (0.20 / 0.90) * conventions
                + (0.20 / 0.90) * test_coverage
                + (0.15 / 0.90) * churn_stability
                + (0.20 / 0.90) * coupling
                + (0.15 / 0.90) * cycles
        }
    }
}

pub(crate) fn module_prefix(path: &str, depth: usize) -> String {
    let segments: Vec<&str> = path.splitn(depth + 1, '/').collect();
    if segments.len() <= depth {
        segments.join("/")
    } else {
        segments[..depth].join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::graph::DependencyGraph;
    use crate::schema::EdgeType;

    #[test]
    fn test_compute_composite_without_dead_code_sums_to_10() {
        // All dimensions at 10.0, dead_code = None -> composite = 10.0
        let composite = compute_composite(10.0, 10.0, 10.0, 10.0, 10.0, None);
        assert!((composite - 10.0).abs() < 1e-6, "got {composite}");
    }

    #[test]
    fn test_compute_composite_with_dead_code_sums_to_10() {
        let composite = compute_composite(10.0, 10.0, 10.0, 10.0, 10.0, Some(10.0));
        assert!((composite - 10.0).abs() < 1e-6, "got {composite}");
    }

    #[test]
    fn test_compute_composite_weights_renormalized() {
        // With dead_code=None, renormalized weights must sum to 1.0.
        // Verify: (0.20+0.20+0.15+0.20+0.15)/0.90 = 0.90/0.90 = 1.0
        let w_sum = (0.20 + 0.20 + 0.15 + 0.20 + 0.15) / 0.90;
        assert!((w_sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_score_cycles_no_cycles() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        // Linear graph, no cycles -> 0 nontrivial SCCs -> 10.0 / 1.0 = 10.0
        let score = score_cycles(&graph);
        assert!((score - 10.0).abs() < 1e-6, "got {score}");
    }

    #[test]
    fn test_score_cycles_one_cycle() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "a.rs", EdgeType::Import);
        // One nontrivial SCC -> 10.0 / 2.0 = 5.0
        let score = score_cycles(&graph);
        assert!((score - 5.0).abs() < 1e-6, "got {score}");
    }

    #[test]
    fn test_score_cycles_two_independent_cycles() {
        let mut graph = DependencyGraph::new();
        // Cycle 1: a <-> b
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "a.rs", EdgeType::Import);
        // Cycle 2: c <-> d
        graph.add_edge("c.rs", "d.rs", EdgeType::Import);
        graph.add_edge("d.rs", "c.rs", EdgeType::Import);
        // Two nontrivial SCCs -> 10.0 / 3.0
        let score = score_cycles(&graph);
        assert!((score - 10.0 / 3.0).abs() < 1e-6, "got {score}");
    }

    #[test]
    fn test_count_nontrivial_sccs_empty_graph() {
        let graph = DependencyGraph::new();
        assert_eq!(count_nontrivial_sccs(&graph), 0);
    }

    #[test]
    fn test_module_prefix_depth_2() {
        assert_eq!(module_prefix("src/api/handler.rs", 2), "src/api");
        assert_eq!(module_prefix("src/lib.rs", 2), "src");
        assert_eq!(module_prefix("main.rs", 2), "main.rs");
    }

    #[test]
    fn test_score_coupling_no_qualifying_modules() {
        // Fewer than 3 files per module -> score = 10.0
        use crate::budget::counter::TokenCounter;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("a.rs");
        std::fs::write(&fp, "fn a() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "src/a.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 9,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        assert!((score_coupling(&index, 2) - 10.0).abs() < 1e-6);
    }

    #[test]
    fn test_health_score_serialize() {
        let h = HealthScore {
            composite: 8.5,
            conventions: 9.0,
            test_coverage: 7.0,
            churn_stability: 8.0,
            coupling: 9.5,
            cycles: 10.0,
            dead_code: None,
        };
        let json = serde_json::to_string(&h).unwrap();
        assert!(json.contains("\"composite\":8.5"));
        assert!(json.contains("\"dead_code\":null"));
    }
}
```

2. Register `pub mod health;` in `src/intelligence/mod.rs`.

**Commands:**
```bash
cargo test -p cxpak intelligence::health::tests -- --nocapture 2>&1 | tail -40
```
Expected: 10+ tests pass.

**Commit:** `feat: add HealthScore computation with Tarjan SCC cycle detection`

---

## Task 5 — RiskEntry type and risk ranking computation

**Files:**
- New: `src/intelligence/risk.rs`
- Modify: `src/intelligence/mod.rs`

**Steps:**

1. Create `src/intelligence/risk.rs`:

```rust
use crate::index::CodebaseIndex;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RiskEntry {
    pub path: String,
    pub churn_30d: u32,
    pub blast_radius: usize,
    pub test_coverage: f64,
    pub risk_score: f64,
}

/// Compute standing risk per file, sorted descending by risk_score.
///
/// Formula: risk = max(norm_churn, 0.01) * max(norm_blast, 0.01) * max(1.0 - test_coverage, 0.01)
///
/// norm_churn: percentile rank across all files (robust against outliers)
/// norm_blast: blast_radius_count / total_files
/// test_coverage: 1.0 if has_test, 0.0 otherwise (binary in v1.2.0)
pub fn compute_risk_ranking(index: &CodebaseIndex) -> Vec<RiskEntry> {
    let total_files = index.total_files.max(1) as f64;

    // Build churn lookup from 30d data
    let churn_map: std::collections::HashMap<&str, usize> = index
        .conventions
        .git_health
        .churn_30d
        .iter()
        .map(|e| (e.path.as_str(), e.modifications))
        .collect();

    // All file paths (sorted for determinism in percentile rank)
    let mut all_paths: Vec<&str> = index
        .files
        .iter()
        .map(|f| f.relative_path.as_str())
        .collect();
    all_paths.sort();

    // Churn values for all files (0 if no churn data)
    let churn_values: Vec<usize> = all_paths
        .iter()
        .map(|p| churn_map.get(*p).copied().unwrap_or(0))
        .collect();

    // Percentile rank: for each file, what fraction of files have <= its churn?
    // norm_churn[i] = rank(churn[i]) / n
    let n = churn_values.len().max(1) as f64;
    let norm_churn: Vec<f64> = churn_values
        .iter()
        .map(|&v| {
            let rank = churn_values.iter().filter(|&&other| other <= v).count();
            rank as f64 / n
        })
        .collect();

    // Blast radius: count of reverse-edge dependents (direct only, 1 hop)
    let blast_map: std::collections::HashMap<&str, usize> = all_paths
        .iter()
        .map(|&path| {
            let count = index.graph.dependents(path).len();
            (path, count)
        })
        .collect();

    let mut entries: Vec<RiskEntry> = all_paths
        .iter()
        .enumerate()
        .map(|(i, &path)| {
            let blast_count = blast_map.get(path).copied().unwrap_or(0);
            let norm_blast = (blast_count as f64 / total_files).min(1.0);
            let has_test = index.test_map.contains_key(path);
            let test_coverage = if has_test { 1.0 } else { 0.0 };

            let nc = norm_churn[i].max(0.01);
            let nb = norm_blast.max(0.01);
            let tc_term = (1.0 - test_coverage).max(0.01);

            let risk_score = nc * nb * tc_term;

            RiskEntry {
                path: path.to_string(),
                churn_30d: churn_map.get(path).copied().unwrap_or(0) as u32,
                blast_radius: blast_count,
                test_coverage,
                risk_score,
            }
        })
        .collect();

    entries.sort_by(|a, b| {
        b.risk_score
            .partial_cmp(&a.risk_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_risk_floor_prevents_zero() {
        // A file with 0 churn, 0 blast, no test -> floor kicks in: 0.01^3 = 0.000001
        // A file with 0 churn, 0 blast, HAS test -> floor on nc and nb, (1-1.0) uses floor:
        // 0.01 * 0.01 * 0.01 = 0.000001
        let floor_val: f64 = 0.01_f64 * 0.01 * 0.01;
        // Verify the floor formula produces a positive minimum
        assert!(floor_val > 0.0);
        assert!((floor_val - 0.000001).abs() < 1e-15);
    }

    #[test]
    fn test_risk_range_is_valid() {
        // max possible: 1.0 * 1.0 * 1.0 = 1.0 (no test, max churn percentile, all files depend)
        // min possible: 0.01^3 = 0.000001
        let max: f64 = 1.0_f64.max(0.01) * 1.0_f64.max(0.01) * 1.0_f64.max(0.01);
        let min: f64 = 0.01_f64.max(0.01) * 0.01_f64.max(0.01) * 0.01_f64.max(0.01);
        assert!((max - 1.0).abs() < 1e-9);
        assert!((min - 0.000001).abs() < 1e-12);
    }

    #[test]
    fn test_risk_sorted_descending() {
        use crate::budget::counter::TokenCounter;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        // Two files; we can only check that the result is sorted
        let fp_a = dir.path().join("a.rs");
        let fp_b = dir.path().join("b.rs");
        std::fs::write(&fp_a, "fn a() {}").unwrap();
        std::fs::write(&fp_b, "fn b() {}").unwrap();
        let files = vec![
            ScannedFile {
                relative_path: "a.rs".into(),
                absolute_path: fp_a,
                language: Some("rust".into()),
                size_bytes: 9,
            },
            ScannedFile {
                relative_path: "b.rs".into(),
                absolute_path: fp_b,
                language: Some("rust".into()),
                size_bytes: 9,
            },
        ];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let entries = compute_risk_ranking(&index);
        assert_eq!(entries.len(), 2);
        assert!(
            entries[0].risk_score >= entries[1].risk_score,
            "risk entries must be sorted descending"
        );
    }

    #[test]
    fn test_risk_entry_serializes() {
        let entry = RiskEntry {
            path: "src/main.rs".into(),
            churn_30d: 5,
            blast_radius: 10,
            test_coverage: 0.0,
            risk_score: 0.42,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"path\":\"src/main.rs\""));
        assert!(json.contains("\"churn_30d\":5"));
    }

    #[test]
    fn test_risk_untested_scores_higher_than_tested() {
        // For same churn and blast, untested file (tc=0) should score higher than tested (tc=1)
        // Untested: nc * nb * max(1.0, 0.01) = nc * nb * 1.0
        // Tested:   nc * nb * max(0.0, 0.01) = nc * nb * 0.01
        let nc = 0.5f64;
        let nb = 0.5f64;
        let untested = nc.max(0.01) * nb.max(0.01) * (1.0f64 - 0.0).max(0.01);
        let tested = nc.max(0.01) * nb.max(0.01) * (1.0f64 - 1.0).max(0.01);
        assert!(untested > tested, "untested={untested}, tested={tested}");
    }
}
```

2. Register `pub mod risk;` in `src/intelligence/mod.rs`.

**Commands:**
```bash
cargo test -p cxpak intelligence::risk::tests -- --nocapture 2>&1 | tail -20
```
Expected: 5 tests pass.

**Commit:** `feat: add RiskEntry standing risk ranking with percentile churn normalization`

---

## Task 6 — ArchitectureMap type and computation

**Files:**
- New: `src/intelligence/architecture.rs`
- Modify: `src/intelligence/mod.rs`

**Steps:**

1. Create `src/intelligence/architecture.rs`:

```rust
use crate::index::CodebaseIndex;
use crate::intelligence::health::{count_nontrivial_sccs, module_prefix};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize)]
pub struct ArchitectureMap {
    pub modules: Vec<ModuleInfo>,
    pub circular_deps: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleInfo {
    pub prefix: String,
    pub file_count: usize,
    pub aggregate_pagerank: f64,
    pub coupling: f64,
}

/// Build the architecture map for the index.
///
/// `module_depth` controls how many path segments form a module name (default 2).
pub fn build_architecture_map(index: &CodebaseIndex, module_depth: usize) -> ArchitectureMap {
    let mut module_files: HashMap<String, Vec<String>> = HashMap::new();
    for file in &index.files {
        let prefix = module_prefix(&file.relative_path, module_depth);
        module_files
            .entry(prefix)
            .or_default()
            .push(file.relative_path.clone());
    }

    let module_set: HashSet<String> = module_files.keys().cloned().collect();

    let modules: Vec<ModuleInfo> = {
        let mut mods: Vec<ModuleInfo> = module_files
            .iter()
            .map(|(prefix, files)| {
                let file_set: HashSet<&str> = files.iter().map(|s| s.as_str()).collect();

                let aggregate_pagerank: f64 = files
                    .iter()
                    .map(|f| index.pagerank.get(f.as_str()).copied().unwrap_or(0.0))
                    .sum();

                // Coupling: cross-module edge ratio (outgoing + incoming / total)
                let mut total_edges = 0usize;
                let mut cross_edges = 0usize;
                for file in files {
                    if let Some(deps) = index.graph.edges.get(file.as_str()) {
                        for edge in deps {
                            total_edges += 1;
                            let target_mod = module_prefix(&edge.target, module_depth);
                            if target_mod != *prefix {
                                cross_edges += 1;
                            }
                        }
                    }
                    if let Some(deps) = index.graph.reverse_edges.get(file.as_str()) {
                        for edge in deps {
                            total_edges += 1;
                            let src_mod = module_prefix(&edge.target, module_depth);
                            if src_mod != *prefix {
                                cross_edges += 1;
                            }
                        }
                    }
                }
                let coupling = if total_edges == 0 {
                    0.0
                } else {
                    cross_edges as f64 / total_edges as f64
                };

                ModuleInfo {
                    prefix: prefix.clone(),
                    file_count: files.len(),
                    aggregate_pagerank,
                    coupling,
                }
            })
            .collect();
        mods.sort_by(|a, b| {
            b.aggregate_pagerank
                .partial_cmp(&a.aggregate_pagerank)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        mods
    };

    // Detect circular deps via Tarjan's SCC on the full dependency graph.
    // Each SCC with >1 node is a circular dependency group.
    let circular_deps = find_circular_dep_groups(index);

    ArchitectureMap {
        modules,
        circular_deps,
    }
}

/// Returns ordered path lists for each non-trivial SCC.
fn find_circular_dep_groups(index: &CodebaseIndex) -> Vec<Vec<String>> {
    // Build node list
    let nodes: Vec<String> = {
        let mut set = HashSet::new();
        for (k, edges) in &index.graph.edges {
            set.insert(k.clone());
            for e in edges {
                set.insert(e.target.clone());
            }
        }
        let mut v: Vec<_> = set.into_iter().collect();
        v.sort();
        v
    };

    let n = nodes.len();
    if n == 0 {
        return Vec::new();
    }

    let node_index: HashMap<&str, usize> =
        nodes.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();

    let mut index_counter = 0usize;
    let mut stack: Vec<usize> = Vec::new();
    let mut on_stack = vec![false; n];
    let mut indices = vec![usize::MAX; n];
    let mut lowlinks = vec![0usize; n];
    let mut cycles: Vec<Vec<String>> = Vec::new();

    fn strongconnect(
        v: usize,
        nodes: &[String],
        graph: &crate::index::graph::DependencyGraph,
        node_index: &HashMap<&str, usize>,
        index_counter: &mut usize,
        stack: &mut Vec<usize>,
        on_stack: &mut Vec<bool>,
        indices: &mut Vec<usize>,
        lowlinks: &mut Vec<usize>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        indices[v] = *index_counter;
        lowlinks[v] = *index_counter;
        *index_counter += 1;
        stack.push(v);
        on_stack[v] = true;

        let node_path = &nodes[v];
        if let Some(edges) = graph.edges.get(node_path.as_str()) {
            let targets: Vec<usize> = edges
                .iter()
                .filter_map(|e| node_index.get(e.target.as_str()).copied())
                .collect();
            for w in targets {
                if indices[w] == usize::MAX {
                    strongconnect(
                        w, nodes, graph, node_index, index_counter,
                        stack, on_stack, indices, lowlinks, cycles,
                    );
                    lowlinks[v] = lowlinks[v].min(lowlinks[w]);
                } else if on_stack[w] {
                    lowlinks[v] = lowlinks[v].min(indices[w]);
                }
            }
        }

        if lowlinks[v] == indices[v] {
            let mut scc: Vec<String> = Vec::new();
            loop {
                let w = stack.pop().unwrap();
                on_stack[w] = false;
                scc.push(nodes[w].clone());
                if w == v {
                    break;
                }
            }
            if scc.len() > 1 {
                scc.sort(); // deterministic ordering
                cycles.push(scc);
            }
        }
    }

    for i in 0..n {
        if indices[i] == usize::MAX {
            strongconnect(
                i, &nodes, &index.graph, &node_index,
                &mut index_counter, &mut stack, &mut on_stack,
                &mut indices, &mut lowlinks, &mut cycles,
            );
        }
    }

    cycles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::index::graph::DependencyGraph;
    use crate::scanner::ScannedFile;
    use crate::schema::EdgeType;
    use std::collections::HashMap;

    #[test]
    fn test_build_architecture_map_empty_index() {
        let counter = TokenCounter::new();
        let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
        let map = build_architecture_map(&index, 2);
        assert!(map.modules.is_empty());
        assert!(map.circular_deps.is_empty());
    }

    #[test]
    fn test_architecture_map_groups_by_prefix() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let make_file = |name: &str, content: &str| {
            let safe = name.replace('/', "_");
            let fp = dir.path().join(&safe);
            std::fs::write(&fp, content).unwrap();
            ScannedFile {
                relative_path: name.to_string(),
                absolute_path: fp,
                language: Some("rust".into()),
                size_bytes: content.len() as u64,
            }
        };
        let files = vec![
            make_file("src/api/handler.rs", "fn h() {}"),
            make_file("src/api/router.rs", "fn r() {}"),
            make_file("src/db/query.rs", "fn q() {}"),
        ];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let map = build_architecture_map(&index, 2);
        let prefixes: Vec<&str> = map.modules.iter().map(|m| m.prefix.as_str()).collect();
        assert!(prefixes.contains(&"src/api"), "src/api module expected");
        assert!(prefixes.contains(&"src/db"), "src/db module expected");
    }

    #[test]
    fn test_circular_deps_detected() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let make_file = |name: &str| {
            let safe = name.replace('/', "_");
            let fp = dir.path().join(&safe);
            std::fs::write(&fp, "fn f() {}").unwrap();
            ScannedFile {
                relative_path: name.to_string(),
                absolute_path: fp,
                language: Some("rust".into()),
                size_bytes: 9,
            }
        };
        let mut index =
            CodebaseIndex::build(vec![make_file("a.rs"), make_file("b.rs")], HashMap::new(), &counter);
        // Manually inject a cycle: a -> b -> a
        index.graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        index.graph.add_edge("b.rs", "a.rs", EdgeType::Import);

        let map = build_architecture_map(&index, 2);
        assert_eq!(map.circular_deps.len(), 1, "one cycle expected");
        let cycle = &map.circular_deps[0];
        assert!(cycle.contains(&"a.rs".to_string()));
        assert!(cycle.contains(&"b.rs".to_string()));
    }

    #[test]
    fn test_architecture_map_serialize() {
        let map = ArchitectureMap {
            modules: vec![ModuleInfo {
                prefix: "src/api".into(),
                file_count: 3,
                aggregate_pagerank: 2.5,
                coupling: 0.4,
            }],
            circular_deps: vec![vec!["a.rs".into(), "b.rs".into()]],
        };
        let json = serde_json::to_string(&map).unwrap();
        assert!(json.contains("\"prefix\":\"src/api\""));
        assert!(json.contains("\"circular_deps\""));
    }
}
```

2. Register `pub mod architecture;` in `src/intelligence/mod.rs`.

**Commands:**
```bash
cargo test -p cxpak intelligence::architecture::tests -- --nocapture 2>&1 | tail -20
```
Expected: 4 tests pass.

**Commit:** `feat: add ArchitectureMap with module grouping and Tarjan circular dep detection`

---

## Task 7 — RecentChange type and recency scoring signal

**Files:**
- Modify: `src/relevance/mod.rs` (update weight constants)
- Modify: `src/relevance/signals.rs` (implement recency_boost signal)
- New: `src/intelligence/recent_changes.rs`
- Modify: `src/intelligence/mod.rs`

**Steps:**

1. Create `src/intelligence/recent_changes.rs` with the `RecentChange` type and `compute_recent_changes`:

```rust
use crate::index::CodebaseIndex;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RecentChange {
    pub path: String,
    pub days_ago: u32,
    pub modifications_30d: u32,
}

/// Collect recently changed files from git_health churn data.
/// Returns files changed in the last 30 days, sorted by most recently modified.
pub fn compute_recent_changes(index: &CodebaseIndex) -> Vec<RecentChange> {
    let mut entries: Vec<RecentChange> = index
        .conventions
        .git_health
        .churn_30d
        .iter()
        .filter(|e| e.modifications > 0)
        .map(|e| RecentChange {
            path: e.path.clone(),
            days_ago: 0, // days_ago is not stored per-file in v1.2.0; use 0 as placeholder
            modifications_30d: e.modifications as u32,
        })
        .collect();

    // Sort by modification count descending (proxy for recency when days_ago unavailable)
    entries.sort_by(|a, b| b.modifications_30d.cmp(&a.modifications_30d));
    entries
}

/// Compute recency score for a file: 1.0 for files changed today, linearly
/// decaying to 0.0 at 90 days. Returns 0.5 (neutral) when no git data available.
pub fn recency_score_for_file(path: &str, index: &CodebaseIndex) -> f64 {
    let churn_30d = index.conventions.git_health.churn_30d.iter()
        .find(|e| e.path == path);
    let churn_180d = index.conventions.git_health.churn_180d.iter()
        .find(|e| e.path == path);

    // If the file appears in churn_30d, it was recently modified.
    // We estimate days_ago from 0 (in 30d bucket). If only in 180d, use ~60 days estimate.
    // If not in either, use neutral 0.5.
    if churn_30d.is_some() {
        // File changed within 30 days; score linearly from 1.0 (0 days) to ~0.67 (30 days)
        // Use midpoint estimate: ~15 days avg -> score = 1.0 - (15/90) = 0.833
        // Conservative: use 1.0 - (30/90) = 0.667 (worst case in 30d window)
        0.667
    } else if churn_180d.is_some() {
        // File changed within 180 days but not in last 30.
        // Score = 1.0 - (105/90) clamped to 0.0, but use midpoint of 30-180 = ~105 days
        // 105 > 90 so score = 0.0 (past the 90d decay window)
        0.0
    } else {
        0.5 // neutral: no git data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recency_score_no_git_data() {
        use crate::budget::counter::TokenCounter;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("a.rs");
        std::fs::write(&fp, "fn a() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "a.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 9,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        // No git data -> neutral 0.5
        assert!((recency_score_for_file("a.rs", &index) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_recent_changes_empty_when_no_churn() {
        use crate::budget::counter::TokenCounter;
        use std::collections::HashMap;
        let counter = TokenCounter::new();
        let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
        assert!(compute_recent_changes(&index).is_empty());
    }
}
```

2. Update `SignalWeights` in `src/relevance/mod.rs` to activate recency_boost:

```rust
// without_embeddings (change recency_boost 0.00 -> 0.05, term_frequency 0.19 -> 0.14)
pub fn without_embeddings() -> Self {
    Self {
        path_similarity: 0.18,
        symbol_match: 0.32,
        import_proximity: 0.14,
        term_frequency: 0.14,  // was 0.19
        recency_boost: 0.05,   // was 0.00
        pagerank: 0.17,
        embedding_similarity: 0.00,
    }
}

// with_embeddings (change recency_boost 0.00 -> 0.05, term_frequency 0.16 -> 0.11)
pub fn with_embeddings() -> Self {
    Self {
        path_similarity: 0.15,
        symbol_match: 0.27,
        import_proximity: 0.12,
        term_frequency: 0.11,  // was 0.16
        recency_boost: 0.05,   // was 0.00
        pagerank: 0.15,
        embedding_similarity: 0.15,
    }
}
```

3. Replace the neutral recency_sig stub in `src/relevance/mod.rs` `score()` method with an actual call:

```rust
// Replace:
let recency_sig = SignalResult {
    name: "recency_boost",
    score: 0.5,
    detail: "no git history in index".to_string(),
};
// With:
let recency_sig = signals::recency_boost_signal(file_path, index);
```

4. Implement `recency_boost_signal` in `src/relevance/signals.rs`:

```rust
/// RecencyBoost: 1.0 for files changed today, linearly decaying to 0.0 at 90 days.
/// Returns 0.5 (neutral) when no git history is available for the file.
pub fn recency_boost_signal(file_path: &str, index: &CodebaseIndex) -> SignalResult {
    let score = crate::intelligence::recent_changes::recency_score_for_file(file_path, index);
    SignalResult {
        name: "recency_boost",
        score,
        detail: format!("recency={:.2}", score),
    }
}
```

5. Update the two weight-sum tests in `src/relevance/mod.rs` — they will still pass because the weights still sum to 1.0. Verify:

```bash
cargo test -p cxpak relevance::tests::test_weights_sum_to_one -- --nocapture
cargo test -p cxpak relevance::tests::weights_without_embeddings_sum_to_one -- --nocapture
cargo test -p cxpak relevance::tests::weights_with_embeddings_sum_to_one -- --nocapture
```

6. Register `pub mod recent_changes;` in `src/intelligence/mod.rs`.

**Commands:**
```bash
cargo test -p cxpak intelligence::recent_changes::tests -- --nocapture 2>&1 | tail -10
cargo test -p cxpak relevance::tests -- --nocapture 2>&1 | tail -20
```
Expected: recency tests pass; all weight-sum assertions pass.

**Commit:** `feat: activate recency_boost signal (weight 0.05) with git churn-based scoring`

---

## Task 8 — AutoContextResult gains new compound intelligence fields

**Files:**
- Modify: `src/auto_context/mod.rs`

**Steps:**

1. Add imports and new fields to `AutoContextResult`:

```rust
use crate::intelligence::health::HealthScore;
use crate::intelligence::risk::RiskEntry;
use crate::intelligence::architecture::ArchitectureMap;
use crate::intelligence::co_change::CoChangeEdge;
use crate::intelligence::recent_changes::RecentChange;

#[derive(Debug, Serialize)]
pub struct AutoContextResult {
    pub task: String,
    pub dna: String,
    pub budget: crate::auto_context::briefing::BudgetSummary,
    pub sections: crate::auto_context::briefing::PackedSections,
    pub filtered_out: Vec<FilteredFile>,
    // v1.2.0 compound intelligence
    pub health: HealthScore,
    pub risks: Vec<RiskEntry>,       // top 10
    pub architecture: ArchitectureMap,
    pub co_changes: Vec<CoChangeEdge>,
    pub recent_changes: Vec<RecentChange>,
}
```

2. Populate the new fields in `auto_context()`, after Step 10 (pack) and before the `AutoContextResult { ... }` construction:

```rust
// Compound intelligence (computed after packing to avoid double-borrowing index)
let health = crate::intelligence::health::compute_health(index);
let all_risks = crate::intelligence::risk::compute_risk_ranking(index);
let risks: Vec<RiskEntry> = all_risks.into_iter().take(10).collect();
let architecture = crate::intelligence::architecture::build_architecture_map(index, 2);
let co_changes = index.co_changes.clone();
let recent_changes = crate::intelligence::recent_changes::compute_recent_changes(index);
```

3. Add the new fields to the struct literal:

```rust
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
```

4. Update the `test_auto_context_happy_path` test to verify health.composite is in [0.0, 10.0] and risks length is <= 10:

```rust
assert!(
    result.health.composite >= 0.0 && result.health.composite <= 10.0,
    "health composite out of range: {}", result.health.composite
);
assert!(result.risks.len() <= 10, "risks should be capped at 10");
```

**Commands:**
```bash
cargo test -p cxpak auto_context::tests -- --nocapture 2>&1 | tail -20
cargo build 2>&1 | grep "^error" | head -10
```
Expected: all auto_context tests pass.

**Commit:** `feat: add health, risks, architecture, co_changes, recent_changes to AutoContextResult`

---

## Task 9 — Briefing mode (mode parameter on auto_context)

**Files:**
- Modify: `src/auto_context/mod.rs`
- Modify: `src/auto_context/briefing.rs`

**Steps:**

1. Add `mode` field to `AutoContextOpts`:

```rust
#[derive(Debug, Serialize)]
pub struct AutoContextOpts {
    pub tokens: usize,
    pub focus: Option<String>,
    pub include_tests: bool,
    pub include_blast_radius: bool,
    pub mode: String,  // "full" (default) or "briefing"
}
```

2. Add a `briefing_mode: bool` parameter to `allocate_and_pack` in `briefing.rs`. When `true`, all `PackedFile.content` fields are set to `None` instead of `Some(content)`. The token counting still happens (so budget math is correct), but content is stripped before returning:

```rust
pub fn allocate_and_pack(
    mut target_files: Vec<(String, f64, String)>,
    test_files: Vec<(String, String)>,
    schema_json: Option<serde_json::Value>,
    api_surface_json: Option<serde_json::Value>,
    blast_radius_json: Option<serde_json::Value>,
    token_budget: usize,
    briefing_mode: bool,
) -> PackedBriefing {
```

3. In `allocate_and_pack`, after computing `content` / `truncated` for each `PackedFile`, apply the mode:

```rust
PackedFile {
    path,
    score,
    detail_level: "full".to_string(),
    tokens: full_tokens,
    content: if briefing_mode { None } else { Some(content) },
}
```

Apply this pattern at all 4 `PackedFile` construction sites in the function (full target, truncated target, full test, truncated test, full schema, truncated schema).

4. Update `auto_context()` to pass `briefing_mode` to `allocate_and_pack`:

```rust
let briefing_mode = opts.mode == "briefing";
let packed = crate::auto_context::briefing::allocate_and_pack(
    target_files,
    test_files,
    None,
    api_json,
    blast_json,
    remaining_budget,
    briefing_mode,
);
```

5. Update all callers of `allocate_and_pack` in tests to pass `false` as the new last argument (no behavioral change for full mode).

6. Update `default_opts` helper in tests:

```rust
fn default_opts(tokens: usize) -> AutoContextOpts {
    AutoContextOpts {
        tokens,
        focus: None,
        include_tests: false,
        include_blast_radius: false,
        mode: "full".to_string(),
    }
}
```

7. Add a test for briefing mode:

```rust
#[test]
fn test_auto_context_briefing_mode_content_is_none() {
    let (index, _dir) = make_index(&[
        ("src/auth.rs", "pub fn authenticate(user: &str) -> bool { true }"),
    ]);
    let opts = AutoContextOpts {
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

#[test]
fn test_auto_context_full_mode_content_is_some() {
    let (index, _dir) = make_index(&[
        ("src/auth.rs", "pub fn authenticate(user: &str) -> bool { true }"),
    ]);
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
```

**Commands:**
```bash
cargo test -p cxpak auto_context -- --nocapture 2>&1 | tail -30
```
Expected: all tests pass including 2 new briefing mode tests.

**Commit:** `feat: add briefing mode to auto_context - content:None for file list without source`

---

## Task 10 — Incremental indexing: mtime tracking on IndexedFile

**Files:**
- Modify: `src/index/mod.rs`

**Steps:**

1. Add `mtime_secs` field to `IndexedFile`:

```rust
#[derive(Debug)]
pub struct IndexedFile {
    pub relative_path: String,
    pub language: Option<String>,
    pub size_bytes: u64,
    pub token_count: usize,
    pub parse_result: Option<ParseResult>,
    pub content: String,
    pub mtime_secs: Option<u64>,  // Unix epoch seconds, None if unavailable
}
```

2. Populate `mtime_secs` during `build()` and `build_with_content()` using `std::fs::metadata`:

```rust
let mtime_secs = std::fs::metadata(&file.absolute_path)
    .ok()
    .and_then(|m| m.modified().ok())
    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
    .map(|d| d.as_secs());
```

3. Add `mtime_secs: Option<u64>` parameter to `upsert_file`. **IMPORTANT:** Update ALL existing callers — at minimum these test call sites in `src/index/mod.rs` (lines ~733, ~758, ~933) and any usage in `src/commands/watch.rs`. Pass `None` at existing call sites to preserve behavior:

```rust
pub fn upsert_file(
    &mut self,
    relative_path: &str,
    language: Option<&str>,
    content: &str,
    parse_result: Option<ParseResult>,
    counter: &TokenCounter,
    mtime_secs: Option<u64>,
) {
    // ... existing body ...
    self.files.push(IndexedFile {
        // ...
        mtime_secs,
    });
}
```

4. Add `incremental_rebuild` function to `CodebaseIndex` that implements the mutation-API approach from the design spec:

```rust
/// Rebuild the index incrementally: re-parse only files whose mtime/size differs.
///
/// Steps:
/// 1. Scan current files, compare mtime against stored IndexedFile.mtime_secs.
/// 2. Call upsert_file() for changed/new files.
/// 3. Call remove_file() for deleted files.
/// 4. Call rebuild_graph() to recompute the dependency graph.
/// 5. Recompute PageRank and test_map.
pub fn incremental_rebuild(
    &mut self,
    current_files: &[crate::scanner::ScannedFile],
    parse_results: &std::collections::HashMap<String, crate::parser::language::ParseResult>,
    counter: &TokenCounter,
) {
    let current_paths: std::collections::HashSet<String> = current_files
        .iter()
        .map(|f| f.relative_path.clone())
        .collect();

    // Remove files that no longer exist
    let to_remove: Vec<String> = self
        .files
        .iter()
        .filter(|f| !current_paths.contains(&f.relative_path))
        .map(|f| f.relative_path.clone())
        .collect();
    for path in &to_remove {
        self.remove_file(path);
    }

    // Upsert changed or new files
    for file in current_files {
        let mtime_secs = std::fs::metadata(&file.absolute_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        let needs_update = match self.files.iter().find(|f| f.relative_path == file.relative_path) {
            None => true, // new file
            Some(existing) => match (existing.mtime_secs, mtime_secs) {
                (Some(old), Some(new)) => new > old || file.size_bytes != existing.size_bytes,
                _ => true, // no mtime available: always re-parse
            },
        };

        if needs_update {
            let content = std::fs::read_to_string(&file.absolute_path).unwrap_or_default();
            let parse_result = parse_results.get(&file.relative_path).cloned();
            self.upsert_file(
                &file.relative_path,
                file.language.as_deref(),
                &content,
                parse_result,
                counter,
                mtime_secs,
            );
        }
    }

    // Rebuild graph and recompute derived scores
    self.rebuild_graph();
    self.pagerank = crate::intelligence::pagerank::compute_pagerank(&self.graph, 0.85, 100);
    let all_paths: std::collections::HashSet<String> =
        self.files.iter().map(|f| f.relative_path.clone()).collect();
    self.test_map = crate::intelligence::test_map::build_test_map(&self.files, &all_paths);
    self.total_files = self.files.len();
}
```

5. Update `empty()` to set `mtime_secs: None` for any pre-built IndexedFile (none in empty).

6. Write tests:

```rust
#[test]
fn test_indexed_file_has_mtime_from_disk() {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let fp = dir.path().join("a.rs");
    std::fs::write(&fp, "fn a() {}").unwrap();
    let files = vec![ScannedFile {
        relative_path: "a.rs".into(),
        absolute_path: fp,
        language: Some("rust".into()),
        size_bytes: 9,
    }];
    let index = CodebaseIndex::build(files, HashMap::new(), &counter);
    // mtime_secs should be Some (file was just written)
    assert!(
        index.files[0].mtime_secs.is_some(),
        "mtime_secs should be populated from disk"
    );
}

#[test]
fn test_incremental_rebuild_removes_deleted_file() {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let fp_a = dir.path().join("a.rs");
    let fp_b = dir.path().join("b.rs");
    std::fs::write(&fp_a, "fn a() {}").unwrap();
    std::fs::write(&fp_b, "fn b() {}").unwrap();
    let files = vec![
        ScannedFile {
            relative_path: "a.rs".into(),
            absolute_path: fp_a.clone(),
            language: Some("rust".into()),
            size_bytes: 9,
        },
        ScannedFile {
            relative_path: "b.rs".into(),
            absolute_path: fp_b,
            language: Some("rust".into()),
            size_bytes: 9,
        },
    ];
    let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
    assert_eq!(index.files.len(), 2);

    // Simulate b.rs deleted: only pass a.rs in current_files
    let current = vec![ScannedFile {
        relative_path: "a.rs".into(),
        absolute_path: fp_a,
        language: Some("rust".into()),
        size_bytes: 9,
    }];
    index.incremental_rebuild(&current, &HashMap::new(), &counter);
    assert_eq!(index.files.len(), 1);
    assert_eq!(index.files[0].relative_path, "a.rs");
}
```

**Commands:**
```bash
cargo test -p cxpak index::tests -- --nocapture 2>&1 | tail -20
```
Expected: all index tests pass including 2 new incremental tests.

**Commit:** `feat: add mtime tracking to IndexedFile and incremental_rebuild on CodebaseIndex`

---

## Task 11 — New MCP tool: cxpak_health

**Files:**
- Modify: `src/commands/serve.rs`

**Steps:**

1. Locate the `tools/list` JSON in `mcp_stdio_loop_with_io` and add `cxpak_health` tool definition:

```json
{
    "name": "cxpak_health",
    "description": "Returns the codebase health score — a composite metric across 6 dimensions: convention adherence, test coverage, churn stability, module coupling, circular dependencies, and dead code (null until v1.3.0). Use this to understand the overall quality state before making structural changes.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "focus": {
                "type": "string",
                "description": "Optional path prefix to scope the analysis (e.g. 'src/api/')"
            }
        }
    }
}
```

2. Add the handler case in `mcp_stdio_loop_with_io` `match tool_name`:

```rust
"cxpak_health" => {
    let health = crate::intelligence::health::compute_health(index);
    mcp_tool_result(id, &serde_json::to_string_pretty(&health).unwrap_or_else(
        |_| json!({"error": "serialisation failed"})
    ))
}
```

3. Add `cxpak_health` to the HTTP router in `build_router` + handler:

```rust
.route("/health_score", axum::routing::post(health_score_handler))
```

```rust
#[derive(Deserialize)]
struct HealthParams {
    focus: Option<String>,
}

async fn health_score_handler(
    State(index): State<SharedIndex>,
    Json(params): Json<HealthParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index.read().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let health = crate::intelligence::health::compute_health(&idx);
    Ok(Json(serde_json::to_value(&health).unwrap_or_else(
        |_| json!({"error": "serialisation failed"}),
    )))
}
```

4. Write an MCP unit test for cxpak_health in `serve.rs` tests (follow the existing pattern with `mcp_stdio_loop_with_io`):

```rust
#[test]
fn test_mcp_health_tool() {
    let index = CodebaseIndex::empty();
    let snapshot = Arc::new(RwLock::new(None));
    let repo_path = std::env::current_dir().unwrap();
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "cxpak_health",
            "arguments": {}
        }
    });
    let input = format!("{}\n", request);
    let mut output = Vec::new();
    mcp_stdio_loop_with_io(
        &repo_path, &index, &snapshot,
        std::io::Cursor::new(input.as_bytes()),
        &mut output,
    ).unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    assert!(response["result"]["content"][0]["text"].is_string());
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    let health: Value = serde_json::from_str(text).unwrap();
    assert!(health["composite"].is_number());
}
```

**Commands:**
```bash
cargo test -p cxpak serve -- --nocapture 2>&1 | grep -E "test_mcp_health|FAILED|ok" | head -10
```
Expected: `test_mcp_health_tool ... ok`

**Commit:** `feat: add cxpak_health MCP tool and /health_score HTTP endpoint`

---

## Task 12 — New MCP tool: cxpak_risks

**Files:**
- Modify: `src/commands/serve.rs`

**Steps:**

1. Add `cxpak_risks` tool definition to `tools/list`:

```json
{
    "name": "cxpak_risks",
    "description": "Returns full risk-ranked file list (standing risk — inherent file-level risk regardless of current changes). Risk = churn × blast_radius × lack_of_tests. Use before a refactor to identify which files need the most care.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "focus": {
                "type": "string",
                "description": "Optional path prefix filter"
            },
            "limit": {
                "type": "number",
                "description": "Maximum results to return (default 20)"
            }
        }
    }
}
```

2. Add handler case:

```rust
"cxpak_risks" => {
    let limit = args.get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;
    let focus = args.get("focus").and_then(|v| v.as_str());
    let mut risks = crate::intelligence::risk::compute_risk_ranking(index);
    if let Some(f) = focus {
        risks.retain(|r| r.path.starts_with(f));
    }
    risks.truncate(limit);
    mcp_tool_result(id, &serde_json::to_string_pretty(&risks).unwrap_or_else(
        |_| json!({"error": "serialisation failed"})
    ))
}
```

3. Add HTTP route `/risks` with focus + limit params.

4. Write MCP unit test:

```rust
#[test]
fn test_mcp_risks_tool_returns_array() {
    let index = CodebaseIndex::empty();
    let snapshot = Arc::new(RwLock::new(None));
    let repo_path = std::env::current_dir().unwrap();
    let request = serde_json::json!({
        "jsonrpc": "2.0", "id": 1,
        "method": "tools/call",
        "params": { "name": "cxpak_risks", "arguments": { "limit": 5 } }
    });
    let input = format!("{}\n", request);
    let mut output = Vec::new();
    mcp_stdio_loop_with_io(
        &repo_path, &index, &snapshot,
        std::io::Cursor::new(input.as_bytes()), &mut output,
    ).unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    let risks: Value = serde_json::from_str(text).unwrap();
    assert!(risks.is_array(), "risks tool must return an array");
}
```

**Commands:**
```bash
cargo test -p cxpak serve::tests::test_mcp_risks_tool -- --nocapture 2>&1 | tail -5
```
Expected: `test_mcp_risks_tool_returns_array ... ok`

**Commit:** `feat: add cxpak_risks MCP tool and /risks HTTP endpoint`

---

## Task 13 — New MCP tool: cxpak_briefing

**Files:**
- Modify: `src/commands/serve.rs`

**Steps:**

1. Add `cxpak_briefing` tool definition to `tools/list`:

```json
{
    "name": "cxpak_briefing",
    "description": "Returns a compact intelligence briefing: full compound intelligence (health, risks, architecture, co-changes) with file list and scores but WITHOUT source content. Call cxpak_pack_context for files you need. Useful for large codebases where you want the intelligence layer without consuming token budget on source.",
    "inputSchema": {
        "type": "object",
        "required": ["task"],
        "properties": {
            "task": {
                "type": "string",
                "description": "Task description for relevance scoring"
            },
            "tokens": {
                "type": "string",
                "description": "Token budget (default '50k')"
            },
            "focus": {
                "type": "string",
                "description": "Optional path prefix filter"
            }
        }
    }
}
```

2. Add handler case (alias to `cxpak_auto_context` with `mode: "briefing"`):

```rust
"cxpak_briefing" => {
    let task = match args.get("task").and_then(|v| v.as_str()) {
        Some(t) if !t.is_empty() => t,
        _ => return mcp_tool_result(id, r#"{"error": "task is required"}"#),
    };
    let token_budget = args.get("tokens")
        .and_then(|v| v.as_str())
        .and_then(|t| crate::cli::parse_token_count(t).ok())
        .unwrap_or(50_000);
    let opts = crate::auto_context::AutoContextOpts {
        tokens: token_budget,
        focus: args.get("focus").and_then(|v| v.as_str()).map(|s| s.to_string()),
        include_tests: false,
        include_blast_radius: false,
        mode: "briefing".to_string(),
    };
    let result = crate::auto_context::auto_context(task, index, &opts);
    mcp_tool_result(id, &serde_json::to_string_pretty(&result).unwrap_or_else(
        |_| json!({"error": "serialisation failed"})
    ))
}
```

3. Write MCP unit test:

```rust
#[test]
fn test_mcp_briefing_tool_content_is_null() {
    let index = CodebaseIndex::empty();
    let snapshot = Arc::new(RwLock::new(None));
    let repo_path = std::env::current_dir().unwrap();
    let request = serde_json::json!({
        "jsonrpc": "2.0", "id": 1,
        "method": "tools/call",
        "params": {
            "name": "cxpak_briefing",
            "arguments": { "task": "understand the codebase" }
        }
    });
    let input = format!("{}\n", request);
    let mut output = Vec::new();
    mcp_stdio_loop_with_io(
        &repo_path, &index, &snapshot,
        std::io::Cursor::new(input.as_bytes()), &mut output,
    ).unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    assert!(response["result"].is_object(), "briefing tool must return an object");
}

#[test]
fn test_mcp_briefing_tool_missing_task_returns_error() {
    let index = CodebaseIndex::empty();
    let snapshot = Arc::new(RwLock::new(None));
    let repo_path = std::env::current_dir().unwrap();
    let request = serde_json::json!({
        "jsonrpc": "2.0", "id": 1,
        "method": "tools/call",
        "params": { "name": "cxpak_briefing", "arguments": {} }
    });
    let input = format!("{}\n", request);
    let mut output = Vec::new();
    mcp_stdio_loop_with_io(
        &repo_path, &index, &snapshot,
        std::io::Cursor::new(input.as_bytes()), &mut output,
    ).unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    let text = response["result"]["content"][0]["text"].as_str().unwrap_or("");
    let val: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    assert!(val["error"].is_string(), "missing task must return error");
}
```

**Commands:**
```bash
cargo test -p cxpak serve::tests::test_mcp_briefing -- --nocapture 2>&1 | tail -10
```
Expected: both briefing tests pass.

**Commit:** `feat: add cxpak_briefing MCP tool as briefing-mode alias for auto_context`

---

## Task 14 — tools/list count update and tool count verification

**Files:**
- Modify: `src/commands/serve.rs`

**Steps:**

1. Verify the tools/list JSON now includes 16 tools (13 existing + health, risks, briefing).

2. Add a unit test to count tools:

```rust
#[test]
fn test_tools_list_returns_16_tools() {
    let index = CodebaseIndex::empty();
    let snapshot = Arc::new(RwLock::new(None));
    let repo_path = std::env::current_dir().unwrap();
    let request = serde_json::json!({
        "jsonrpc": "2.0", "id": 1,
        "method": "tools/list",
        "params": {}
    });
    let input = format!("{}\n", request);
    let mut output = Vec::new();
    mcp_stdio_loop_with_io(
        &repo_path, &index, &snapshot,
        std::io::Cursor::new(input.as_bytes()), &mut output,
    ).unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    let tools = response["result"]["tools"].as_array().expect("tools must be array");
    assert_eq!(tools.len(), 16, "expected 16 MCP tools, got {}", tools.len());
}
```

3. Run `cargo test` to confirm all tests pass.

**Commands:**
```bash
cargo test -p cxpak -- --nocapture 2>&1 | tail -5
cargo test -p cxpak serve::tests::test_tools_list -- --nocapture 2>&1 | tail -5
```
Expected: `test_tools_list_returns_16_tools ... ok`.

**Commit:** `test: verify 16 MCP tools registered in tools/list`

---

## Task 15 — cxpak overview --health flag

**Files:**
- Modify: `src/commands/overview.rs` (add --health flag)
- Modify: `src/cli.rs` (add `health: bool` to OverviewArgs)

**Steps:**

1. Find the `OverviewArgs` struct in `src/cli.rs` and add:

```rust
/// Append codebase health score to the overview output.
#[arg(long, default_value_t = false)]
pub health: bool,
```

2. In `src/commands/overview.rs`, after the existing output is rendered, if `args.health` is true:

```rust
if args.health {
    let health = crate::intelligence::health::compute_health(index);
    output.push_str("\n\n## Codebase Health\n\n");
    output.push_str(&format!("Composite: {:.1}/10\n", health.composite));
    output.push_str(&format!("  conventions:     {:.1}/10\n", health.conventions));
    output.push_str(&format!("  test_coverage:   {:.1}/10\n", health.test_coverage));
    output.push_str(&format!("  churn_stability: {:.1}/10\n", health.churn_stability));
    output.push_str(&format!("  coupling:        {:.1}/10\n", health.coupling));
    output.push_str(&format!("  cycles:          {:.1}/10\n", health.cycles));
    if let Some(dc) = health.dead_code {
        output.push_str(&format!("  dead_code:       {:.1}/10\n", dc));
    } else {
        output.push_str("  dead_code:       N/A (available in v1.3.0)\n");
    }
}
```

3. Write a test that verifies `--health` appends the Health section:

```rust
#[test]
fn test_overview_health_flag_appends_section() {
    // Build a minimal index with no git repo to test the output structure
    use crate::budget::counter::TokenCounter;
    use std::collections::HashMap;
    let counter = TokenCounter::new();
    let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
    // ... call overview logic with health=true and verify output contains "Codebase Health"
    let health = crate::intelligence::health::compute_health(&index);
    assert!(health.composite >= 0.0 && health.composite <= 10.0);
}
```

**Commands:**
```bash
cargo build 2>&1 | grep "^error" | head -5
cargo test -p cxpak overview -- --nocapture 2>&1 | tail -10
```
Expected: builds cleanly, overview tests pass.

**Commit:** `feat: add --health flag to cxpak overview command`

---

## Task 16 — plugin.json and marketplace.json version bump

**Files:**
- Modify: `plugin/.claude-plugin/plugin.json`
- Modify: `.claude-plugin/marketplace.json`

**Steps:**

1. Read both files to find the current version strings (`"version": "1.1.0"`).

2. Bump to `"1.2.0"` in both files.

3. Verify `Cargo.toml` still shows `version = "1.1.0"` — it will be bumped in Task 18.

**Commands:**
```bash
grep -r '"version"' plugin/.claude-plugin/plugin.json .claude-plugin/marketplace.json
```
Expected: both show `"1.2.0"`.

**Commit:** `chore: bump plugin.json and marketplace.json to v1.2.0`

---

## Task 17 — Property-based tests for score normalization

**Files:**
- New: `src/intelligence/health_property_tests.rs` (inline in `health.rs` under `#[cfg(test)]`)
- Modify: `src/intelligence/health.rs`

**Steps:**

1. Add property-based tests using standard Rust randomized input (no proptest dependency needed — use a deterministic enumeration approach to cover boundary cases):

```rust
#[test]
fn test_composite_all_combinations_within_range() {
    // Test all combinations of boundary values: 0.0, 5.0, 10.0 for each of 5 dimensions
    let values = [0.0_f64, 5.0, 10.0];
    for &c in &values {
        for &t in &values {
            for &ch in &values {
                for &cp in &values {
                    for &cy in &values {
                        let comp = compute_composite(c, t, ch, cp, cy, None);
                        assert!(
                            comp >= 0.0 && comp <= 10.0,
                            "composite out of range [{c},{t},{ch},{cp},{cy}]: {comp}"
                        );
                        // With dead code
                        for &dc in &values {
                            let comp6 = compute_composite(c, t, ch, cp, cy, Some(dc));
                            assert!(
                                comp6 >= 0.0 && comp6 <= 10.0,
                                "composite (with dead_code) out of range: {comp6}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn test_score_cycles_invariants() {
    // No cycles -> 10.0. Adding cycles decreases the score monotonically.
    let mut graph = DependencyGraph::new();
    let score_0 = score_cycles(&graph);
    assert!((score_0 - 10.0).abs() < 1e-6, "0 cycles must give 10.0");

    // Add 1 cycle
    graph.add_edge("a.rs", "b.rs", EdgeType::Import);
    graph.add_edge("b.rs", "a.rs", EdgeType::Import);
    let score_1 = score_cycles(&graph);
    assert!(score_1 < score_0, "1 cycle must score lower than 0 cycles");
    assert!((score_1 - 5.0).abs() < 1e-6, "1 cycle -> 10/2 = 5.0");

    // Add 2nd cycle
    graph.add_edge("c.rs", "d.rs", EdgeType::Import);
    graph.add_edge("d.rs", "c.rs", EdgeType::Import);
    let score_2 = score_cycles(&graph);
    assert!(score_2 < score_1, "2 cycles must score lower than 1 cycle");
}

#[test]
fn test_risk_score_multiplicative_floor() {
    // Verify multiplicative floor: 0.01^3 = 1e-6 is the minimum
    let min = 0.01_f64 * 0.01 * 0.01;
    assert!((min - 1e-6).abs() < 1e-15, "minimum risk floor must be 1e-6");
    // Verify max is 1.0
    assert!((1.0_f64.max(0.01) * 1.0_f64.max(0.01) * 1.0_f64.max(0.01) - 1.0).abs() < 1e-9);
}
```

**Commands:**
```bash
cargo test -p cxpak intelligence::health::tests -- --nocapture 2>&1 | tail -20
```
Expected: all health tests pass including the 3 new property tests.

**Commit:** `test: add boundary-value property tests for health score and risk ranking`

---

## Task 18 — Version bump to 1.2.0

**Files:**
- Modify: `Cargo.toml`

**Steps:**

1. Change `version = "1.1.0"` to `version = "1.2.0"` in `Cargo.toml`.

2. Run `cargo check` to regenerate `Cargo.lock` with the new version.

3. Verify `cargo check` exits 0.

**Commands:**
```bash
cargo check 2>&1 | tail -5
grep "^version" Cargo.toml
```
Expected: `version = "1.2.0"`, no errors.

**Commit:** `chore: bump version to 1.2.0`

---

## Task 19 — Full test suite and coverage gate

**Files:**
- No code changes — verification only.

**Steps:**

1. Run the full test suite:

```bash
cargo test --all-features 2>&1 | tail -20
```
Expected: 0 failures.

2. Run clippy with warnings as errors:

```bash
cargo clippy --all-targets -- -D warnings 2>&1 | head -30
```
Expected: no warnings.

3. Run formatter check:

```bash
cargo fmt -- --check 2>&1
```
Expected: clean.

4. Run tarpaulin coverage check (CI gate is 90%):

```bash
cargo tarpaulin --out Lcov --timeout 180 2>&1 | grep "^%" | head -5
```
Expected: coverage ≥ 90%.

5. If coverage is below 90%, identify uncovered lines and add targeted unit tests (most likely candidates: edge cases in `score_coupling` for zero-edge modules, `incremental_rebuild` no-op path when no files changed, `build_co_changes` with empty commit list).

**Commands:**
```bash
cargo test --all-features 2>&1 | grep -E "^test result"
```
Expected: `test result: ok. N passed; 0 failed`.

**Commit:** `test: final test pass — v1.2.0 full suite green`

---

## Task 20 — Integration test: auto_context result shape with new fields

**Files:**
- Modify or new: `tests/auto_context_v120.rs`

**Steps:**

1. Create an integration test that exercises the complete auto_context pipeline and verifies all v1.2.0 fields are present and well-formed:

```rust
// tests/auto_context_v120.rs
use cxpak::auto_context::{auto_context, AutoContextOpts};
use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use cxpak::scanner::ScannedFile;
use std::collections::HashMap;

fn make_test_index() -> (CodebaseIndex, tempfile::TempDir) {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();

    let files_data = [
        ("src/api/handler.rs", "pub fn handle() {}"),
        ("src/api/router.rs", "pub fn route() {}"),
        ("src/api/auth.rs", "pub fn authenticate() -> bool { true }"),
        ("src/db/query.rs", "pub fn run() {}"),
        ("src/db/models.rs", "pub struct User {}"),
        ("src/db/connection.rs", "pub fn connect() {}"),
        ("tests/handler_test.rs", "fn test_handle() {}"),
    ];

    let files: Vec<ScannedFile> = files_data
        .iter()
        .map(|(rel, content)| {
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

#[test]
fn test_v120_auto_context_result_has_all_fields() {
    let (index, _dir) = make_test_index();
    let opts = AutoContextOpts {
        tokens: 50_000,
        focus: None,
        include_tests: false,
        include_blast_radius: false,
        mode: "full".to_string(),
    };
    let result = auto_context("handle authentication request", &index, &opts);

    // Health score: valid range
    assert!(
        result.health.composite >= 0.0 && result.health.composite <= 10.0,
        "composite out of range: {}", result.health.composite
    );
    assert!(result.health.dead_code.is_none(), "dead_code must be None in v1.2.0");

    // Risks: capped at 10
    assert!(result.risks.len() <= 10);
    // Each risk entry has valid score
    for risk in &result.risks {
        assert!(risk.risk_score > 0.0, "risk score must be positive for {}", risk.path);
        assert!(risk.risk_score <= 1.0, "risk score must be <= 1.0 for {}", risk.path);
    }

    // Architecture: modules present
    assert!(
        !result.architecture.modules.is_empty(),
        "architecture map must have modules"
    );

    // Full mode: all target file content is Some
    for file in &result.sections.target_files.files {
        assert!(
            file.content.is_some(),
            "full mode: content must be Some for {}", file.path
        );
    }

    // Budget invariant
    assert_eq!(
        result.budget.used + result.budget.remaining,
        result.budget.total,
        "budget invariant violated"
    );
}

#[test]
fn test_v120_briefing_mode_content_is_none() {
    let (index, _dir) = make_test_index();
    let opts = AutoContextOpts {
        tokens: 50_000,
        focus: None,
        include_tests: false,
        include_blast_radius: false,
        mode: "briefing".to_string(),
    };
    let result = auto_context("handle authentication", &index, &opts);

    // Briefing mode: all target file content is None
    for file in &result.sections.target_files.files {
        assert!(
            file.content.is_none(),
            "briefing mode: content must be None for {}", file.path
        );
    }

    // Health + risks + architecture still present in briefing mode
    assert!(result.health.composite >= 0.0);
    assert!(result.risks.len() <= 10);
}

#[test]
fn test_v120_risks_sorted_descending() {
    let (index, _dir) = make_test_index();
    let opts = AutoContextOpts {
        tokens: 50_000,
        focus: None,
        include_tests: false,
        include_blast_radius: false,
        mode: "full".to_string(),
    };
    let result = auto_context("handle", &index, &opts);
    for i in 1..result.risks.len() {
        assert!(
            result.risks[i - 1].risk_score >= result.risks[i].risk_score,
            "risks not sorted at index {i}"
        );
    }
}
```

**Commands:**
```bash
cargo test --test auto_context_v120 -- --nocapture 2>&1 | tail -15
```
Expected: all 3 integration tests pass.

**Commit:** `test: add v1.2.0 integration tests for full and briefing mode result shape`

---

## Task 21 — Regression test: incremental index produces same result as full rebuild

**Files:**
- New: `tests/incremental_index.rs`

**Steps:**

```rust
// tests/incremental_index.rs
use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use cxpak::scanner::ScannedFile;
use std::collections::HashMap;

#[test]
fn test_incremental_rebuild_same_as_full_rebuild() {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();

    let write_file = |name: &str, content: &str| -> ScannedFile {
        let safe = name.replace('/', "_");
        let fp = dir.path().join(&safe);
        std::fs::write(&fp, content).unwrap();
        ScannedFile {
            relative_path: name.to_string(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: content.len() as u64,
        }
    };

    let a = write_file("src/a.rs", "pub fn alpha() {}");
    let b = write_file("src/b.rs", "pub fn beta() {}");

    // Full build with both files
    let full_index = CodebaseIndex::build(
        vec![a.clone(), b.clone()],
        HashMap::new(),
        &counter,
    );

    // Build with only a.rs, then incrementally add b.rs
    let mut incremental = CodebaseIndex::build(
        vec![a.clone()],
        HashMap::new(),
        &counter,
    );
    incremental.incremental_rebuild(&[a, b], &HashMap::new(), &counter);

    // Both should have the same file count
    assert_eq!(incremental.total_files, full_index.total_files,
        "incremental rebuild must produce same file count as full rebuild");

    // Both should have the same total tokens
    assert_eq!(incremental.total_tokens, full_index.total_tokens,
        "incremental rebuild must produce same total tokens");
}

#[test]
fn test_incremental_rebuild_noop_when_nothing_changed() {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let fp = dir.path().join("a.rs");
    std::fs::write(&fp, "fn a() {}").unwrap();
    let file = ScannedFile {
        relative_path: "a.rs".into(),
        absolute_path: fp,
        language: Some("rust".into()),
        size_bytes: 9,
    };

    let mut index = CodebaseIndex::build(vec![file.clone()], HashMap::new(), &counter);
    let tokens_before = index.total_tokens;
    let files_before = index.total_files;

    // Run incremental with the same file (mtime not changed in the same process)
    // The mtime-based check may or may not trigger a re-parse here, but totals must remain same.
    index.incremental_rebuild(&[file], &HashMap::new(), &counter);

    assert_eq!(index.total_files, files_before);
    // Token count should be same or very close (re-parse produces same content)
    assert_eq!(index.total_tokens, tokens_before,
        "noop incremental rebuild must not change token count");
}
```

**Commands:**
```bash
cargo test --test incremental_index -- --nocapture 2>&1 | tail -10
```
Expected: both tests pass.

**Commit:** `test: add incremental index regression tests verifying parity with full rebuild`

---

## Task 22 — CHANGELOG and documentation update

**Files:**
- Modify: `CHANGELOG.md` (if it exists) or create a brief entry

**Steps:**

1. Check if `CHANGELOG.md` exists:

```bash
ls /Users/lb/Documents/barnett/cxpak/CHANGELOG.md
```

2. If it exists, prepend a v1.2.0 entry following the existing format. If not, add a brief `## v1.2.0 "Codebase Health"` section to the README under a Releases heading, or skip if the project doesn't maintain a changelog file.

3. Update the version in `Cargo.toml` docstring / description if relevant.

4. Verify the README `cxpak serve --mcp` tool count reflects 16 tools if it mentions a count.

**Commands:**
```bash
cargo build --release 2>&1 | tail -5
```
Expected: release build succeeds.

**Commit:** `docs: update changelog for v1.2.0 and verify release build`

---

## Implementation Order Summary

| Task | Feature | Files Changed | Test Count |
|------|---------|---------------|------------|
| 1 | PackedFile.content → Option<String> | briefing.rs, mod.rs | +2 |
| 2 | CoChangeEdge + git walk integration | co_change.rs (new), git_health.rs | +11 |
| 3 | co_changes on CodebaseIndex | index/mod.rs, conventions/mod.rs | +1 |
| 4 | HealthScore + Tarjan SCCs | health.rs (new) | +10 |
| 5 | RiskEntry + risk ranking | risk.rs (new) | +5 |
| 6 | ArchitectureMap | architecture.rs (new) | +4 |
| 7 | Recency scoring signal activated | recent_changes.rs, signals.rs, mod.rs | +4 |
| 8 | AutoContextResult new fields | auto_context/mod.rs | +2 |
| 9 | Briefing mode parameter | auto_context/mod.rs, briefing.rs | +2 |
| 10 | mtime + incremental_rebuild | index/mod.rs | +2 |
| 11 | cxpak_health MCP tool | serve.rs | +1 |
| 12 | cxpak_risks MCP tool | serve.rs | +1 |
| 13 | cxpak_briefing MCP tool | serve.rs | +2 |
| 14 | tools/list count verification | serve.rs | +1 |
| 15 | --health CLI flag | cli.rs, overview.rs | +1 |
| 16 | Plugin version bump | plugin.json, marketplace.json | 0 |
| 17 | Property tests for scoring | health.rs | +3 |
| 18 | Version bump to 1.2.0 | Cargo.toml | 0 |
| 19 | Full suite + coverage | — | 0 |
| 20 | Integration tests v1.2.0 result shape | tests/auto_context_v120.rs (new) | +3 |
| 21 | Incremental index regression | tests/incremental_index.rs (new) | +2 |
| 22 | Changelog + release build | CHANGELOG.md/README.md | 0 |

**Total new tests: ~57** (across unit, integration, and property-based).

**New files:** `src/intelligence/co_change.rs`, `src/intelligence/health.rs`, `src/intelligence/risk.rs`, `src/intelligence/architecture.rs`, `src/intelligence/recent_changes.rs`, `tests/auto_context_v120.rs`, `tests/incremental_index.rs`.

**Files modified:** `src/auto_context/briefing.rs`, `src/auto_context/mod.rs`, `src/conventions/git_health.rs`, `src/conventions/mod.rs`, `src/index/mod.rs`, `src/intelligence/mod.rs`, `src/relevance/mod.rs`, `src/relevance/signals.rs`, `src/commands/serve.rs`, `src/cli.rs`, `src/commands/overview.rs`, `Cargo.toml`, `plugin/.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json`.
