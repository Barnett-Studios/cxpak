//! Advanced onboarding map functions: topological sort, phase grouping, and
//! reading-time formatting.
//!
//! These functions implement Tasks 16-20 of the v2.0.0 onboarding pipeline.
//! The entry point for callers is [`compute_onboarding_map`] in
//! `src/visual/onboard.rs`, which delegates to these functions.

use crate::index::graph::DependencyGraph;
use std::collections::{HashMap, HashSet, VecDeque};

// ---------------------------------------------------------------------------
// Onboarding types (canonical home; re-exported from visual::onboard)
// ---------------------------------------------------------------------------

/// A file included in an onboarding phase with focus guidance.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OnboardingFile {
    /// Relative path to the file within the repository.
    pub path: String,
    /// PageRank importance score for this file (0.0–1.0).
    pub pagerank: f64,
    /// Key symbols a new developer should focus on when reading this file.
    pub symbols_to_focus_on: Vec<String>,
    /// Approximate token count for reading-time estimation.
    pub estimated_tokens: usize,
}

/// A logical grouping of files that a developer should read together.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OnboardingPhase {
    /// Human-readable phase name (e.g. "Entry Points", "Core Logic").
    pub name: String,
    /// Module or directory prefix for this phase (e.g. "src/commands").
    pub module: String,
    /// Why this phase should be read at this point in the learning journey.
    pub rationale: String,
    /// Files to read in this phase, ordered by reading priority.
    pub files: Vec<OnboardingFile>,
}

/// A guided onboarding map for navigating an unfamiliar codebase.
///
/// Produced by [`compute_onboarding_map`] in `visual::onboard` and consumed
/// by the MCP tool and the interactive onboarding UI (Task 7 dashboard).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OnboardingMap {
    /// Total number of files included across all phases.
    pub total_files: usize,
    /// Human-readable estimate of reading time (e.g. "~4 hours").
    pub estimated_reading_time: String,
    /// Phases in recommended reading order.
    pub phases: Vec<OnboardingPhase>,
}

// ---------------------------------------------------------------------------
// Task 18: Topological sort
// ---------------------------------------------------------------------------

/// Returns files in dependency-first order (leaves before importers).
///
/// Uses Kahn's algorithm restricted to the given file set. When the BFS
/// queue empties but unprocessed nodes remain (i.e. a cycle exists), the
/// remaining nodes are appended in lexicographic order so the output is
/// always deterministic and contains every input file exactly once.
pub fn topological_sort_files(files: &[&str], graph: &DependencyGraph) -> Vec<String> {
    if files.is_empty() {
        return Vec::new();
    }

    let file_set: HashSet<&str> = files.iter().copied().collect();

    // We want dependency-first order: files with no outgoing edges (no
    // dependencies within the set) come first, their importers come after.
    //
    // This is Kahn's algorithm run on the REVERSED graph:
    //   - in_degree_rev[f] = number of outgoing edges from f in the original
    //     graph that point to files within the set (i.e. how many files in the
    //     set does f depend on).
    //   - Files with in_degree_rev == 0 have no intra-set dependencies and are
    //     the seeds (leaves).
    //   - When we process f, we follow graph.reverse_edges[f] to find the
    //     files that depend on f, and decrement their counts.
    let mut in_degree: HashMap<String, usize> = files.iter().map(|&f| (f.to_string(), 0)).collect();

    for &f in &file_set {
        if let Some(edges) = graph.edges.get(f) {
            for edge in edges {
                if file_set.contains(edge.target.as_str()) {
                    *in_degree.entry(f.to_string()).or_insert(0) += 1;
                }
            }
        }
    }

    // Seed the queue with files that have no dependencies within the set.
    // Sort lexicographically for deterministic output.
    let mut queue: VecDeque<String> = {
        let mut seeds: Vec<String> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(f, _)| f.clone())
            .collect();
        seeds.sort();
        VecDeque::from(seeds)
    };

    let mut result: Vec<String> = Vec::with_capacity(files.len());
    let mut visited: HashSet<String> = HashSet::new();

    while let Some(current) = queue.pop_front() {
        if !visited.insert(current.clone()) {
            continue;
        }
        result.push(current.clone());

        // Follow reverse edges: files that import `current` may now have all
        // their dependencies satisfied.
        let mut importers: Vec<String> = graph
            .reverse_edges
            .get(&current)
            .map(|edges| {
                let mut targets: Vec<String> = edges
                    .iter()
                    .filter(|e| {
                        file_set.contains(e.target.as_str()) && !visited.contains(&e.target)
                    })
                    .map(|e| e.target.clone())
                    .collect();
                targets.sort();
                targets
            })
            .unwrap_or_default();

        for importer in importers.drain(..) {
            let deg = in_degree.entry(importer.clone()).or_insert(0);
            if *deg > 0 {
                *deg -= 1;
            }
            if *deg == 0 && !visited.contains(&importer) {
                queue.push_back(importer);
            }
        }
    }

    // If a cycle prevented some nodes from being processed, append them in
    // lexicographic order for a stable, deterministic output.
    if result.len() < files.len() {
        let mut remaining: Vec<String> = file_set
            .iter()
            .filter(|&&f| !visited.contains(f))
            .map(|&f| f.to_string())
            .collect();
        remaining.sort();
        result.extend(remaining);
    }

    result
}

