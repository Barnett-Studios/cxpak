# v0.13.0 Implementation Plan: Intelligence

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add structural intelligence to cxpak — PageRank symbol importance, blast radius analysis, API surface extraction, and test file mapping — all deterministic, all computed from the existing typed dependency graph.

**Architecture:** New `src/intelligence/` module with four files. The biggest prerequisite change is caching `DependencyGraph` on `CodebaseIndex` (currently built on-demand at each call site). Once cached, PageRank and test mapping are computed at build time. Two new MCP tools (blast_radius, api_surface) bring the total to 9. PageRank and test mapping enhance existing tools internally.

**Tech Stack:** Rust, existing `DependencyGraph` with `TypedEdge` (v0.12.0), `regex` crate (for route detection)

**Spec:** `docs/superpowers/specs/2026-03-22-v0130-design.md`

---

## File Structure

### New Files
- `src/intelligence/mod.rs` — public API, re-exports
- `src/intelligence/pagerank.rs` — `compute_pagerank()`, `build_symbol_cross_refs()`, `symbol_importance()`, `pagerank_signal()`
- `src/intelligence/blast_radius.rs` — `compute_blast_radius()`, `compute_risk()`, categorization logic
- `src/intelligence/api_surface.rs` — `extract_api_surface()`, route detection (12 frameworks), gRPC/GraphQL extraction
- `src/intelligence/test_map.rs` — `build_test_map()`, naming convention matching, import analysis

### Modified Files
- `src/main.rs` — add `pub mod intelligence;`
- `src/index/mod.rs` — add `graph`, `pagerank`, `test_map` fields to `CodebaseIndex`, build at index time
- `src/relevance/mod.rs` — add `pagerank` weight to `SignalWeights`, update `score()` to include PageRank signal
- `src/relevance/signals.rs` — add `pub fn pagerank_signal()`
- `src/context_quality/degradation.rs` — update composite score formula to 0.6/0.2/0.2
- `src/commands/serve.rs` — add 2 new MCP tools, add `include_tests` to pack_context, update graph usage to `index.graph`
- `src/commands/trace.rs` — use `index.graph` instead of building on-demand
- `src/commands/diff.rs` — use `index.graph` instead of building on-demand
- `src/commands/overview.rs` — use `index.graph` instead of building on-demand
- `src/relevance/seed.rs` — use `index.graph` or prebuilt graph, remove fallback `build_dependency_graph` call

---

## Stream 1: Graph Caching + PageRank

### Task 1: Scaffold intelligence module

**Files:**
- Create: `src/intelligence/mod.rs`
- Create: `src/intelligence/pagerank.rs`
- Create: `src/intelligence/blast_radius.rs`
- Create: `src/intelligence/api_surface.rs`
- Create: `src/intelligence/test_map.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Create module files with minimal content**

`src/intelligence/mod.rs`:
```rust
pub mod api_surface;
pub mod blast_radius;
pub mod pagerank;
pub mod test_map;
```

Empty scaffolds for each submodule.

- [ ] **Step 2: Add `pub mod intelligence;` to `src/main.rs`**

- [ ] **Step 3: Verify compilation**

Run: `cargo check`

- [ ] **Step 4: Commit**

```bash
git add src/intelligence/ src/main.rs
git commit -m "feat: scaffold intelligence module for v0.13.0"
```

### Task 2: Cache `DependencyGraph` on `CodebaseIndex`

**Files:**
- Modify: `src/index/mod.rs`

This is the biggest architectural change — moves graph from on-demand to cached.

- [ ] **Step 1: Add `graph` field to `CodebaseIndex`**

```rust
use crate::index::graph::DependencyGraph;

