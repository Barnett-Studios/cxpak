use super::ScoredFile;
use crate::index::graph::build_dependency_graph;
use crate::index::CodebaseIndex;
use std::collections::HashMap;

/// Default score threshold for seed selection.
pub const SEED_THRESHOLD: f64 = 0.3;

/// Discount factor for dependency fan-out scores.
pub const FANOUT_DISCOUNT: f64 = 0.7;

/// Select seed files above threshold, then fan out to 1-hop dependency neighbors.
///
/// Algorithm:
/// 1. Filter scored files above threshold -> seeds
/// 2. Build dependency graph via `build_dependency_graph(index)`
/// 3. For each seed, look up 1-hop neighbors in the graph (both directions)
/// 4. For each neighbor not already a seed, add with score = seed_score * FANOUT_DISCOUNT
/// 5. If a neighbor was added by multiple seeds, keep the highest score
/// 6. Sort all by score descending
/// 7. Truncate to limit
pub fn select_seeds(
    scored: &[ScoredFile],
    index: &CodebaseIndex,
    threshold: f64,
    limit: usize,
) -> Vec<ScoredFile> {
    // Step 1: Filter above threshold
    let mut result_map: HashMap<String, ScoredFile> = HashMap::new();

    for sf in scored {
        if sf.score >= threshold {
            result_map.insert(sf.path.clone(), sf.clone());
        }
    }

    if result_map.is_empty() {
        return Vec::new();
    }

    // Step 2: Build dependency graph
    let graph = build_dependency_graph(index);

    // Step 3-5: Fan out to 1-hop neighbors
    let seed_paths: Vec<String> = result_map.keys().cloned().collect();
    for seed_path in &seed_paths {
        let seed_score = result_map[seed_path].score;
        let fanout_score = seed_score * FANOUT_DISCOUNT;

        // Look up 1-hop neighbors in both directions
        let mut neighbors: Vec<String> = Vec::new();

        // Outgoing dependencies (files this seed depends on)
        if let Some(deps) = graph.dependencies(seed_path) {
            neighbors.extend(deps.iter().cloned());
        }

        // Incoming dependents (files that depend on this seed)
        for dep in graph.dependents(seed_path) {
            neighbors.push(dep.to_string());
        }

        for neighbor in neighbors {
            // Only add if neighbor is in the index
            let neighbor_in_index = index.files.iter().any(|f| f.relative_path == neighbor);
            if !neighbor_in_index {
                continue;
            }

            match result_map.get(&neighbor) {
                Some(existing) if existing.score >= fanout_score => {
                    // Already has a higher or equal score, skip
                }
                _ => {
                    // Find token count for the neighbor
                    let token_count = index
                        .files
                        .iter()
                        .find(|f| f.relative_path == neighbor)
                        .map(|f| f.token_count)
                        .unwrap_or(0);

                    result_map.insert(
                        neighbor.clone(),
                        ScoredFile {
                            path: neighbor,
                            score: fanout_score,
                            signals: vec![],
                            token_count,
                        },
                    );
                }
            }
        }
    }

    // Step 6: Sort by score descending
    let mut results: Vec<ScoredFile> = result_map.into_values().collect();
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Step 7: Truncate to limit
    results.truncate(limit);

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::parser::language::{Import, ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    /// Build a test index with four files and import relationships:
    ///   src/api.rs  --imports-->  src/middleware.rs  --imports-->  src/config.rs
    ///   src/utils.rs (isolated, no imports)
    ///
    /// Import sources use `src::middleware` / `src::config` format so that
    /// `build_dependency_graph` resolves them to `src/middleware.rs` and `src/config.rs`.
    fn make_seed_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let fp1 = dir.path().join("src/api.rs");
        let fp2 = dir.path().join("src/middleware.rs");
        let fp3 = dir.path().join("src/config.rs");
        let fp4 = dir.path().join("src/utils.rs");
        std::fs::write(&fp1, "use crate::middleware; fn api() {}").unwrap();
        std::fs::write(&fp2, "use crate::config; fn middleware() {}").unwrap();
        std::fs::write(&fp3, "fn config() {}").unwrap();
        std::fs::write(&fp4, "fn utils() {}").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "src/api.rs".into(),
                absolute_path: fp1,
                language: Some("rust".into()),
                size_bytes: 35,
            },
            ScannedFile {
                relative_path: "src/middleware.rs".into(),
                absolute_path: fp2,
                language: Some("rust".into()),
                size_bytes: 40,
            },
            ScannedFile {
                relative_path: "src/config.rs".into(),
                absolute_path: fp3,
                language: Some("rust".into()),
                size_bytes: 14,
            },
            ScannedFile {
                relative_path: "src/utils.rs".into(),
                absolute_path: fp4,
                language: Some("rust".into()),
                size_bytes: 14,
            },
        ];

        let mut pr = HashMap::new();
        pr.insert(
            "src/api.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "api".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn api()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![Import {
                    source: "src::middleware".into(),
                    names: vec!["middleware".into()],
                }],
                exports: vec![],
            },
        );
        pr.insert(
            "src/middleware.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "middleware".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn middleware()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![Import {
                    source: "src::config".into(),
                    names: vec!["config".into()],
                }],
                exports: vec![],
            },
        );

        CodebaseIndex::build(files, pr, &counter)
    }

    #[test]
    fn test_select_seeds_threshold_filtering() {
        let index = make_seed_index();
        let scored = vec![
            ScoredFile {
                path: "src/api.rs".into(),
                score: 0.8,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/middleware.rs".into(),
                score: 0.5,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/config.rs".into(),
                score: 0.2,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/utils.rs".into(),
                score: 0.1,
                signals: vec![],
                token_count: 10,
            },
        ];
        let seeds = select_seeds(&scored, &index, SEED_THRESHOLD, 100);
        let paths: Vec<&str> = seeds.iter().map(|s| s.path.as_str()).collect();
        assert!(paths.contains(&"src/api.rs"));
        assert!(paths.contains(&"src/middleware.rs"));
        // config.rs below threshold (0.2 < 0.3), but may appear as dependency fan-out
        assert!(!paths.contains(&"src/utils.rs")); // too low, not a dependency
    }

    #[test]
    fn test_select_seeds_fanout_discount() {
        let index = make_seed_index();
        let scored = vec![
            ScoredFile {
                path: "src/api.rs".into(),
                score: 0.8,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/middleware.rs".into(),
                score: 0.1,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/config.rs".into(),
                score: 0.1,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/utils.rs".into(),
                score: 0.1,
                signals: vec![],
                token_count: 10,
            },
        ];
        let seeds = select_seeds(&scored, &index, SEED_THRESHOLD, 100);
        // middleware.rs should be added via fan-out from api.rs (0.8 * 0.7 = 0.56)
        let middleware = seeds.iter().find(|s| s.path == "src/middleware.rs");
        assert!(
            middleware.is_some(),
            "middleware should be added via fan-out"
        );
        assert!(
            (middleware.unwrap().score - 0.56).abs() < 0.01,
            "fan-out score should be seed_score * 0.7 = 0.56, got {}",
            middleware.unwrap().score
        );
    }

    #[test]
    fn test_select_seeds_limit() {
        let index = make_seed_index();
        let scored = vec![
            ScoredFile {
                path: "src/api.rs".into(),
                score: 0.8,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/middleware.rs".into(),
                score: 0.7,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/config.rs".into(),
                score: 0.6,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/utils.rs".into(),
                score: 0.5,
                signals: vec![],
                token_count: 10,
            },
        ];
        let seeds = select_seeds(&scored, &index, SEED_THRESHOLD, 2);
        assert!(seeds.len() <= 2);
    }

    #[test]
    fn test_select_seeds_empty_results() {
        let index = make_seed_index();
        let scored: Vec<ScoredFile> = vec![];
        let seeds = select_seeds(&scored, &index, SEED_THRESHOLD, 100);
        assert!(seeds.is_empty());
    }

    #[test]
    fn test_select_seeds_all_below_threshold() {
        let index = make_seed_index();
        let scored = vec![
            ScoredFile {
                path: "src/api.rs".into(),
                score: 0.1,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/utils.rs".into(),
                score: 0.05,
                signals: vec![],
                token_count: 10,
            },
        ];
        let seeds = select_seeds(&scored, &index, SEED_THRESHOLD, 100);
        assert!(seeds.is_empty());
    }

    #[test]
    fn test_select_seeds_sorted_by_score() {
        let index = make_seed_index();
        let scored = vec![
            ScoredFile {
                path: "src/api.rs".into(),
                score: 0.5,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/middleware.rs".into(),
                score: 0.8,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/config.rs".into(),
                score: 0.6,
                signals: vec![],
                token_count: 10,
            },
        ];
        let seeds = select_seeds(&scored, &index, SEED_THRESHOLD, 100);
        for i in 1..seeds.len() {
            assert!(
                seeds[i - 1].score >= seeds[i].score,
                "results should be sorted descending"
            );
        }
    }

    #[test]
    fn test_select_seeds_fanout_keeps_higher_score() {
        let index = make_seed_index();
        // middleware.rs is already above threshold with score 0.9
        // api.rs fan-out would give it 0.8 * 0.7 = 0.56 which is lower
        // Should keep 0.9
        let scored = vec![
            ScoredFile {
                path: "src/api.rs".into(),
                score: 0.8,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/middleware.rs".into(),
                score: 0.9,
                signals: vec![],
                token_count: 10,
            },
        ];
        let seeds = select_seeds(&scored, &index, SEED_THRESHOLD, 100);
        let middleware = seeds
            .iter()
            .find(|s| s.path == "src/middleware.rs")
            .unwrap();
        assert!(
            (middleware.score - 0.9).abs() < 0.01,
            "should keep original higher score 0.9, got {}",
            middleware.score
        );
    }

    #[test]
    fn test_select_seeds_reverse_dependency_fanout() {
        let index = make_seed_index();
        // Only middleware.rs is above threshold. api.rs imports middleware.rs, so
        // api.rs should appear as a reverse-dependency fan-out neighbor.
        let scored = vec![
            ScoredFile {
                path: "src/api.rs".into(),
                score: 0.1,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/middleware.rs".into(),
                score: 0.8,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/config.rs".into(),
                score: 0.1,
                signals: vec![],
                token_count: 10,
            },
            ScoredFile {
                path: "src/utils.rs".into(),
                score: 0.1,
                signals: vec![],
                token_count: 10,
            },
        ];
        let seeds = select_seeds(&scored, &index, SEED_THRESHOLD, 100);
        let paths: Vec<&str> = seeds.iter().map(|s| s.path.as_str()).collect();
        // api.rs depends on middleware.rs, so it's a reverse-dependency neighbor
        assert!(
            paths.contains(&"src/api.rs"),
            "api.rs should be added via reverse-dependency fan-out from middleware.rs"
        );
        // config.rs is a forward dependency of middleware.rs
        assert!(
            paths.contains(&"src/config.rs"),
            "config.rs should be added via forward-dependency fan-out from middleware.rs"
        );
    }
}