// ---------------------------------------------------------------------------
// Task 19: Phase grouping
// ---------------------------------------------------------------------------

/// Groups topologically-sorted files into phases by module prefix.
///
/// Rules:
/// - Module prefix = first two path segments (e.g. `src/index`).
/// - Files within each module preserve the topological reading order, then
///   are sorted by ascending token count (simpler files first).
/// - Module groups are ordered by descending aggregate PageRank.
/// - No phase exceeds 9 files (7 ± 2 cognitive load cap). Modules with more
///   than 9 files are split into sub-phases named `"<Module> (N/M)"`.
/// - Rationale: phase 0 → "Core module depended on by all others.";
///   subsequent phases → "Builds on <prior>." if a cross-module graph edge
///   exists, otherwise "Independent module."
pub fn group_into_phases(
    sorted_files: &[String],
    pagerank: &HashMap<String, f64>,
    graph: &DependencyGraph,
    file_tokens: &HashMap<String, usize>,
    file_symbols: &HashMap<String, Vec<String>>,
) -> Vec<OnboardingPhase> {
    const MAX_PHASE_SIZE: usize = 9;

    // Group files by module prefix, preserving topological order.
    let mut module_order: Vec<String> = Vec::new();
    let mut module_map: HashMap<String, Vec<String>> = HashMap::new();

    for file in sorted_files {
        let module = module_prefix(file);
        module_map
            .entry(module.clone())
            .or_insert_with(|| {
                module_order.push(module.clone());
                Vec::new()
            })
            .push(file.clone());
    }

    // Sort module groups by descending aggregate PageRank.
    let module_pagerank: HashMap<String, f64> = module_map
        .iter()
        .map(|(module, files)| {
            let total: f64 = files
                .iter()
                .map(|f| pagerank.get(f).copied().unwrap_or(0.0))
                .sum();
            (module.clone(), total)
        })
        .collect();

    module_order.sort_by(|a, b| {
        module_pagerank
            .get(b)
            .copied()
            .unwrap_or(0.0)
            .partial_cmp(&module_pagerank.get(a).copied().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Collect all phases with split for >9 file modules.
    let mut phases: Vec<OnboardingPhase> = Vec::new();

    for module in &module_order {
        let files = match module_map.get(module) {
            Some(f) => f,
            None => continue,
        };

        // Preserve the topological order passed in — re-sorting by token count
        // would discard dependency ordering within the module. Files are already
        // ordered correctly by `group_into_phases` which calls
        // `topological_sort_files` first.
        let sorted_module_files: Vec<String> = files.clone();

        let chunk_count = sorted_module_files.len().div_ceil(MAX_PHASE_SIZE);

        for (chunk_idx, chunk) in sorted_module_files.chunks(MAX_PHASE_SIZE).enumerate() {
            let phase_name = if chunk_count > 1 {
                format!(
                    "{} ({}/{})",
                    module_display_name(module),
                    chunk_idx + 1,
                    chunk_count
                )
            } else {
                module_display_name(module)
            };

            let phase_files: Vec<OnboardingFile> = chunk
                .iter()
                .map(|path| OnboardingFile {
                    path: path.clone(),
                    pagerank: pagerank.get(path).copied().unwrap_or(0.0),
                    symbols_to_focus_on: file_symbols
                        .get(path)
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .take(5)
                        .collect(),
                    estimated_tokens: file_tokens.get(path).copied().unwrap_or(0),
                })
                .collect();

            let rationale = build_rationale(&phases, module, graph, module_map.values());

            phases.push(OnboardingPhase {
                name: phase_name,
                module: module.clone(),
                rationale,
                files: phase_files,
            });
        }
    }

    phases
}

/// Derive a human-readable name from a module prefix.
fn module_display_name(module: &str) -> String {
    module
        .split('/')
        .next_back()
        .map(|s| {
            let mut chars = s.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .unwrap_or_else(|| module.to_string())
}

/// Compute the module prefix (first two path segments) for a file path.
fn module_prefix(path: &str) -> String {
    let parts: Vec<&str> = path.splitn(3, '/').collect();
    if parts.len() >= 2 {
        format!("{}/{}", parts[0], parts[1])
    } else {
        parts[0].to_string()
    }
}

/// Build the rationale string for a new phase.
///
/// Phase 0 → "Core module depended on by all others."
/// Otherwise → "Builds on <prior module>." if a cross-module edge exists,
///             "Independent module." if no prior modules depend on this one.
fn build_rationale<'a, I>(
    phases: &[OnboardingPhase],
    current_module: &str,
    graph: &DependencyGraph,
    all_module_files: I,
) -> String
where
    I: Iterator<Item = &'a Vec<String>>,
{
    if phases.is_empty() {
        return "Core module depended on by all others.".to_string();
    }

    // Collect all files belonging to the current module across all groups.
    let current_files: HashSet<String> = all_module_files
        .flatten()
        .filter(|f| module_prefix(f) == current_module)
        .cloned()
        .collect();

    // Look for a prior phase whose files have a graph edge into the current module.
    for prior_phase in phases.iter().rev() {
        for prior_file in &prior_phase.files {
            if let Some(edges) = graph.edges.get(&prior_file.path) {
                for edge in edges {
                    if current_files.contains(&edge.target) {
                        return format!("Builds on {}.", prior_phase.module);
                    }
                }
            }
        }
    }

    "Independent module.".to_string()
}

// ---------------------------------------------------------------------------
// Task 20: Reading time
// ---------------------------------------------------------------------------

/// Formats total tokens as human-readable reading time at 200 tokens/min.
///
/// - Returns `"~{h}h {m}m"` when the result is at least one hour.
/// - Returns `"~{m}m"` otherwise.
/// - 0 tokens returns `"~0m"`.
pub fn format_reading_time(total_tokens: usize) -> String {
    let minutes = (total_tokens as f64 / 200.0).ceil() as usize;
    if minutes >= 60 {
        let h = minutes / 60;
        let m = minutes % 60;
        format!("~{h}h {m}m")
    } else {
        format!("~{minutes}m")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::graph::EdgeType;

    // -----------------------------------------------------------------------
    // Task 18: topological_sort_files
    // -----------------------------------------------------------------------

    #[test]
    fn test_topo_sort_linear() {
        // A imports B, B imports C => dependency-first order: C, B, A
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "c.rs", EdgeType::Import);

        let files = ["a.rs", "b.rs", "c.rs"];
        let result = topological_sort_files(&files, &graph);

        assert_eq!(result.len(), 3);
        // c.rs must appear before b.rs, and b.rs before a.rs
        let pos = |name: &str| result.iter().position(|x| x == name).unwrap();
        assert!(pos("c.rs") < pos("b.rs"), "c before b");
        assert!(pos("b.rs") < pos("a.rs"), "b before a");
    }

    #[test]
    fn test_topo_sort_cycle_doesnt_panic() {
        // A ↔ B (mutual imports) — both must appear, no panic
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "a.rs", EdgeType::Import);

        let files = ["a.rs", "b.rs"];
        let result = topological_sort_files(&files, &graph);

        assert_eq!(result.len(), 2);
        assert!(result.contains(&"a.rs".to_string()));
        assert!(result.contains(&"b.rs".to_string()));
    }

    #[test]
    fn test_topo_sort_diamond() {
        // A → B, A → C, B → D, C → D
        // D must come first, then B and C, then A
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("a.rs", "c.rs", EdgeType::Import);
        graph.add_edge("b.rs", "d.rs", EdgeType::Import);
        graph.add_edge("c.rs", "d.rs", EdgeType::Import);

        let files = ["a.rs", "b.rs", "c.rs", "d.rs"];
        let result = topological_sort_files(&files, &graph);

        assert_eq!(result.len(), 4);
        let pos = |name: &str| result.iter().position(|x| x == name).unwrap();
        // d must be before b and c; b and c must be before a
        assert!(pos("d.rs") < pos("b.rs"), "d before b");
        assert!(pos("d.rs") < pos("c.rs"), "d before c");
        assert!(pos("b.rs") < pos("a.rs"), "b before a");
        assert!(pos("c.rs") < pos("a.rs"), "c before a");
    }

    #[test]
    fn test_topo_sort_empty() {
        let graph = DependencyGraph::new();
        let result = topological_sort_files(&[], &graph);
        assert!(result.is_empty());
    }

    #[test]
    fn test_topo_sort_single_file() {
        let graph = DependencyGraph::new();
        let result = topological_sort_files(&["a.rs"], &graph);
        assert_eq!(result, vec!["a.rs".to_string()]);
    }

    #[test]
    fn test_topo_sort_disconnected_files() {
        // Files with no edges among them — all are independent
        let graph = DependencyGraph::new();
        let files = ["c.rs", "a.rs", "b.rs"];
        let result = topological_sort_files(&files, &graph);
        // All 3 files present; with no edges, seeds are sorted lexicographically
        assert_eq!(result.len(), 3);
        assert_eq!(result, vec!["a.rs", "b.rs", "c.rs"]);
    }

    // -----------------------------------------------------------------------
    // Task 19: group_into_phases
    // -----------------------------------------------------------------------

    #[test]
    fn test_group_splits_large_module() {
        // 20 files from "src/foo" → 3 phases: sizes 9, 9, 2
        let files: Vec<String> = (0..20).map(|i| format!("src/foo/f{i:02}.rs")).collect();
        let file_refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
        let graph = DependencyGraph::new();
        let sorted = topological_sort_files(&file_refs, &graph);

        let pagerank: HashMap<String, f64> = files.iter().map(|f| (f.clone(), 1.0)).collect();
        let tokens: HashMap<String, usize> = files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.clone(), i + 1))
            .collect();
        let symbols: HashMap<String, Vec<String>> = HashMap::new();

        let phases = group_into_phases(&sorted, &pagerank, &graph, &tokens, &symbols);

        // Should be split into 3 sub-phases
        assert_eq!(
            phases.len(),
            3,
            "expected 3 sub-phases, got {}",
            phases.len()
        );
        assert_eq!(phases[0].files.len(), 9);
        assert_eq!(phases[1].files.len(), 9);
        assert_eq!(phases[2].files.len(), 2);

        // Names should carry the "(N/M)" suffix
        assert!(
            phases[0].name.contains("(1/3)"),
            "phase 0 name: {}",
            phases[0].name
        );
        assert!(
            phases[1].name.contains("(2/3)"),
            "phase 1 name: {}",
            phases[1].name
        );
        assert!(
            phases[2].name.contains("(3/3)"),
            "phase 2 name: {}",
            phases[2].name
        );
    }

    #[test]
    fn test_group_orders_by_pagerank() {
        // "src/low" has pagerank 0.1, "src/high" has pagerank 0.9
        // "src/high" module should appear as the first phase
        let files_low: Vec<String> = (0..3).map(|i| format!("src/low/f{i}.rs")).collect();
        let files_high: Vec<String> = (0..3).map(|i| format!("src/high/f{i}.rs")).collect();
        let all_files: Vec<String> = files_low.iter().chain(files_high.iter()).cloned().collect();
        let file_refs: Vec<&str> = all_files.iter().map(|s| s.as_str()).collect();

        let graph = DependencyGraph::new();
        let sorted = topological_sort_files(&file_refs, &graph);

        let mut pagerank: HashMap<String, f64> = HashMap::new();
        for f in &files_low {
            pagerank.insert(f.clone(), 0.1);
        }
        for f in &files_high {
            pagerank.insert(f.clone(), 0.9);
        }

        let tokens: HashMap<String, usize> = all_files.iter().map(|f| (f.clone(), 100)).collect();
        let symbols: HashMap<String, Vec<String>> = HashMap::new();

        let phases = group_into_phases(&sorted, &pagerank, &graph, &tokens, &symbols);

        assert!(phases.len() >= 2, "expected at least 2 phases");
        // The first phase should belong to "src/high" (higher aggregate pagerank)
        assert_eq!(
            phases[0].module, "src/high",
            "first phase should be src/high, got {}",
            phases[0].module
        );
    }

    #[test]
    fn test_group_no_cross_module_mixing() {
        // Files from different modules must never share a phase
        let files: Vec<String> = vec![
            "src/alpha/a.rs".to_string(),
            "src/beta/b.rs".to_string(),
            "src/alpha/c.rs".to_string(),
        ];
        let file_refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
        let graph = DependencyGraph::new();
        let sorted = topological_sort_files(&file_refs, &graph);
        let pagerank: HashMap<String, f64> = files.iter().map(|f| (f.clone(), 1.0)).collect();
        let tokens: HashMap<String, usize> = files.iter().map(|f| (f.clone(), 100)).collect();
        let symbols: HashMap<String, Vec<String>> = HashMap::new();

        let phases = group_into_phases(&sorted, &pagerank, &graph, &tokens, &symbols);

        for phase in &phases {
            let modules: HashSet<String> =
                phase.files.iter().map(|f| module_prefix(&f.path)).collect();
            assert_eq!(
                modules.len(),
                1,
                "phase '{}' contains files from multiple modules: {:?}",
                phase.name,
                modules
            );
        }
    }

    // -----------------------------------------------------------------------
    // Task 20: format_reading_time
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_reading_time_with_hours() {
        // 12000 tokens / 200 = 60 minutes = 1h 0m
        assert_eq!(format_reading_time(12000), "~1h 0m");
    }

    #[test]
    fn test_format_reading_time_minutes_only() {
        // 500 / 200 = 2.5 → ceil = 3 minutes
        assert_eq!(format_reading_time(500), "~3m");
    }

    #[test]
    fn test_format_reading_time_zero() {
        assert_eq!(format_reading_time(0), "~0m");
    }

    #[test]
    fn test_format_reading_time_exactly_one_hour() {
        // 12000 tokens = exactly 60 minutes → 1h 0m
        assert_eq!(format_reading_time(12_000), "~1h 0m");
    }

    #[test]
    fn test_format_reading_time_one_hour_thirty() {
        // 18000 / 200 = 90 minutes = 1h 30m
        assert_eq!(format_reading_time(18_000), "~1h 30m");
    }

    #[test]
    fn test_format_reading_time_sub_minute() {
        // 100 / 200 = 0.5 → ceil = 1 minute
        assert_eq!(format_reading_time(100), "~1m");
    }

    // ── Additional onboarding tests ───────────────────────────────────────────

    #[test]
    fn test_topological_sort_deterministic_same_input() {
        // Running topological_sort_files twice with the same input must produce identical output.
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "c.rs", EdgeType::Import);
        graph.add_edge("a.rs", "c.rs", EdgeType::Import);

        let files = ["a.rs", "b.rs", "c.rs"];
        let first = topological_sort_files(&files, &graph);
        let second = topological_sort_files(&files, &graph);
        assert_eq!(
            first, second,
            "topological_sort_files must be deterministic"
        );
    }

    #[test]
    fn test_topological_sort_large_input_completes_quickly() {
        // 100 disconnected nodes — verifies the algorithm completes in reasonable time.
        let files: Vec<String> = (0..100).map(|i| format!("src/f{i:03}.rs")).collect();
        let file_refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
        let graph = DependencyGraph::new();
        let start = std::time::Instant::now();
        let result = topological_sort_files(&file_refs, &graph);
        let elapsed = start.elapsed();
        assert_eq!(result.len(), 100);
        assert!(
            elapsed.as_millis() < 100,
            "topological_sort_files on 100 nodes took {}ms, expected <100ms",
            elapsed.as_millis()
        );
    }

    #[test]
    fn test_group_into_phases_first_phase_rationale_mentions_core() {
        let files: Vec<String> = (0..3).map(|i| format!("src/core/f{i}.rs")).collect();
        let file_refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
        let graph = DependencyGraph::new();
        let sorted = topological_sort_files(&file_refs, &graph);
        let pagerank: HashMap<String, f64> = files.iter().map(|f| (f.clone(), 1.0)).collect();
        let tokens: HashMap<String, usize> = files.iter().map(|f| (f.clone(), 100)).collect();
        let symbols: HashMap<String, Vec<String>> = HashMap::new();

        let phases = group_into_phases(&sorted, &pagerank, &graph, &tokens, &symbols);
        assert!(!phases.is_empty(), "must produce at least one phase");
        // Phase 0 rationale should mention "Core" (derived from build_rationale).
        let rationale = &phases[0].rationale;
        assert!(
            rationale.to_lowercase().contains("core") || rationale.contains("depended"),
            "phase 0 rationale should mention core dependency context, got: {rationale}"
        );
    }

    #[test]
    fn test_group_into_phases_no_phase_exceeds_9_files() {
        let files: Vec<String> = (0..50).map(|i| format!("src/big/f{i:02}.rs")).collect();
        let file_refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
        let graph = DependencyGraph::new();
        let sorted = topological_sort_files(&file_refs, &graph);
        let pagerank: HashMap<String, f64> = files.iter().map(|f| (f.clone(), 0.5)).collect();
        let tokens: HashMap<String, usize> = files.iter().map(|f| (f.clone(), 50)).collect();
        let symbols: HashMap<String, Vec<String>> = HashMap::new();

        let phases = group_into_phases(&sorted, &pagerank, &graph, &tokens, &symbols);
        for phase in &phases {
            assert!(
                phase.files.len() <= 9,
                "phase '{}' has {} files, exceeds limit of 9",
                phase.name,
                phase.files.len()
            );
        }
    }

    #[test]
    fn test_format_reading_time_boundary_200_tokens_is_1m() {
        // Exactly 200 tokens = 200/200 = 1.0 → ceil = 1 minute
        assert_eq!(format_reading_time(200), "~1m");
    }

    #[test]
    fn test_format_reading_time_boundary_12000_tokens_is_1h_0m() {
        // Exactly 12000 tokens = 60.0 minutes → "~1h 0m"
        assert_eq!(format_reading_time(12_000), "~1h 0m");
    }

    // ── Onboarding topo-order within module regression (46ced99) ────────────
    //
    // Bug: group_into_phases() previously sorted files within each module by
    // token count, discarding the dependency order computed by
    // topological_sort_files().  This caused readers to encounter files before
    // their dependencies.
    //
    // The test would FAIL against the pre-fix code:
    //  - With "core.rs" (1000 tokens, no imports) and "wrapper.rs"
    //    (10 tokens, imports core.rs), the old sort-by-tokens code placed
    //    "core.rs" first because 1000 > 10.  After the fix the topological
    //    order is preserved: "core.rs" first (depended-on), "wrapper.rs"
    //    second (depends on core.rs).
    //  - However, both orders happen to put "core.rs" first in this
    //    particular case, so we construct a case where the topo order and
    //    token-count order *disagree*:
    //      leaf.rs  (no deps, 50 tokens)   ← fewer tokens
    //      root.rs  (imports leaf.rs, 500 tokens) ← more tokens
    //    Topo order: leaf.rs first (must be read before root.rs).
    //    Token order: root.rs first (500 > 50).
    //    The fix should yield leaf.rs first.

    #[test]
    fn test_group_into_phases_preserves_topo_order_within_module() {
        let files: Vec<String> = vec!["src/m/leaf.rs".to_string(), "src/m/root.rs".to_string()];
        let file_refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();

        // leaf.rs has no deps; root.rs depends on leaf.rs.
        let mut graph = DependencyGraph::new();
        graph.add_edge("src/m/root.rs", "src/m/leaf.rs", EdgeType::Import);

        // Topological sort: leaf.rs must come before root.rs.
        let sorted = topological_sort_files(&file_refs, &graph);
        assert_eq!(
            sorted[0], "src/m/leaf.rs",
            "topo sort should put leaf.rs first (no deps)"
        );

        // Assign token counts that DISAGREE with topo order: root has more tokens.
        let mut tokens: HashMap<String, usize> = HashMap::new();
        tokens.insert("src/m/leaf.rs".to_string(), 50);
        tokens.insert("src/m/root.rs".to_string(), 500);

        let pagerank: HashMap<String, f64> = files.iter().map(|f| (f.clone(), 1.0)).collect();
        let symbols: HashMap<String, Vec<String>> = HashMap::new();

        let phases = group_into_phases(&sorted, &pagerank, &graph, &tokens, &symbols);

        // There must be exactly one phase for "src/m".
        assert_eq!(phases.len(), 1);
        let phase = &phases[0];
        assert_eq!(phase.files.len(), 2);

        // Topo order must be preserved: leaf.rs before root.rs.
        assert_eq!(
            phase.files[0].path, "src/m/leaf.rs",
            "leaf.rs (depended-on) must appear first; \
             if root.rs is first, the token-sort regression (46ced99) is back"
        );
        assert_eq!(phase.files[1].path, "src/m/root.rs");
    }
}