pub struct CodebaseIndex {
    pub files: Vec<IndexedFile>,
    pub language_stats: HashMap<String, LanguageStats>,
    pub total_files: usize,
    pub total_bytes: u64,
    pub total_tokens: usize,
    pub term_frequencies: HashMap<String, HashMap<String, u32>>,
    pub domains: HashSet<Domain>,
    pub schema: Option<SchemaIndex>,
    pub graph: DependencyGraph,        // NEW
    pub pagerank: HashMap<String, f64>, // NEW (populated in Task 4)
    pub test_map: HashMap<String, Vec<crate::intelligence::test_map::TestFileRef>>, // NEW (populated in Task 9)
}
```

- [ ] **Step 2: Build graph in `build()` and `build_with_content()`**

Restructure both constructors:

```rust
// After building base index + schema:
let mut index = Self {
    // ... all existing fields ...
    schema: None,
    graph: DependencyGraph::new(),
    pagerank: HashMap::new(),
    test_map: HashMap::new(),
};
index.schema = crate::schema::detect::build_schema_index(&index);
index.graph = crate::index::graph::build_dependency_graph(&index, index.schema.as_ref());
// pagerank and test_map populated later (Tasks 4, 9)
index
```

- [ ] **Step 3: Update `build_dependency_graph()` to NOT take `&CodebaseIndex` (circular ref)**

The graph builder currently takes `&CodebaseIndex`. But now the graph is built as part of `CodebaseIndex::build()`, before the struct is complete. Change `build_dependency_graph` to accept `&[IndexedFile]` and `Option<&SchemaIndex>` instead of `&CodebaseIndex`:

```rust
pub fn build_dependency_graph(
    files: &[IndexedFile],
    schema: Option<&SchemaIndex>,
) -> DependencyGraph
```

Update internal logic to iterate `files` instead of `index.files`.

**ALSO update `src/schema/link.rs`:** `build_schema_edges()` currently takes `&CodebaseIndex`. It must also be changed to accept `&[IndexedFile]` + `&SchemaIndex` since it is called from inside `build_dependency_graph()`. Add `src/schema/link.rs` to modified files for this task.

**Test call sites in `serve.rs`:** Three integration tests (~lines 2826, 2893, 2978) manually assign `index.schema = Some(schema)` AFTER construction, meaning the cached graph misses schema edges. These tests must either:
- Call `index.graph = build_dependency_graph(&index.files, index.schema.as_ref());` after mutating schema
- Or use a helper `index.rebuild_graph()` method

Add a `pub fn rebuild_graph(&mut self)` convenience method on `CodebaseIndex` for this pattern.

- [ ] **Step 4: Update all on-demand callers to use `index.graph`**

Replace in each file:

| File | Old | New |
|---|---|---|
| `src/commands/trace.rs` | `let graph = build_dependency_graph(&index, index.schema.as_ref());` | `let graph = &index.graph;` |
| `src/commands/diff.rs` | `let graph = build_dependency_graph(&index, index.schema.as_ref());` | `let graph = &index.graph;` |
| `src/commands/overview.rs` | `let graph = build_dependency_graph(&index);` | `let graph = &index.graph;` |
| `src/commands/serve.rs` (~2 sites) | `build_dependency_graph(index, ...)` | `&index.graph` |
| `src/relevance/seed.rs` (fallback) | `owned_graph = build_dependency_graph(index, None);` | `owned_graph = build_dependency_graph(&index.files, index.schema.as_ref());` |

**Note on `seed.rs`:** All callers in `serve.rs` must pass `Some(&index.graph)` to `select_seeds_with_graph`, making the fallback dead code. Keep the fallback for safety but update it to use the new signature: `build_dependency_graph(&index.files, index.schema.as_ref())`. Add a comment: `// Fallback: should not be reached — callers pass prebuilt index.graph`.

- [ ] **Step 5: Run all tests**

Run: `cargo test --verbose`
Expected: All pass. Graph is now cached but behavior is identical.

- [ ] **Step 6: Commit**

```bash
git add src/index/mod.rs src/index/graph.rs src/commands/trace.rs src/commands/diff.rs src/commands/overview.rs src/commands/serve.rs src/relevance/seed.rs
git commit -m "feat: cache DependencyGraph on CodebaseIndex, eliminate on-demand construction"
```

### Task 3: Implement `compute_pagerank()`

**Files:**
- Modify: `src/intelligence/pagerank.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::graph::DependencyGraph;
    use crate::schema::EdgeType;

    #[test]
    fn test_pagerank_empty_graph() {
        let graph = DependencyGraph::new();
        let ranks = compute_pagerank(&graph, 0.85, 100);
        assert!(ranks.is_empty());
    }

    #[test]
    fn test_pagerank_single_edge() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        let ranks = compute_pagerank(&graph, 0.85, 100);
        assert!(ranks["b.rs"] > ranks["a.rs"], "imported file should rank higher");
    }

    #[test]
    fn test_pagerank_linear_chain() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "c.rs", EdgeType::Import);
        let ranks = compute_pagerank(&graph, 0.85, 100);
        assert!(ranks["c.rs"] > ranks["b.rs"]);
        assert!(ranks["b.rs"] > ranks["a.rs"]);
    }

    #[test]
    fn test_pagerank_star_pattern() {
        let mut graph = DependencyGraph::new();
        for i in 0..5 {
            graph.add_edge(&format!("file_{i}.rs"), "common.rs", EdgeType::Import);
        }
        let ranks = compute_pagerank(&graph, 0.85, 100);
        let common_rank = ranks["common.rs"];
        for i in 0..5 {
            assert!(common_rank > ranks[&format!("file_{i}.rs")]);
        }
    }

    #[test]
    fn test_pagerank_cycle() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "a.rs", EdgeType::Import);
        let ranks = compute_pagerank(&graph, 0.85, 100);
        assert!((ranks["a.rs"] - ranks["b.rs"]).abs() < 0.01, "cycle should produce equal ranks");
    }

    #[test]
    fn test_pagerank_disconnected() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("c.rs", "d.rs", EdgeType::Import);
        let ranks = compute_pagerank(&graph, 0.85, 100);
        assert!(ranks.contains_key("a.rs"));
        assert!(ranks.contains_key("c.rs"));
    }

    #[test]
    fn test_pagerank_convergence() {
        let mut graph = DependencyGraph::new();
        for i in 0..10 {
            graph.add_edge(&format!("f{i}.rs"), &format!("f{}.rs", (i+1) % 10), EdgeType::Import);
        }
        let ranks = compute_pagerank(&graph, 0.85, 100);
        // All nodes in a cycle should have approximately equal rank
        let first = ranks["f0.rs"];
        for i in 1..10 {
            assert!((ranks[&format!("f{i}.rs")] - first).abs() < 0.01);
        }
    }

    #[test]
    fn test_pagerank_normalized() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "c.rs", EdgeType::Import);
        let ranks = compute_pagerank(&graph, 0.85, 100);
        for &score in ranks.values() {
            assert!(score >= 0.0 && score <= 1.0);
        }
    }
}
```

- [ ] **Step 2: Implement `compute_pagerank()`**

```rust
use crate::index::graph::DependencyGraph;
use std::collections::HashMap;

pub fn compute_pagerank(
    graph: &DependencyGraph,
    damping: f64,
    max_iterations: usize,
) -> HashMap<String, f64> {
    // Collect all nodes
    let mut nodes: Vec<String> = Vec::new();
    for key in graph.edges.keys() {
        if !nodes.contains(key) { nodes.push(key.clone()); }
    }
    for key in graph.reverse_edges.keys() {
        if !nodes.contains(key) { nodes.push(key.clone()); }
    }
    // Also add all edge targets
    for edges in graph.edges.values() {
        for edge in edges {
            if !nodes.contains(&edge.target) { nodes.push(edge.target.clone()); }
        }
    }

    if nodes.is_empty() {
        return HashMap::new();
    }

    let n = nodes.len() as f64;
    let initial = 1.0 / n;
    let mut ranks: HashMap<String, f64> = nodes.iter().map(|node| (node.clone(), initial)).collect();
    let convergence_threshold = 1e-6;

    for _ in 0..max_iterations {
        let mut new_ranks: HashMap<String, f64> = HashMap::new();
        let mut max_delta = 0.0_f64;

        // Dangling node redistribution: nodes with no outgoing edges
        // leak rank out of the system. Collect their total rank and
        // redistribute uniformly to all nodes (standard PageRank fix).
        let dangling_sum: f64 = nodes.iter()
            .filter(|node| graph.edges.get(*node).map(|e| e.is_empty()).unwrap_or(true))
            .map(|node| ranks.get(node).unwrap_or(&0.0))
            .sum();

        for node in &nodes {
            // Sum rank contributions from nodes that have edges TO this node
            // (i.e., nodes that import/reference this node)
            let incoming_sum: f64 = graph.reverse_edges
                .get(node)
                .map(|incoming| {
                    incoming.iter().map(|edge| {
                        // IMPORTANT: In reverse_edges[node], each TypedEdge.target
                        // is the file that IMPORTS node (the forward-edge source).
                        // Naming is confusing but correct: reverse edge "target" = importer.
                        let source = &edge.target;
                        let source_out_degree = graph.edges
                            .get(source)
                            .map(|e| e.len())
                            .unwrap_or(1) as f64;
                        ranks.get(source).unwrap_or(&0.0) / source_out_degree
                    }).sum()
                })
                .unwrap_or(0.0);

            let new_rank = (1.0 - damping) / n + damping * (incoming_sum + dangling_sum / n);
            let old_rank = ranks.get(node).unwrap_or(&0.0);
            max_delta = max_delta.max((new_rank - old_rank).abs());
            new_ranks.insert(node.clone(), new_rank);
        }

        ranks = new_ranks;

        if max_delta < convergence_threshold {
            break;
        }
    }

    // Normalize to 0.0-1.0
    let max_rank = ranks.values().cloned().fold(0.0_f64, f64::max);
    if max_rank > 0.0 {
        for rank in ranks.values_mut() {
            *rank /= max_rank;
        }
    }

    ranks
}
```

- [ ] **Step 3: Run tests, verify pass**

Run: `cargo test intelligence::pagerank --verbose`

- [ ] **Step 4: Commit**

```bash
git add src/intelligence/pagerank.rs
git commit -m "feat: implement PageRank algorithm for file importance scoring"
```

### Task 4: Implement symbol importance + cross-reference index

**Files:**
- Modify: `src/intelligence/pagerank.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_build_symbol_cross_refs() {
    let mut tf = HashMap::new();
    tf.insert("a.rs".to_string(), {
        let mut m = HashMap::new();
        m.insert("authenticate".to_string(), 3u32);
        m.insert("user".to_string(), 2u32);
        m
    });
    tf.insert("b.rs".to_string(), {
        let mut m = HashMap::new();
        m.insert("authenticate".to_string(), 1u32);
        m
    });
    let refs = build_symbol_cross_refs(&tf);
    assert!(refs["authenticate"].contains("a.rs"));
    assert!(refs["authenticate"].contains("b.rs"));
    assert_eq!(refs["authenticate"].len(), 2);
    assert_eq!(refs["user"].len(), 1);
}

#[test]
fn test_symbol_importance_public_referenced() {
    let mut cross_refs = HashMap::new();
    cross_refs.insert("authenticate".to_string(), {
        let mut s = HashSet::new();
        s.insert("other.rs".to_string());
        s
    });
    let sym = Symbol {
        name: "authenticate".to_string(),
        visibility: Visibility::Public,
        ..make_test_symbol()
    };
    let importance = symbol_importance(&sym, 0.8, &cross_refs, "auth.rs");
    assert_eq!(importance, 0.8 * 1.0); // pagerank * public+referenced
}

#[test]
fn test_symbol_importance_public_unreferenced() {
    let cross_refs = HashMap::new();
    let sym = Symbol {
        name: "helper".to_string(),
        visibility: Visibility::Public,
        ..make_test_symbol()
    };
    let importance = symbol_importance(&sym, 0.8, &cross_refs, "auth.rs");
    assert_eq!(importance, 0.8 * 0.7);
}

#[test]
fn test_symbol_importance_private() {
    let cross_refs = HashMap::new();
    let sym = Symbol {
        name: "internal".to_string(),
        visibility: Visibility::Private,
        ..make_test_symbol()
    };
    let importance = symbol_importance(&sym, 0.8, &cross_refs, "auth.rs");
    assert_eq!(importance, 0.8 * 0.3);
}
```

- [ ] **Step 2: Implement `build_symbol_cross_refs()` and `symbol_importance()`**

Per the spec — inverted index from term_frequencies, O(1) lookups.

- [ ] **Step 3: Wire PageRank computation into `CodebaseIndex::build()`**

After building the graph:
```rust
index.pagerank = crate::intelligence::pagerank::compute_pagerank(&index.graph, 0.85, 100);
```

- [ ] **Step 4: Run tests, verify pass**

- [ ] **Step 5: Commit**

```bash
git add src/intelligence/pagerank.rs src/index/mod.rs
git commit -m "feat: symbol importance with cross-reference index, PageRank cached on index"
```

### Task 5: Add PageRank as signal #6 in relevance scoring

**Files:**
- Modify: `src/relevance/signals.rs`
- Modify: `src/relevance/mod.rs`

- [ ] **Step 1: Add `pagerank_signal()` function**

In `src/relevance/signals.rs`:
```rust
pub fn pagerank_signal(file_path: &str, pagerank: &HashMap<String, f64>) -> SignalResult {
    let score = pagerank.get(file_path).copied().unwrap_or(0.0);
    SignalResult {
        name: "pagerank",
        score,
        detail: format!("pagerank={:.4}", score),
    }
}
```

- [ ] **Step 2: Add `pagerank` weight to `SignalWeights`**

```rust
pub struct SignalWeights {
    pub path_similarity: f64,    // 0.18
    pub symbol_match: f64,       // 0.32
    pub import_proximity: f64,   // 0.14
    pub term_frequency: f64,     // 0.19
    pub recency_boost: f64,      // 0.00
    pub pagerank: f64,           // 0.17
}
```

Update `Default` impl with new values summing to 1.0.

- [ ] **Step 3: Update `MultiSignalScorer::score()` to include PageRank**

The `RelevanceScorer` trait method signature stays unchanged. `MultiSignalScorer` reads `pagerank` from the stored `CodebaseIndex` reference. Since `score()` takes `&CodebaseIndex`, it can access `index.pagerank`:

```rust
let pr_sig = signals::pagerank_signal(file_path, &index.pagerank);
// ... add w.pagerank * pr_sig.score to combined
```

Wait — the current `RelevanceScorer::score()` takes `(query, file_path, index)` where `index: &CodebaseIndex`. Since `index` now has `pagerank`, this works without any trait change.

- [ ] **Step 4: Write tests**

```rust
#[test]
fn test_pagerank_signal_found() {
    let mut pr = HashMap::new();
    pr.insert("api.rs".to_string(), 0.85);
    let result = pagerank_signal("api.rs", &pr);
    assert_eq!(result.score, 0.85);
}

#[test]
fn test_pagerank_signal_not_found() {
    let pr = HashMap::new();
    let result = pagerank_signal("unknown.rs", &pr);
    assert_eq!(result.score, 0.0);
}

#[test]
fn test_weights_sum_to_one() {
    let w = SignalWeights::default();
    let sum = w.path_similarity + w.symbol_match + w.import_proximity
        + w.term_frequency + w.recency_boost + w.pagerank;
    assert!((sum - 1.0).abs() < 0.001);
}

#[test]
fn test_high_pagerank_boosts_score() {
    // Build index where file A has high PageRank and file B has low
    // Score both for same query — A should score higher
}
```

- [ ] **Step 5: Run all relevance tests**

Run: `cargo test relevance --verbose`

- [ ] **Step 6: Commit**

```bash
git add src/relevance/signals.rs src/relevance/mod.rs
git commit -m "feat: add PageRank as signal #6 in MultiSignalScorer"
```

### Task 6: Update degradation formula

**Files:**
- Modify: `src/context_quality/degradation.rs`

- [ ] **Step 1: Update `allocate_with_degradation()` to accept PageRank**

Add `pagerank: Option<&HashMap<String, f64>>` parameter. When present:
```rust
let pr = pagerank.and_then(|p| p.get(&file_path)).copied().unwrap_or(0.0);
let priority = score * 0.6 + cp * 0.2 + pr * 0.2;
```

When `None` (backwards compatibility): `let priority = score * 0.7 + cp * 0.3;`

- [ ] **Step 2: Update callers to pass PageRank**

`serve.rs` pack_context handler: pass `Some(&index.pagerank)`.

- [ ] **Step 3: Write test**

```rust
#[test]
fn test_degradation_with_pagerank() {
    // Two files same relevance score, same concept priority
    // File A has PageRank 0.9, file B has 0.1
    // File A should survive at higher detail level
}
```

- [ ] **Step 4: Run tests, verify pass**

- [ ] **Step 5: Commit**

```bash
git add src/context_quality/degradation.rs src/commands/serve.rs
git commit -m "feat: update degradation formula to include PageRank (0.6/0.2/0.2)"
```

---

## Stream 2: Test File Mapping

### Task 7: Naming convention matching

**Files:**
- Modify: `src/intelligence/test_map.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_naming_rust() { /* foo.rs → foo_test.rs, tests/foo.rs */ }
#[test]
fn test_naming_python() { /* auth.py → test_auth.py, tests/test_auth.py */ }
#[test]
fn test_naming_java() { /* User.java → UserTest.java */ }
#[test]
fn test_naming_typescript() { /* foo.ts → foo.test.ts, foo.spec.ts, __tests__/foo.test.ts */ }
#[test]
fn test_naming_go() {
    // Go convention: test file in SAME directory as source.
    // src/db/store.go → src/db/store_test.go (preserve full directory path)
    // NOT src/store_test.go — directory must be preserved.
}
#[test]
fn test_naming_ruby() { /* foo.rb → spec/foo_spec.rb, test/foo_test.rb */ }
#[test]
fn test_naming_no_match() { /* standalone.rs with no test file → empty */ }
```

- [ ] **Step 2: Implement naming convention matching**

```rust
pub fn find_test_files_by_name(
    source_path: &str,
    all_paths: &HashSet<String>,
) -> Vec<TestFileRef>
```

Generate candidate test file paths from the source path using the language-specific patterns from the spec. Check each against `all_paths`.

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/intelligence/test_map.rs
git commit -m "feat: test file mapping via naming conventions (6 language patterns)"
```

### Task 8: Import analysis for test mapping

**Files:**
- Modify: `src/intelligence/test_map.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_import_analysis() {
    // test_auth.py imports src.auth.middleware
    // → maps test_auth.py to src/auth/middleware.py
}
#[test]
fn test_both_name_and_import() {
    // test_auth.py BOTH matches naming AND imports auth.py → confidence Both
}
#[test]
fn test_multiple_sources_per_test() {
    // integration_test.py imports 3 modules → maps to all 3
}
#[test]
fn test_multiple_tests_per_source() {
    // auth.py has both unit test and integration test → both listed
}
```

- [ ] **Step 2: Implement import analysis**

```rust
pub fn find_test_files_by_imports(
    index: &[IndexedFile],
) -> HashMap<String, Vec<TestFileRef>>
```

For each file whose path contains `test`, `spec`, or `__tests__`, examine its imports. Resolve import sources to file paths (reuse logic from `build_dependency_graph`). Create mapping entries.

- [ ] **Step 3: Implement `build_test_map()` orchestrator**

Merges naming convention results + import analysis. Deduplicates. Sets confidence levels.

```rust
pub fn build_test_map(
    files: &[IndexedFile],
    all_paths: &HashSet<String>,
) -> HashMap<String, Vec<TestFileRef>>
```

- [ ] **Step 4: Run tests, verify pass**

- [ ] **Step 5: Commit**

```bash
git add src/intelligence/test_map.rs
git commit -m "feat: test file mapping via import analysis + orchestrator"
```

### Task 9: Wire test mapping into CodebaseIndex

**Files:**
- Modify: `src/index/mod.rs`

- [ ] **Step 1: Populate `test_map` during build**

After graph + PageRank:
```rust
let all_paths: HashSet<String> = index.files.iter().map(|f| f.relative_path.clone()).collect();
index.test_map = crate::intelligence::test_map::build_test_map(&index.files, &all_paths);
```

- [ ] **Step 2: Write integration test**

```rust
#[test]
fn test_index_builds_test_map() {
    // Create temp repo with src/auth.rs and tests/auth_test.rs
    // Build index → verify test_map contains the mapping
}
```

- [ ] **Step 3: Run tests**

- [ ] **Step 4: Commit**

```bash
git add src/index/mod.rs
git commit -m "feat: populate test_map on CodebaseIndex at build time"
```

### Task 10: Wire test mapping into pack_context

**Files:**
- Modify: `src/commands/serve.rs`

- [ ] **Step 1: Add `include_tests` parameter to `cxpak_pack_context` schema**

```json
"include_tests": { "type": "boolean", "description": "Auto-include test files for packed source files (default true)", "default": true }
```

- [ ] **Step 2: Implement auto-inclusion**

When `include_tests` is true, for each packed file check `index.test_map`. Add mapped test files with `"included_as": "test_file"`.

- [ ] **Step 3: Write tests**

```rust
#[test]
fn test_pack_context_includes_tests() { ... }
#[test]
fn test_pack_context_excludes_tests_when_false() { ... }
```

- [ ] **Step 4: Commit**

```bash
git add src/commands/serve.rs
git commit -m "feat: auto-include test files in pack_context (include_tests param)"
```

---

## Stream 3: Blast Radius

### Task 11: Implement blast radius core

**Files:**
- Modify: `src/intelligence/blast_radius.rs`

**COMPILE-TIME DEPENDENCY:** This task requires `TestFileRef` from `src/intelligence/test_map.rs` (Tasks 7-8). Must be implemented AFTER Stream 2 completes. Add `use crate::intelligence::test_map::TestFileRef;` at the top of `blast_radius.rs`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_blast_radius_direct_dependent() { ... }
#[test]
fn test_blast_radius_transitive() { ... }
#[test]
fn test_blast_radius_depth_limit() { ... }
#[test]
fn test_blast_radius_focus_filter() { ... }
#[test]
fn test_blast_radius_schema_dependents() { ... }
#[test]
fn test_blast_radius_test_files_categorized() {
    // tests/auth_test.rs imports src/auth.rs AND is in test_map for auth.rs
    // → categorized as test_files (not direct_dependents), priority wins
    // ALSO: tests/unrelated_test.rs has test-pattern path BUT is NOT in test_map
    // → falls through to direct_dependents (must satisfy BOTH conditions)
}
#[test]
fn test_risk_hop_decay() { ... }
#[test]
fn test_risk_edge_weight() { ... }
#[test]
fn test_risk_untested_penalty() { ... }
#[test]
fn test_risk_pagerank_factor() { ... }
#[test]
fn test_risk_thresholds() { ... }
#[test]
fn test_blast_radius_empty() { ... }
#[test]
fn test_blast_radius_multiple_changed_files() { ... }
#[test]
fn test_blast_radius_circular_no_panic() { ... }
#[test]
fn test_blast_radius_mixed_edge_types_highest_wins() { ... }
#[test]
fn test_risk_clamped_to_one() {
    // Direct dependent (hops=1) with PageRank=1.0, untested (penalty=1.2), Import edge (1.0)
    // raw = 1.0/(1+1) * 1.0 * 1.0 * 1.2 = 0.6 — within bounds
    // BUT: use hops=1 (NOT 0) for direct dependents. Changed files are seeds, not results.
    // This test verifies the convention: first BFS frontier = hops 1.
    let risk = compute_risk(1, &EdgeType::Import, 1.0, false);
    assert!(risk <= 1.0);
    assert!(risk > 0.5, "direct dependent with high pagerank should be high risk");
}
```

- [ ] **Step 2: Implement `compute_risk()` and `compute_blast_radius()`**

```rust
pub struct BlastRadiusResult {
    pub changed_files: Vec<String>,
    pub total_affected: usize,
    pub categories: BlastRadiusCategories,
    pub risk_summary: RiskSummary,
}

pub struct BlastRadiusCategories {
    pub direct_dependents: Vec<AffectedFile>,
    pub transitive_dependents: Vec<AffectedFile>,
    pub test_files: Vec<AffectedFile>,
    pub schema_dependents: Vec<AffectedFile>,
}

pub struct AffectedFile {
    pub path: String,
    pub edge_type: String,
    pub hops: usize,
    pub risk: f64,
    pub note: Option<String>,
}

pub struct RiskSummary {
    pub high: usize,
    pub medium: usize,
    pub low: usize,
}

pub fn compute_blast_radius(
    changed_files: &[&str],
    graph: &DependencyGraph,
    pagerank: &HashMap<String, f64>,
    test_map: &HashMap<String, Vec<TestFileRef>>,
    depth: usize,
    focus: Option<&str>,
) -> BlastRadiusResult
```

BFS on reverse edges, categorize, score, respect depth + focus.

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/intelligence/blast_radius.rs
git commit -m "feat: blast radius analysis with risk scoring and categorization"
```

### Task 12: Wire blast radius as MCP tool

**Files:**
- Modify: `src/commands/serve.rs`

- [ ] **Step 1: Add `cxpak_blast_radius` to tools/list schema**

- [ ] **Step 2: Implement handler**

Parse args, call `compute_blast_radius()`, serialize result.

- [ ] **Step 3: Add `POST /blast_radius` HTTP endpoint**

- [ ] **Step 4: Write MCP round-trip test**

- [ ] **Step 5: Update tools/list test to expect 9 tools (was 7)**

- [ ] **Step 6: Commit**

```bash
git add src/commands/serve.rs
git commit -m "feat: add cxpak_blast_radius MCP tool (#8)"
```

---

## Stream 4: API Surface

### Task 13: Public symbol extraction

**Files:**
- Modify: `src/intelligence/api_surface.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_extract_public_symbols_only() { ... }
#[test]
fn test_doc_comments_included() { ... }
#[test]
fn test_sorted_by_pagerank() { ... }
#[test]
fn test_private_symbols_excluded() { ... }
#[test]
fn test_focus_filter() { ... }
```

- [ ] **Step 2: Implement public symbol extraction**

Filter index to public symbols, include signature + doc comment (reuse v0.11.0 doc extraction), sort by PageRank.

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/intelligence/api_surface.rs
git commit -m "feat: public symbol extraction for API surface"
```

### Task 14: Route detection (12 frameworks)

**Files:**
- Modify: `src/intelligence/api_surface.rs`

- [ ] **Step 1: Write failing tests — one per framework**

```rust
#[test]
fn test_route_express() {
    let content = r#"app.get("/api/users", getUsers);"#;
    let routes = detect_routes(content, "routes.ts");
    assert_eq!(routes[0].method, "GET");
    assert_eq!(routes[0].path, "/api/users");
}
// ... 11 more framework tests ...
#[test]
fn test_route_non_route_string() {
    let content = r#"let path = "/api/users";"#;
    let routes = detect_routes(content, "config.ts");
    assert!(routes.is_empty());
}
```

- [ ] **Step 2: Implement `detect_routes()`**

12 regex patterns, each extracting method + path + handler.

**IMPORTANT implementation notes:**
- All alternation operators must be unescaped `|` in Rust source, NOT `\|` as shown in the Markdown spec table.
- Echo pattern: use `(e|g|echo|group)` (NOT just `(e|g)` which is too broad and produces false positives on any single-char variable).
- Django (`*urls*.py`), Rails (`*routes*`), Phoenix (`*router*`): the file-glob condition is a **pre-filter on file paths**, NOT part of the regex. Apply `file.relative_path.contains("urls")` etc. before running the regex on content.

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/intelligence/api_surface.rs
git commit -m "feat: HTTP route detection for 12 frameworks"
```

### Task 15: gRPC + GraphQL extraction and orchestrator

**Files:**
- Modify: `src/intelligence/api_surface.rs`

- [ ] **Step 1: Implement gRPC/GraphQL extraction from existing parsed symbols**

Filter Proto files for `Service`/`Method` kinds, GraphQL files for `Query`/`Mutation`/`Type` kinds.

- [ ] **Step 2: Implement `extract_api_surface()` orchestrator**

Combines: public symbols + routes + gRPC + GraphQL. Applies token budget via degradation.

- [ ] **Step 3: Write integration test**

- [ ] **Step 4: Commit**

```bash
git add src/intelligence/api_surface.rs
git commit -m "feat: API surface orchestrator with gRPC, GraphQL, token budget"
```

### Task 16: Wire API surface as MCP tool

**Files:**
- Modify: `src/commands/serve.rs`

- [ ] **Step 1: Add `cxpak_api_surface` to tools/list schema**

- [ ] **Step 2: Implement handler**

- [ ] **Step 3: Add `POST /api_surface` HTTP endpoint**

- [ ] **Step 4: Write MCP round-trip test**

- [ ] **Step 5: Commit**

```bash
git add src/commands/serve.rs
git commit -m "feat: add cxpak_api_surface MCP tool (#9)"
```

---

## Stream 5: Integration + Documentation + QA

### Task 17: Integration tests

**Files:**
- Add tests

- [ ] **Step 1: Write end-to-end tests**

```rust
#[test]
fn test_pipeline_pagerank_blast_radius() { ... }
#[test]
fn test_pipeline_api_surface_with_budget() { ... }
#[test]
fn test_pipeline_test_map_in_pack_context() { ... }
#[test]
fn test_pipeline_context_for_task_with_pagerank() { ... }
#[test]
fn test_mcp_blast_radius_and_api_surface_same_session() { ... }
#[test]
fn test_large_repo_all_features() { ... }
#[test]
fn test_no_tests_repo() { ... }
#[test]
fn test_no_routes_repo() { ... }
#[test]
fn test_single_file_repo() { ... }
#[test]
fn test_all_features_combined() { ... }
```

- [ ] **Step 2: Run all tests**

Run: `cargo test --verbose`

- [ ] **Step 3: Commit**

```bash
git add tests/ src/
git commit -m "test: integration tests for v0.13.0 intelligence features"
```

### Task 18: Documentation

**Files:**
- Modify: `README.md`, `.claude/CLAUDE.md`, `plugin/README.md`

- [ ] **Step 1: Update all docs**

- README: document PageRank, blast_radius, api_surface, test mapping, 9 MCP tools
- CLAUDE.md: add intelligence module to architecture
- Plugin README: document new MCP tools

- [ ] **Step 2: Commit**

```bash
git add README.md .claude/CLAUDE.md plugin/README.md
git commit -m "docs: document intelligence features for v0.13.0"
```

### Task 19: Version bump

- [ ] **Step 1: Bump to 0.13.0** in Cargo.toml, plugin.json, marketplace.json, ensure-cxpak

- [ ] **Step 2: Commit**

```bash
git add Cargo.toml plugin/.claude-plugin/plugin.json .claude-plugin/marketplace.json plugin/lib/ensure-cxpak
git commit -m "chore: bump version to 0.13.0"
```

### Task 20: Pre-Release QA + CI Validation

- [ ] **Step 1: Run full test suite** — `cargo test --verbose`
- [ ] **Step 2: Run clippy** — `cargo clippy --all-targets -- -D warnings`
- [ ] **Step 3: Run formatter** — `cargo fmt -- --check`
- [ ] **Step 4: Run coverage** — `cargo tarpaulin --verbose --all-features --workspace --timeout 120 --fail-under 90`
- [ ] **Step 5: Manual QA — PageRank**

```bash
cargo run -- overview --tokens 10k .
# Verify files are ordered by importance in the module map
```

- [ ] **Step 6: Manual QA — blast radius**

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_blast_radius","arguments":{"files":["src/index/mod.rs"]}}}' | cargo run --features daemon -- serve --mcp .
# Verify categorized results with risk scores
```

- [ ] **Step 7: Manual QA — API surface**

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_api_surface","arguments":{}}}' | cargo run --features daemon -- serve --mcp .
# Verify public symbols extracted, sorted by importance
```

- [ ] **Step 8: Manual QA — test mapping**

Verify `pack_context` auto-includes test files when `include_tests: true`.

- [ ] **Step 9: Simulate CI**

```bash
cargo build --verbose && cargo test --verbose && cargo clippy --all-targets -- -D warnings && cargo fmt -- --check && cargo tarpaulin --verbose --all-features --workspace --timeout 120 --fail-under 90
```

- [ ] **Step 10: Tag and push**

```bash
git tag v0.13.0
git push origin main --tags
```

---

## Task Summary

| Stream | Tasks | Dependencies |
|---|---|---|
| 1. Graph Caching + PageRank | Tasks 1-6 | Sequential (scaffold → cache graph → PageRank → symbol importance → signal → degradation) |
| 2. Test Mapping | Tasks 7-10 | Task 2 (cached graph needed for import analysis) |
| 3. Blast Radius | Tasks 11-12 | Tasks 4+9 (needs PageRank + test_map for risk scoring) |
| 4. API Surface | Tasks 13-16 | Task 4 (needs PageRank for sorting) |
| 5. Integration + QA | Tasks 17-20 | All prior |

**Parallelizable:** After Task 6, Streams 2-4 can overlap:
- Tasks 7-10 (test mapping) are independent of Tasks 13-16 (API surface)
- Tasks 11-12 (blast radius) need test_map from Task 9, so wait for Stream 2
- Tasks 13-16 (API surface) only need PageRank from Task 4

**Critical path:** Tasks 1-6 → (Tasks 7-10 → Tasks 11-12) ∥ Tasks 13-16 → Tasks 17-20

**Total: 20 tasks, ~80 new tests, 100% branch coverage on `src/intelligence/`, 95%+ on modified modules, 90%+ overall CI gate. Task 20 is the release gate.**
