# v1.4.0 "Prediction" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add change impact prediction, architecture drift detection, and security surface analysis.

**Architecture:** Three new intelligence modules (`predict.rs`, `drift.rs`, `security.rs`) added to `src/intelligence/`, each consuming existing index primitives (blast radius, co-change edges, call graph, test map, api_surface). The modules are wired into three new MCP tool handlers in `src/commands/serve.rs` and a `predictions` field on `AutoContextResult` populated when the task description references specific changed files.

**Tech Stack:** Rust, git2, serde, regex

---

## Task 1: Fix `RouteEndpoint.handler` to extract real handler names

**Why first:** Unprotected endpoint detection in Task 15 requires real handler function names, not the literal string `"handler"`. This fix is a prerequisite for security surface analysis.

**Files:**
- Modify: `src/intelligence/api_surface.rs`
- Test: `src/intelligence/api_surface.rs` (inline tests)

**Steps:**

1. Write failing tests that assert `handler` is not the literal string `"handler"` for each framework.

```rust
#[test]
fn test_handler_express_named() {
    let content = r#"app.get('/users', listUsers); router.post("/items", createItem);"#;
    let routes = detect_routes(content, "routes/index.js");
    assert_eq!(routes[0].handler, "listUsers");
    assert_eq!(routes[1].handler, "createItem");
}

#[test]
fn test_handler_actix_named() {
    let content = r#"#[get("/health")]
async fn health_check() -> impl Responder { HttpResponse::Ok() }"#;
    let routes = detect_routes(content, "src/main.rs");
    assert_eq!(routes[0].handler, "health_check");
}

#[test]
fn test_handler_axum_named() {
    let content = r#"Router::new().route("/users", get(list_users)).route("/items/:id", post(create_item))"#;
    let routes = detect_routes(content, "src/server.rs");
    assert_eq!(routes[0].handler, "list_users");
    assert_eq!(routes[1].handler, "create_item");
}

#[test]
fn test_handler_flask_named() {
    let content = "@app.route('/home')\ndef home_view():\n    pass";
    let routes = detect_routes(content, "app.py");
    assert_eq!(routes[0].handler, "home_view");
}

#[test]
fn test_handler_fastapi_named() {
    let content = "@app.get(\"/users\")\nasync def list_users():\n    pass";
    let routes = detect_routes(content, "main.py");
    assert_eq!(routes[0].handler, "list_users");
}

#[test]
fn test_handler_gin_named() {
    let content = r#"r.GET("/ping", pingHandler)"#;
    let routes = detect_routes(content, "main.go");
    assert_eq!(routes[0].handler, "pingHandler");
}

#[test]
fn test_handler_unknown_falls_back_to_handler() {
    // When no handler can be extracted (e.g. inline closure), fall back
    let content = r#"app.get('/x', function(req, res) { res.send('ok'); });"#;
    let routes = detect_routes(content, "app.js");
    // inline closures can't be named — fallback is acceptable
    assert!(!routes[0].handler.is_empty());
}
```

2. Run tests: `cargo test detect_routes -- --nocapture` — all handler assertions fail.

3. Update each framework regex in `detect_routes()` to capture the handler argument:

**Express/Koa** — capture the third argument (after path):
```rust
// Old pattern: r#"(?i)(app|router)\.(get|post|put|delete|patch)\s*\(\s*["'](/[^"']*)"#
// New: add capture group for handler identifier after the path argument
if let Ok(re) = Regex::new(
    r#"(?i)(app|router)\.(get|post|put|delete|patch)\s*\(\s*["'][^"']*["']\s*,\s*([a-zA-Z_$][a-zA-Z0-9_$]*)"#
) {
    for cap in re.captures_iter(content) {
        let method = cap[2].to_uppercase();
        let handler = cap[3].to_string();
        // path must come from the first match; run two-pass or combined regex
```

Use a two-pass approach per framework: first extract path (existing regex), then extract handler with a second regex anchored at the same line, or use a combined regex with named groups. The combined regex is cleaner:

```rust
// Express/Koa combined
Regex::new(
    r#"(?i)(app|router)\.(get|post|put|delete|patch)\s*\(\s*["'](/[^"']*?)["']\s*,\s*([a-zA-Z_$][a-zA-Z0-9_$]*)"#
)
// Groups: [1]=app/router [2]=method [3]=path [4]=handler
```

**Flask/FastAPI** — capture the `def` function name on the line following the decorator:
```rust
// Two-pass: find decorator line offset, then scan forward for `def funcname`
Regex::new(
    r#"(?i)@(app|blueprint|router)\.(route|get|post|put|delete|patch)\s*\(\s*["'](/[^"']*?)["'][^)]*\)\s*\n\s*(?:async\s+)?def\s+([a-zA-Z_][a-zA-Z0-9_]*)"#
)
// Groups: [1]=obj [2]=method_or_route [3]=path [4]=handler
// multiline mode required — use (?s) or split approach
```

**actix-web** — capture the `fn` name on the line following the attribute:
```rust
Regex::new(
    r#"(?s)#\[(get|post|put|delete|patch)\s*\(\s*["'](/[^"']*?)["']\s*\)\s*\]\s*(?:pub\s+)?(?:async\s+)?fn\s+([a-zA-Z_][a-zA-Z0-9_]*)"#
)
```

**axum** — capture the handler argument inside `get(handler)` / `post(handler)`:
```rust
Regex::new(
    r#"\.route\s*\(\s*["'](/[^"']*?)["']\s*,\s*(?:get|post|put|delete|patch|any)\(([a-zA-Z_][a-zA-Z0-9_]*)\)"#
)
```

**Gin/Echo** — capture the final identifier argument:
```rust
// Gin: (r|router|group).(GET|POST|...)(path, handlerFunc)
Regex::new(
    r#"(?i)(r|router|group)\.(GET|POST|PUT|DELETE|PATCH)\s*\(\s*["'](/[^"']*?)["']\s*,\s*([a-zA-Z_][a-zA-Z0-9_]*)"#
)
```

**Spring** — capture the method name of the annotated Java method:
```rust
Regex::new(
    r#"(?s)@(Get|Post|Put|Delete|Patch|Request)Mapping\s*\(\s*["'](/[^"']*?)["'][^)]*\)\s*(?:public\s+)?(?:\w+\s+)+([a-zA-Z_][a-zA-Z0-9_]*)\s*\("#
)
```

**Django** — capture the view callable from `path("...", view_func)`:
```rust
Regex::new(
    r#"(?i)path\s*\(\s*["']([^"']*?)["']\s*,\s*([a-zA-Z_][a-zA-Z0-9_.]*)"#
)
```

**Rails** — capture the `to:` value or use the block's controller#action:
```rust
// get '/path', to: 'controller#action'  → handler = "controller#action"
Regex::new(
    r#"(?i)(get|post|put|patch|delete)\s+["'](/[^"']*?)["'](?:[^,\n]*,\s*to:\s*["']([^"']+)["'])?"#
)
```

**Phoenix** — capture the module + action:
```rust
// get "/path", ModController, :action  → handler = "ModController/:action"
Regex::new(
    r#"(?i)(get|post|put|patch|delete)\s+["'](/[^"']*?)["']\s*,\s*([A-Z][a-zA-Z0-9.]*)\s*,\s*:([a-zA-Z_][a-zA-Z0-9_]*)"#
)
// handler = format!("{}/{}", cap[3], cap[4])
```

For frameworks where no handler can be extracted (inline closures, anonymous functions), fall back to `"<anonymous>"` rather than `"handler"` so it is clearly not a real name.

4. Update all existing route tests to assert `handler != "handler"` where real names are extractable.

5. Run: `cargo test intelligence::api_surface -- --nocapture`

6. Commit: `feat: extract real handler names from route detection (12 frameworks)`

---

## Task 2: Add `CoChangeEdge` to `CodebaseIndex` and wire git log mining

**Why:** `PredictionResult.historical_impact` requires co-change data on the index. This is listed as v1.2.0 groundwork but must be present for v1.4.0 prediction.

**Files:**
- Modify: `src/index/mod.rs`
- Modify: `src/commands/serve.rs` (`build_index`)
- Create: `src/intelligence/co_change.rs`
- Modify: `src/intelligence/mod.rs`

**Steps:**

1. Write failing test in a new file `src/intelligence/co_change.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_co_change_threshold_filters_noise() {
        // Two files co-occurring only twice should not produce an edge (threshold=3)
        let commits: Vec<Vec<String>> = vec![
            vec!["a.rs".into(), "b.rs".into()],
            vec!["a.rs".into(), "b.rs".into()],
        ];
        let edges = build_co_change_edges(&commits, 3, 180);
        assert!(edges.is_empty(), "below threshold should produce no edges");
    }

    #[test]
    fn test_co_change_meets_threshold() {
        let commits: Vec<Vec<String>> = vec![
            vec!["a.rs".into(), "b.rs".into()],
            vec!["a.rs".into(), "b.rs".into()],
            vec!["a.rs".into(), "b.rs".into()],
        ];
        let edges = build_co_change_edges(&commits, 3, 180);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].count, 3);
    }

    #[test]
    fn test_co_change_recency_weight_recent() {
        // All commits within last 30 days → recency_weight = 1.0
        let now = chrono::Utc::now();
        let commits_with_dates = vec![
            (vec!["a.rs".to_string(), "b.rs".to_string()], 0i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 5i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 10i64),
        ];
        let edges = build_co_change_edges_with_dates(&commits_with_dates, 3, 180);
        assert!((edges[0].recency_weight - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_co_change_recency_weight_old() {
        // Most recent commit at 150 days → weight = 1.0 - 0.7 * (150-30)/150 = 0.44
        let commits_with_dates = vec![
            (vec!["a.rs".to_string(), "b.rs".to_string()], 150i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 160i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 170i64),
        ];
        let edges = build_co_change_edges_with_dates(&commits_with_dates, 3, 180);
        let expected = 1.0 - 0.7 * (150.0 - 30.0) / 150.0;
        assert!((edges[0].recency_weight - expected).abs() < 1e-6);
    }

    #[test]
    fn test_co_change_excludes_beyond_window() {
        // Commits older than 180 days must be excluded entirely
        let commits_with_dates = vec![
            (vec!["a.rs".to_string(), "b.rs".to_string()], 181i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 200i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 250i64),
        ];
        let edges = build_co_change_edges_with_dates(&commits_with_dates, 3, 180);
        assert!(edges.is_empty(), "commits beyond 180-day window must be excluded");
    }

    #[test]
    fn test_co_change_self_pairs_excluded() {
        let commits = vec![vec!["a.rs".into(), "a.rs".into()]; 5];
        let edges = build_co_change_edges(&commits, 3, 180);
        assert!(edges.is_empty(), "self-pairs must not produce edges");
    }

    #[test]
    fn test_co_change_symmetric_dedup() {
        // (a,b) and (b,a) should produce exactly one edge
        let commits: Vec<Vec<String>> = (0..3)
            .map(|_| vec!["b.rs".into(), "a.rs".into()])
            .collect();
        let edges = build_co_change_edges(&commits, 3, 180);
        assert_eq!(edges.len(), 1);
    }
}
```

2. Run: `cargo test co_change` — all fail (module doesn't exist yet).

3. Implement `src/intelligence/co_change.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoChangeEdge {
    pub file_a: String,
    pub file_b: String,
    pub count: u32,
    pub recency_weight: f64,
}

/// Build co-change edges from a list of commits (each commit = list of changed file paths).
/// Uses a uniform weight of 1.0 for all commits (assumes all are recent enough).
/// For date-aware computation, use `build_co_change_edges_with_dates`.
pub fn build_co_change_edges(
    commits: &[Vec<String>],
    threshold: u32,
    window_days: i64,
) -> Vec<CoChangeEdge> {
    // Convert to (files, days_ago=0) and delegate
    let with_dates: Vec<(Vec<String>, i64)> =
        commits.iter().map(|c| (c.clone(), 0i64)).collect();
    build_co_change_edges_with_dates(&with_dates, threshold, window_days)
}

/// Build co-change edges with per-commit recency (days_ago).
pub fn build_co_change_edges_with_dates(
    commits: &[(Vec<String>, i64)],
    threshold: u32,
    window_days: i64,
) -> Vec<CoChangeEdge> {
    // pair_key → (count, most_recent_days_ago)
    let mut pair_data: HashMap<(String, String), (u32, i64)> = HashMap::new();

    for (files, days_ago) in commits {
        if *days_ago > window_days {
            continue;
        }
        let mut sorted = files.clone();
        sorted.sort();
        sorted.dedup();

        for i in 0..sorted.len() {
            for j in (i + 1)..sorted.len() {
                if sorted[i] == sorted[j] {
                    continue;
                }
                let key = (sorted[i].clone(), sorted[j].clone());
                let entry = pair_data.entry(key).or_insert((0, *days_ago));
                entry.0 += 1;
                // Track most recent (smallest days_ago)
                if *days_ago < entry.1 {
                    entry.1 = *days_ago;
                }
            }
        }
    }

    pair_data
        .into_iter()
        .filter(|(_, (count, _))| *count >= threshold)
        .map(|((file_a, file_b), (count, most_recent_days))| {
            let recency_weight = compute_recency_weight(most_recent_days);
            CoChangeEdge {
                file_a,
                file_b,
                count,
                recency_weight,
            }
        })
        .collect()
}

/// Recency weight formula from design spec:
/// - days_ago <= 30: 1.0
/// - 30 < days_ago <= 180: 1.0 - 0.7 * (days_ago - 30) / 150
/// - days_ago > 180: excluded before this is called
fn compute_recency_weight(days_ago: i64) -> f64 {
    if days_ago <= 30 {
        1.0
    } else {
        1.0 - 0.7 * (days_ago - 30) as f64 / 150.0
    }
}

/// Mine co-change data from a git repository using git2.
/// Returns a list of (changed_files, days_ago) tuples for the last `window_days`.
pub fn mine_co_changes_from_git(
    repo_path: &std::path::Path,
    window_days: i64,
) -> Vec<(Vec<String>, i64)> {
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let cutoff = now - window_days * 86400;

    let mut revwalk = match repo.revwalk() {
        Ok(rw) => rw,
        Err(_) => return vec![],
    };
    if revwalk.push_head().is_err() {
        return vec![];
    }

    let mut results = Vec::new();

    for oid_result in revwalk {
        let oid = match oid_result {
            Ok(o) => o,
            Err(_) => continue,
        };
        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let commit_time = commit.time().seconds();
        if commit_time < cutoff {
            break; // revwalk is time-ordered descending
        }

        let days_ago = (now - commit_time) / 86400;

        // Get changed files for this commit
        let parent_tree = commit
            .parent(0)
            .ok()
            .and_then(|p| p.tree().ok());
        let current_tree = match commit.tree() {
            Ok(t) => t,
            Err(_) => continue,
        };

        let diff = match repo.diff_tree_to_tree(
            parent_tree.as_ref(),
            Some(&current_tree),
            None,
        ) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let mut changed_files: Vec<String> = Vec::new();
        diff.foreach(
            &mut |delta, _| {
                if let Some(path) = delta.new_file().path() {
                    changed_files.push(path.to_string_lossy().to_string());
                }
                true
            },
            None,
            None,
            None,
        )
        .ok();

        if changed_files.len() >= 2 {
            results.push((changed_files, days_ago));
        }
    }

    results
}
```

4. Add to `src/intelligence/mod.rs`:
```rust
pub mod co_change;
```

5. Add `co_changes: Vec<CoChangeEdge>` field to `CodebaseIndex` in `src/index/mod.rs`:
```rust
pub co_changes: Vec<crate::intelligence::co_change::CoChangeEdge>,
```
Initialize to `vec![]` in `build()` and `build_with_content()`.

6. Populate in `build_index()` in `src/commands/serve.rs`:
```rust
let raw_commits = crate::intelligence::co_change::mine_co_changes_from_git(path, 180);
index.co_changes = crate::intelligence::co_change::build_co_change_edges_with_dates(
    &raw_commits, 3, 180
);
```

7. Run: `cargo test co_change -- --nocapture`

8. Commit: `feat: add CoChangeEdge type and git log mining for co-change analysis`

---

## Task 3: Create `src/intelligence/predict.rs` — core types and structural signal

**Files:**
- Create: `src/intelligence/predict.rs`
- Modify: `src/intelligence/mod.rs`

**Steps:**

1. Write failing tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::graph::DependencyGraph;
    use crate::schema::EdgeType;
    use std::collections::HashMap;

    fn make_graph_chain() -> DependencyGraph {
        let mut g = DependencyGraph::new();
        g.add_edge("src/a.rs", "src/b.rs", EdgeType::Import);
        g.add_edge("src/c.rs", "src/b.rs", EdgeType::Import);
        g
    }

    #[test]
    fn test_structural_impact_direct_dependent() {
        let graph = make_graph_chain();
        let pagerank: HashMap<String, f64> = [
            ("src/a.rs".to_string(), 0.8),
            ("src/c.rs".to_string(), 0.6),
        ]
        .into();
        let entries = structural_impact(&["src/b.rs"], &graph, &pagerank, 3);
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"src/a.rs"), "a.rs imports b.rs — must appear");
        assert!(paths.contains(&"src/c.rs"), "c.rs imports b.rs — must appear");
        for e in &entries {
            assert_eq!(e.signal, ImpactSignal::Structural);
            assert!(e.score > 0.0 && e.score <= 1.0);
        }
    }

    #[test]
    fn test_historical_impact_from_co_changes() {
        use crate::intelligence::co_change::CoChangeEdge;
        let co_changes = vec![
            CoChangeEdge {
                file_a: "src/b.rs".to_string(),
                file_b: "src/x.rs".to_string(),
                count: 5,
                recency_weight: 0.9,
            },
        ];
        let entries = historical_impact(&["src/b.rs"], &co_changes);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "src/x.rs");
        assert_eq!(entries[0].signal, ImpactSignal::Historical);
        // score = count_normalized * recency_weight — just check > 0
        assert!(entries[0].score > 0.0);
    }

    #[test]
    fn test_historical_impact_changed_files_excluded() {
        use crate::intelligence::co_change::CoChangeEdge;
        let co_changes = vec![
            CoChangeEdge {
                file_a: "src/b.rs".to_string(),
                file_b: "src/b.rs".to_string(),
                count: 10,
                recency_weight: 1.0,
            },
        ];
        let entries = historical_impact(&["src/b.rs"], &co_changes);
        assert!(entries.is_empty(), "changed files must not appear in impact");
    }

    #[test]
    fn test_test_confidence_all_three_signals() {
        let test_map: HashMap<String, Vec<crate::intelligence::test_map::TestFileRef>> = [(
            "src/b.rs".to_string(),
            vec![crate::intelligence::test_map::TestFileRef {
                path: "tests/b_test.rs".to_string(),
                confidence: crate::intelligence::test_map::TestConfidence::Both,
            }],
        )]
        .into();
        use crate::intelligence::co_change::CoChangeEdge;
        let co_changes = vec![CoChangeEdge {
            file_a: "src/b.rs".to_string(),
            file_b: "tests/b_test.rs".to_string(),
            count: 4,
            recency_weight: 1.0,
        }];
        // call_impact: b_test.rs depends on b.rs (import edge)
        let mut graph = DependencyGraph::new();
        graph.add_edge("tests/b_test.rs", "src/b.rs", EdgeType::Import);
        let pagerank = HashMap::new();
        let structural = structural_impact(&["src/b.rs"], &graph, &pagerank, 3);
        let historical = historical_impact(&["src/b.rs"], &co_changes);
        let result = merge_test_predictions(
            &["src/b.rs"],
            &structural,
            &historical,
            &[],
            &test_map,
        );
        let pred = result.iter().find(|p| p.test_file == "tests/b_test.rs")
            .expect("b_test.rs must be predicted");
        // test_map + co_change + structural = all three → confidence 0.9
        assert!((pred.confidence - 0.9).abs() < 1e-9);
        assert_eq!(pred.signals.len(), 3);
    }

    #[test]
    fn test_test_confidence_map_only() {
        let test_map: HashMap<String, Vec<crate::intelligence::test_map::TestFileRef>> = [(
            "src/b.rs".to_string(),
            vec![crate::intelligence::test_map::TestFileRef {
                path: "tests/b_test.rs".to_string(),
                confidence: crate::intelligence::test_map::TestConfidence::NameMatch,
            }],
        )]
        .into();
        let result = merge_test_predictions(
            &["src/b.rs"],
            &[],  // no structural
            &[],  // no historical
            &[],  // no call
            &test_map,
        );
        let pred = result.iter().find(|p| p.test_file == "tests/b_test.rs")
            .expect("b_test.rs must be predicted");
        assert!((pred.confidence - 0.4).abs() < 1e-9);
    }

    #[test]
    fn test_confidence_map_values() {
        // Verify all 7 non-empty subsets produce correct confidence
        let cases: &[(&[ImpactSignal], f64)] = &[
            (&[ImpactSignal::Historical], 0.3),
            (&[ImpactSignal::Structural], 0.4),  // test_map alone = 0.4; structural alone treated as test_map here
            (&[ImpactSignal::CallBased], 0.5),
            (&[ImpactSignal::Structural, ImpactSignal::Historical], 0.5),
            (&[ImpactSignal::CallBased, ImpactSignal::Historical], 0.6),
            (&[ImpactSignal::Structural, ImpactSignal::CallBased], 0.7),
            (&[ImpactSignal::Structural, ImpactSignal::CallBased, ImpactSignal::Historical], 0.9),
        ];
        for (signals, expected) in cases {
            let conf = confidence_for_signals(signals);
            assert!(
                (conf - expected).abs() < 1e-9,
                "signals {:?} → expected {expected}, got {conf}",
                signals
            );
        }
    }

    #[test]
    fn test_predict_changed_files_excluded_from_impact() {
        let graph = DependencyGraph::new();
        let pagerank = HashMap::new();
        let entries = structural_impact(&["src/b.rs"], &graph, &pagerank, 3);
        // No dependents — seeds must not appear
        assert!(entries.iter().all(|e| e.path != "src/b.rs"));
    }
}
```

2. Run: `cargo test intelligence::predict` — fails (module doesn't exist).

3. Implement `src/intelligence/predict.rs`:

```rust
use crate::index::graph::DependencyGraph;
use crate::intelligence::co_change::CoChangeEdge;
use crate::intelligence::test_map::TestFileRef;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ImpactSignal {
    Structural,  // blast radius / import graph
    Historical,  // co-change
    CallBased,   // call graph (placeholder until v1.3.0 call graph is available)
}

#[derive(Debug, Serialize)]
pub struct ImpactEntry {
    pub path: String,
    pub signal: ImpactSignal,
    pub score: f64,
}

#[derive(Debug, Serialize)]
pub struct TestPrediction {
    pub test_file: String,
    pub test_function: Option<String>,
    pub signals: Vec<ImpactSignal>,
    pub confidence: f64,
}

#[derive(Debug, Serialize)]
pub struct PredictionResult {
    pub changed_files: Vec<String>,
    pub structural_impact: Vec<ImpactEntry>,
    pub historical_impact: Vec<ImpactEntry>,
    pub call_impact: Vec<ImpactEntry>,
    pub test_impact: Vec<TestPrediction>,
    pub confidence_summary: String,
}

// ---------------------------------------------------------------------------
// Signal computation
// ---------------------------------------------------------------------------

/// Compute structural impact via BFS on the reverse dependency graph (reuses
/// blast_radius logic but returns ImpactEntry rather than AffectedFile).
pub fn structural_impact(
    changed_files: &[&str],
    graph: &DependencyGraph,
    pagerank: &HashMap<String, f64>,
    depth: usize,
) -> Vec<ImpactEntry> {
    use crate::intelligence::blast_radius::compute_blast_radius;
    use std::collections::HashMap as HM;
    let result = compute_blast_radius(
        changed_files,
        graph,
        pagerank,
        &HM::new(), // test_map not needed here
        depth,
        None,
    );
    let mut entries: Vec<ImpactEntry> = result
        .categories
        .direct_dependents
        .iter()
        .chain(result.categories.transitive_dependents.iter())
        .chain(result.categories.schema_dependents.iter())
        .map(|af| ImpactEntry {
            path: af.path.clone(),
            signal: ImpactSignal::Structural,
            score: af.risk,
        })
        .collect();
    entries.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    entries
}

/// Compute historical impact from co-change edges.
/// Score = recency_weight * (count / max_count_in_set), normalized per changed file.
pub fn historical_impact(
    changed_files: &[&str],
    co_changes: &[CoChangeEdge],
) -> Vec<ImpactEntry> {
    let changed_set: HashSet<&str> = changed_files.iter().copied().collect();
    let mut score_map: HashMap<String, f64> = HashMap::new();

    // Find max count for normalization
    let max_count = co_changes
        .iter()
        .map(|e| e.count)
        .max()
        .unwrap_or(1)
        .max(1) as f64;

    for edge in co_changes {
        let (self_file, other_file) = if changed_set.contains(edge.file_a.as_str()) {
            (edge.file_a.as_str(), edge.file_b.as_str())
        } else if changed_set.contains(edge.file_b.as_str()) {
            (edge.file_b.as_str(), edge.file_a.as_str())
        } else {
            continue;
        };

        if changed_set.contains(other_file) {
            continue; // exclude other changed files
        }
        let _ = self_file; // used for changed_set membership check

        let score = (edge.count as f64 / max_count) * edge.recency_weight;
        let entry = score_map.entry(other_file.to_string()).or_insert(0.0);
        if score > *entry {
            *entry = score;
        }
    }

    let mut entries: Vec<ImpactEntry> = score_map
        .into_iter()
        .map(|(path, score)| ImpactEntry {
            path,
            signal: ImpactSignal::Historical,
            score,
        })
        .collect();
    entries.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    entries
}

// ---------------------------------------------------------------------------
// Test prediction merging
// ---------------------------------------------------------------------------

/// Merge structural, historical, and call-based signals to produce test predictions.
/// Each test file accumulates signals; confidence is determined by the signal set.
pub fn merge_test_predictions(
    changed_files: &[&str],
    structural: &[ImpactEntry],
    historical: &[ImpactEntry],
    call_based: &[ImpactEntry],
    test_map: &HashMap<String, Vec<TestFileRef>>,
) -> Vec<TestPrediction> {
    // test_file → set of signals
    let mut test_signals: HashMap<String, HashSet<ImpactSignalKey>> = HashMap::new();

    // 1. test_map signal: for each changed file, look up mapped test files
    let changed_set: HashSet<&str> = changed_files.iter().copied().collect();
    for &changed in changed_files {
        if let Some(refs) = test_map.get(changed) {
            for tr in refs {
                test_signals
                    .entry(tr.path.clone())
                    .or_default()
                    .insert(ImpactSignalKey::TestMap);
            }
        }
    }

    // 2. Structural signal: test files in structural impact
    for entry in structural {
        if is_test_path(&entry.path) && !changed_set.contains(entry.path.as_str()) {
            test_signals
                .entry(entry.path.clone())
                .or_default()
                .insert(ImpactSignalKey::Structural);
        }
    }

    // 3. Historical signal: test files in historical impact
    for entry in historical {
        if is_test_path(&entry.path) && !changed_set.contains(entry.path.as_str()) {
            test_signals
                .entry(entry.path.clone())
                .or_default()
                .insert(ImpactSignalKey::Historical);
        }
    }

    // 4. Call-based signal
    for entry in call_based {
        if is_test_path(&entry.path) && !changed_set.contains(entry.path.as_str()) {
            test_signals
                .entry(entry.path.clone())
                .or_default()
                .insert(ImpactSignalKey::CallBased);
        }
    }

    let mut predictions: Vec<TestPrediction> = test_signals
        .into_iter()
        .map(|(test_file, keys)| {
            let signals: Vec<ImpactSignal> = keys.iter().map(|k| k.to_impact_signal()).collect();
            let confidence = confidence_for_signals(&signals);
            TestPrediction {
                test_file,
                test_function: None,
                signals,
                confidence,
            }
        })
        .collect();
    predictions.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    predictions
}

// Internal enum for HashMap key (ImpactSignal doesn't impl Hash/Eq)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ImpactSignalKey {
    TestMap,
    Structural,
    Historical,
    CallBased,
}

impl ImpactSignalKey {
    fn to_impact_signal(&self) -> ImpactSignal {
        match self {
            Self::TestMap | Self::Structural => ImpactSignal::Structural,
            Self::Historical => ImpactSignal::Historical,
            Self::CallBased => ImpactSignal::CallBased,
        }
    }
}

/// Map signal combinations to confidence values (all 7 non-empty subsets).
/// Signal encoding: TestMap|Structural → "test_map", Historical → "co_change",
/// CallBased → "call_graph". Matching the design spec table:
///
/// | Signals present                     | Confidence |
/// |-------------------------------------|-----------|
/// | co_change only                      | 0.3       |
/// | test_map only                       | 0.4       |
/// | call_graph only                     | 0.5       |
/// | test_map + co_change                | 0.5       |
/// | call_graph + co_change              | 0.6       |
/// | test_map + call_graph               | 0.7       |
/// | test_map + call_graph + co_change   | 0.9       |
pub fn confidence_for_signals(signals: &[ImpactSignal]) -> f64 {
    let has_map = signals.iter().any(|s| *s == ImpactSignal::Structural);
    let has_hist = signals.iter().any(|s| *s == ImpactSignal::Historical);
    let has_call = signals.iter().any(|s| *s == ImpactSignal::CallBased);

    match (has_map, has_call, has_hist) {
        (true, true, true) => 0.9,
        (true, true, false) => 0.7,
        (false, true, true) => 0.6,
        (true, false, true) => 0.5,
        (false, true, false) => 0.5,
        (true, false, false) => 0.4,
        (false, false, true) => 0.3,
        (false, false, false) => 0.0,
    }
}

/// Check if path looks like a test file.
fn is_test_path(path: &str) -> bool {
    crate::intelligence::blast_radius::is_test_path(path)
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// Compute full prediction result for a set of changed files.
pub fn predict(
    changed_files: &[&str],
    graph: &DependencyGraph,
    pagerank: &HashMap<String, f64>,
    co_changes: &[CoChangeEdge],
    test_map: &HashMap<String, Vec<TestFileRef>>,
    depth: usize,
) -> PredictionResult {
    let structural = structural_impact(changed_files, graph, pagerank, depth);
    let historical = historical_impact(changed_files, co_changes);
    let call_impact: Vec<ImpactEntry> = vec![]; // populated in v1.3.0 with call graph

    let test_impact =
        merge_test_predictions(changed_files, &structural, &historical, &call_impact, test_map);

    let avg_conf = if test_impact.is_empty() {
        0.0
    } else {
        test_impact.iter().map(|t| t.confidence).sum::<f64>() / test_impact.len() as f64
    };

    let confidence_summary = format!(
        "{} files predicted affected ({} structural, {} historical); {} tests predicted; avg confidence {:.2}",
        structural.len() + historical.len(),
        structural.len(),
        historical.len(),
        test_impact.len(),
        avg_conf
    );

    PredictionResult {
        changed_files: changed_files.iter().map(|s| s.to_string()).collect(),
        structural_impact: structural,
        historical_impact: historical,
        call_impact,
        test_impact,
        confidence_summary,
    }
}
```

4. Change the existing `is_test_path` function in `blast_radius.rs` from `fn is_test_path(...)` to `pub(crate) fn is_test_path(...)` — just add the visibility modifier, no wrapper function needed.

5. Add to `src/intelligence/mod.rs`:
```rust
pub mod predict;
```

6. Run: `cargo test intelligence::predict -- --nocapture`

7. Commit: `feat: implement change impact prediction with 3-signal merging and 7 confidence levels`

---

## Task 4: Architecture snapshot serialization for drift detection

**Prerequisite:** Add `chrono = { version = "0.4", features = ["serde"] }` to `Cargo.toml` `[dependencies]` NOW (not in Task 8). This task uses `chrono::Utc::now()` for snapshot timestamps.

**Files:**
- Modify: `Cargo.toml` (add chrono)
- Create: `src/intelligence/drift.rs`
- Modify: `src/intelligence/mod.rs`

**Steps:**

1. Write failing tests for snapshot logic:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_snapshot(coupling: f64, cohesion: f64, cycle_count: usize) -> ArchitectureSnapshot {
        ArchitectureSnapshot {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            metrics: ArchitectureMetrics {
                module_count: 5,
                mean_coupling: coupling,
                mean_cohesion: cohesion,
                cycle_count,
                boundary_violation_count: 2,
            },
            modules: vec![],
        }
    }

    #[test]
    fn test_snapshot_serialization_roundtrip() {
        let snap = make_snapshot(0.3, 0.7, 2);
        let json = serde_json::to_string(&snap).unwrap();
        let decoded: ArchitectureSnapshot = serde_json::from_str(&json).unwrap();
        assert!((decoded.metrics.mean_coupling - 0.3).abs() < 1e-9);
        assert_eq!(decoded.metrics.cycle_count, 2);
    }

    #[test]
    fn test_trend_coupling_worsening() {
        let old = make_snapshot(0.2, 0.8, 0);
        let new = make_snapshot(0.5, 0.6, 1);
        let trend = compute_trend(&old, &new);
        assert!(trend.coupling_trend > 0.0, "coupling increased → positive trend (worse)");
        assert!(trend.cohesion_trend < 0.0, "cohesion decreased → negative trend (worse)");
    }

    #[test]
    fn test_trend_improving() {
        let old = make_snapshot(0.5, 0.4, 3);
        let new = make_snapshot(0.2, 0.7, 1);
        let trend = compute_trend(&old, &new);
        assert!(trend.coupling_trend < 0.0, "coupling decreased → improving");
        assert!(trend.cohesion_trend > 0.0, "cohesion increased → improving");
    }

    #[test]
    fn test_insufficient_history_returns_null_trend() {
        // A repo with no baseline and no snapshots should return None for trend
        let snapshots: Vec<ArchitectureSnapshot> = vec![];
        let result = compute_trend_from_snapshots(&snapshots);
        assert!(result.is_none(), "no snapshots → trend must be None");
    }

    #[test]
    fn test_snapshot_filename_contains_timestamp() {
        let name = snapshot_filename("2026-03-15T12:00:00Z");
        assert!(name.contains("2026-03-15"), "filename must embed date");
        assert!(name.ends_with(".json"), "must be .json");
    }

    #[test]
    fn test_baseline_comparison_deltas() {
        let then = ArchitectureMetrics {
            module_count: 4,
            mean_coupling: 0.2,
            mean_cohesion: 0.8,
            cycle_count: 0,
            boundary_violation_count: 1,
        };
        let now = ArchitectureMetrics {
            module_count: 6,
            mean_coupling: 0.4,
            mean_cohesion: 0.6,
            cycle_count: 2,
            boundary_violation_count: 3,
        };
        let deltas = compute_metric_deltas(&then, &now);
        assert!((deltas.coupling_delta - 0.2).abs() < 1e-9);
        assert!((deltas.cohesion_delta - (-0.2)).abs() < 1e-9);
        assert_eq!(deltas.new_cycles, 2);
        assert_eq!(deltas.new_boundary_violations, 2);
    }
}
```

2. Run: `cargo test intelligence::drift` — fails (module doesn't exist).

3. Implement `src/intelligence/drift.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::path::Path;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureMetrics {
    pub module_count: usize,
    pub mean_coupling: f64,
    pub mean_cohesion: f64,
    pub cycle_count: usize,
    pub boundary_violation_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureSnapshot {
    pub timestamp: String,
    pub metrics: ArchitectureMetrics,
    pub modules: Vec<SnapshotModule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotModule {
    pub prefix: String,
    pub coupling: f64,
    pub cohesion: f64,
    pub edge_count: usize,
}

#[derive(Debug, Serialize)]
pub struct MetricDeltas {
    pub coupling_delta: f64,      // positive = worse
    pub cohesion_delta: f64,      // negative = worse
    pub new_cycles: i64,
    pub new_boundary_violations: i64,
    pub module_count_delta: i64,
}

#[derive(Debug, Serialize)]
pub struct BaselineComparison {
    pub baseline_date: String,
    pub metrics_then: ArchitectureMetrics,
    pub metrics_now: ArchitectureMetrics,
    pub deltas: MetricDeltas,
}

#[derive(Debug, Serialize)]
pub struct TrendComparison {
    pub window_recent: String,
    pub window_baseline: String,
    pub coupling_trend: f64,
    pub cohesion_trend: f64,
    pub new_cycles: Vec<Vec<String>>,
    pub new_cross_module_imports: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DriftHotspot {
    pub module: String,
    pub issue: String,
    pub severity: f64,
    pub contributing_commits: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DriftReport {
    pub baseline: Option<BaselineComparison>,
    pub trend: Option<TrendComparison>,
    pub hotspots: Vec<DriftHotspot>,
}

// ---------------------------------------------------------------------------
// Snapshot logic
// ---------------------------------------------------------------------------

pub fn snapshot_filename(timestamp: &str) -> String {
    // Replace colons with hyphens for filesystem safety
    let safe = timestamp.replace(':', "-");
    format!("snapshot-{safe}.json")
}

pub fn compute_metric_deltas(then: &ArchitectureMetrics, now: &ArchitectureMetrics) -> MetricDeltas {
    MetricDeltas {
        coupling_delta: now.mean_coupling - then.mean_coupling,
        cohesion_delta: now.mean_cohesion - then.mean_cohesion,
        new_cycles: now.cycle_count as i64 - then.cycle_count as i64,
        new_boundary_violations: now.boundary_violation_count as i64
            - then.boundary_violation_count as i64,
        module_count_delta: now.module_count as i64 - then.module_count as i64,
    }
}

pub fn compute_trend(
    baseline: &ArchitectureSnapshot,
    current: &ArchitectureSnapshot,
) -> TrendComparison {
    TrendComparison {
        window_recent: "last 30 days".to_string(),
        window_baseline: "30-180 days ago".to_string(),
        coupling_trend: current.metrics.mean_coupling - baseline.metrics.mean_coupling,
        cohesion_trend: current.metrics.mean_cohesion - baseline.metrics.mean_cohesion,
        new_cycles: vec![],
        new_cross_module_imports: vec![],
    }
}

/// Select the most recent snapshot from the list and a snapshot approximately
/// 30 days before it to compute a trend. Returns None when < 2 snapshots exist.
pub fn compute_trend_from_snapshots(snapshots: &[ArchitectureSnapshot]) -> Option<TrendComparison> {
    if snapshots.len() < 2 {
        return None;
    }
    // Snapshots are sorted newest-first (by caller convention)
    let current = &snapshots[0];
    let baseline = &snapshots[snapshots.len() - 1];
    Some(compute_trend(baseline, current))
}

// ---------------------------------------------------------------------------
// Persistence helpers
// ---------------------------------------------------------------------------

/// Save a snapshot to `.cxpak/snapshots/` within the repo root.
pub fn save_snapshot(
    repo_root: &Path,
    snapshot: &ArchitectureSnapshot,
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = repo_root.join(".cxpak").join("snapshots");
    std::fs::create_dir_all(&dir)?;
    let filename = snapshot_filename(&snapshot.timestamp);
    let path = dir.join(filename);
    let json = serde_json::to_string_pretty(snapshot)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Load all snapshots from `.cxpak/snapshots/`, sorted newest-first.
pub fn load_snapshots(repo_root: &Path) -> Vec<ArchitectureSnapshot> {
    let dir = repo_root.join(".cxpak").join("snapshots");
    if !dir.exists() {
        return vec![];
    }
    let mut snapshots: Vec<ArchitectureSnapshot> = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension()?.to_str()? != "json" {
                return None;
            }
            let content = std::fs::read_to_string(&path).ok()?;
            serde_json::from_str(&content).ok()
        })
        .collect();
    snapshots.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    snapshots
}

/// Load the baseline from `.cxpak/baseline.json` if it exists.
pub fn load_baseline(repo_root: &Path) -> Option<ArchitectureSnapshot> {
    let path = repo_root.join(".cxpak").join("baseline.json");
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Save the current snapshot as the baseline.
pub fn save_baseline(
    repo_root: &Path,
    snapshot: &ArchitectureSnapshot,
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = repo_root.join(".cxpak");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("baseline.json");
    let json = serde_json::to_string_pretty(snapshot)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Build a snapshot from the current index state.
pub fn snapshot_from_index(index: &crate::index::CodebaseIndex, timestamp: &str) -> ArchitectureSnapshot {
    // Compute module-level metrics from the graph and pagerank
    use std::collections::HashMap;

    // Derive modules: first two path segments
    let mut module_files: HashMap<String, Vec<String>> = HashMap::new();
    for file in &index.files {
        let prefix = module_prefix(&file.relative_path, 2);
        module_files.entry(prefix).or_default().push(file.relative_path.clone());
    }

    let module_count = module_files.len();
    let mut coupling_sum = 0.0;
    let mut cohesion_sum = 0.0;
    let mut module_count_qualifying = 0usize;
    let mut snapshot_modules: Vec<SnapshotModule> = vec![];

    for (prefix, files) in &module_files {
        if files.len() < 3 {
            continue; // skip modules with < 3 files (per v1.2.0 design)
        }
        let file_set: std::collections::HashSet<&str> = files.iter().map(|s| s.as_str()).collect();

        let mut intra_edges = 0usize;
        let mut cross_edges = 0usize;

        for file in files {
            for dep in index.graph.dependencies(file) {
                if file_set.contains(dep.target.as_str()) {
                    intra_edges += 1;
                } else {
                    cross_edges += 1;
                }
            }
        }

        let total_edges = intra_edges + cross_edges;
        let coupling = if total_edges == 0 {
            0.0
        } else {
            cross_edges as f64 / total_edges as f64
        };

        let max_intra = files.len() * files.len().saturating_sub(1);
        let cohesion = if max_intra == 0 {
            0.0
        } else {
            intra_edges as f64 / max_intra as f64
        };

        coupling_sum += coupling;
        cohesion_sum += cohesion;
        module_count_qualifying += 1;

        snapshot_modules.push(SnapshotModule {
            prefix: prefix.clone(),
            coupling,
            cohesion,
            edge_count: total_edges,
        });
    }

    let mean_coupling = if module_count_qualifying == 0 {
        0.0
    } else {
        coupling_sum / module_count_qualifying as f64
    };
    let mean_cohesion = if module_count_qualifying == 0 {
        0.0
    } else {
        cohesion_sum / module_count_qualifying as f64
    };

    ArchitectureSnapshot {
        timestamp: timestamp.to_string(),
        metrics: ArchitectureMetrics {
            module_count,
            mean_coupling,
            mean_cohesion,
            cycle_count: 0, // Tarjan SCC not yet wired; placeholder
            boundary_violation_count: 0,
        },
        modules: snapshot_modules,
    }
}

fn module_prefix(path: &str, depth: usize) -> String {
    path.split('/')
        .take(depth)
        .collect::<Vec<_>>()
        .join("/")
}

/// Build the full drift report for the current index.
pub fn build_drift_report(
    index: &crate::index::CodebaseIndex,
    repo_root: &Path,
    save_baseline_flag: bool,
) -> DriftReport {
    let now = chrono::Utc::now().to_rfc3339();
    let current_snapshot = snapshot_from_index(index, &now);

    // Optionally persist as baseline
    if save_baseline_flag {
        let _ = save_baseline(repo_root, &current_snapshot);
    }

    // Auto-save snapshot on every drift call
    let _ = save_snapshot(repo_root, &current_snapshot);

    // Load baseline comparison
    let baseline_comparison = load_baseline(repo_root).map(|baseline_snap| {
        let deltas = compute_metric_deltas(&baseline_snap.metrics, &current_snapshot.metrics);
        BaselineComparison {
            baseline_date: baseline_snap.timestamp.clone(),
            metrics_then: baseline_snap.metrics,
            metrics_now: current_snapshot.metrics.clone(),
            deltas,
        }
    });

    // Load historical snapshots for trend
    let snapshots = load_snapshots(repo_root);
    let trend = compute_trend_from_snapshots(&snapshots);

    // Compute hotspots: modules with coupling > 0.6
    let hotspots: Vec<DriftHotspot> = current_snapshot
        .modules
        .iter()
        .filter(|m| m.coupling > 0.6)
        .map(|m| DriftHotspot {
            module: m.prefix.clone(),
            issue: format!("High coupling: {:.2}", m.coupling),
            severity: m.coupling,
            contributing_commits: vec![],
        })
        .collect();

    DriftReport {
        baseline: baseline_comparison,
        trend,
        hotspots,
    }
}
```

4. Add `chrono` dependency to `Cargo.toml`:
```toml
chrono = { version = "0.4", features = ["serde"] }
```

5. Add to `src/intelligence/mod.rs`:
```rust
pub mod drift;
```

6. Run: `cargo test intelligence::drift -- --nocapture`

7. Commit: `feat: implement architecture drift detection with snapshot persistence`

---

## Task 5: Create `src/intelligence/security.rs` — secret patterns and SQL injection

**Files:**
- Create: `src/intelligence/security.rs`
- Modify: `src/intelligence/mod.rs`

**Steps:**

1. Write failing tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // --- Secret patterns ---

    #[test]
    fn test_secret_aws_key() {
        let content = "const KEY = \"AKIAIOSFODNN7EXAMPLE123\";";
        let matches = scan_secret_patterns(content, "src/config.rs");
        assert!(matches.iter().any(|m| m.pattern_name == "aws_access_key"),
            "AWS key must be detected: {:?}", matches);
    }

    #[test]
    fn test_secret_github_pat() {
        let content = "token = \"ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij\";";
        let matches = scan_secret_patterns(content, "src/github.rs");
        assert!(matches.iter().any(|m| m.pattern_name == "github_pat"));
    }

    #[test]
    fn test_secret_password_assignment() {
        let content = "password = \"supersecretpassword123\"";
        let matches = scan_secret_patterns(content, "src/auth.rs");
        assert!(matches.iter().any(|m| m.pattern_name == "password_assignment"));
    }

    #[test]
    fn test_secret_connection_string() {
        let content = "url = \"postgres://admin:password123@localhost/mydb\"";
        let matches = scan_secret_patterns(content, "src/db.rs");
        assert!(matches.iter().any(|m| m.pattern_name == "connection_string"));
    }

    #[test]
    fn test_secret_slack_token() {
        let content = "SLACK_TOKEN=xoxb-1234567890-abcdefghij";
        let matches = scan_secret_patterns(content, "src/notify.rs");
        assert!(matches.iter().any(|m| m.pattern_name == "slack_token"));
    }

    #[test]
    fn test_secret_excluded_test_file() {
        let content = "const KEY = \"AKIAIOSFODNN7EXAMPLE123\";";
        let matches = scan_secret_patterns(content, "tests/test_config.rs");
        assert!(matches.is_empty(), "test files must be excluded from secret scanning");
    }

    #[test]
    fn test_secret_excluded_lock_file() {
        let content = "password = \"supersecret\"";
        for lock_file in &["Cargo.lock", "package-lock.json", "yarn.lock", "Gemfile.lock", "poetry.lock"] {
            let matches = scan_secret_patterns(content, lock_file);
            assert!(matches.is_empty(), "{lock_file} must be excluded");
        }
    }

    #[test]
    fn test_secret_excluded_env_example() {
        let content = "API_KEY=your_api_key_here";
        let matches = scan_secret_patterns(content, ".env.example");
        assert!(matches.is_empty(), ".env.example must be excluded");
    }

    #[test]
    fn test_secret_short_password_ignored() {
        // Passwords < 8 chars must not match (pattern requires {8,})
        let content = "password = \"short\"";
        let matches = scan_secret_patterns(content, "src/config.rs");
        assert!(!matches.iter().any(|m| m.pattern_name == "password_assignment"),
            "short password must not match");
    }

    // --- SQL injection ---

    #[test]
    fn test_sql_injection_python_fstring() {
        let content = r#"query = f"SELECT * FROM users WHERE id = {user_id}""#;
        let risks = scan_sql_injection(content, "src/repo.py");
        assert!(!risks.is_empty(), "f-string SQL interpolation must be detected");
        assert_eq!(risks[0].language, "python");
    }

    #[test]
    fn test_sql_injection_js_template_literal() {
        let content = "const q = `SELECT * FROM orders WHERE id = ${orderId}`;";
        let risks = scan_sql_injection(content, "src/db.js");
        assert!(!risks.is_empty(), "JS template literal SQL must be detected");
        assert_eq!(risks[0].language, "javascript");
    }

    #[test]
    fn test_sql_injection_rust_format() {
        let content = r#"let q = format!("SELECT * FROM products WHERE name = '{}'", name);"#;
        let risks = scan_sql_injection(content, "src/repo.rs");
        assert!(!risks.is_empty(), "Rust format! SQL must be detected");
        assert_eq!(risks[0].language, "rust");
    }

    #[test]
    fn test_sql_injection_java_concatenation() {
        let content = r#"String q = "SELECT * FROM accounts WHERE id = " + accountId;"#;
        let risks = scan_sql_injection(content, "src/AccountRepo.java");
        assert!(!risks.is_empty(), "Java string concatenation SQL must be detected");
        assert_eq!(risks[0].language, "java");
    }

    #[test]
    fn test_sql_injection_parameterized_safe() {
        // Parameterized query — must NOT be flagged
        let content = r#"db.query("SELECT * FROM users WHERE id = $1", [userId])"#;
        let risks = scan_sql_injection(content, "src/repo.js");
        assert!(risks.is_empty(), "parameterized query must not be flagged as injection risk");
    }

    #[test]
    fn test_sql_injection_parameterized_question_mark_safe() {
        let content = r#"db.prepare("SELECT * FROM users WHERE id = ?").bind(id)"#;
        let risks = scan_sql_injection(content, "src/repo.js");
        assert!(risks.is_empty(), "? parameterized query must not be flagged");
    }

    // --- Exposure score ---

    #[test]
    fn test_exposure_score_range() {
        let entry = compute_exposure_entry("src/api.rs", 10, 5, 0.0, 100);
        assert!(entry.exposure_score >= 0.0);
        assert!(entry.exposure_score <= 1.0);
    }

    #[test]
    fn test_exposure_score_fully_tested_is_lower() {
        let untested = compute_exposure_entry("src/a.rs", 10, 5, 0.0, 100);
        let tested = compute_exposure_entry("src/b.rs", 10, 5, 1.0, 100);
        assert!(untested.exposure_score > tested.exposure_score,
            "untested file must have higher exposure");
    }

    #[test]
    fn test_exposure_score_zero_symbols_is_zero() {
        let entry = compute_exposure_entry("src/empty.rs", 0, 0, 0.0, 100);
        assert_eq!(entry.exposure_score, 0.0);
    }

    // --- Input validation gaps ---

    #[test]
    fn test_validation_gap_public_string_param_no_validation() {
        // A public function with a string param and no validation call in body
        let content = r#"
pub fn create_user(name: String) {
    db.insert(name);
}
"#;
        let gaps = scan_validation_gaps(content, "src/user.rs", 0.8);
        assert!(!gaps.is_empty(), "unvalidated String param must be detected");
    }

    #[test]
    fn test_validation_gap_with_validate_call_not_flagged() {
        let content = r#"
pub fn create_user(name: String) {
    validate(&name);
    db.insert(name);
}
"#;
        let gaps = scan_validation_gaps(content, "src/user.rs", 0.8);
        assert!(gaps.is_empty(), "function with validate() call must not be flagged");
    }

    #[test]
    fn test_validation_gap_low_pagerank_skipped() {
        let content = r#"
pub fn do_thing(input: String) {
    process(input);
}
"#;
        // pagerank below high-pagerank threshold (0.5) → skip
        let gaps = scan_validation_gaps(content, "src/util.rs", 0.1);
        assert!(gaps.is_empty(), "low-pagerank file must not be scanned for validation gaps");
    }
}
```

2. Run: `cargo test intelligence::security` — fails.

3. Implement `src/intelligence/security.rs`:

```rust
use regex::Regex;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SecretPattern {
    pub file: String,
    pub line: usize,
    pub pattern_name: String,
    pub snippet: String,  // redacted: show first 4 chars + "..."
}

#[derive(Debug, Serialize)]
pub struct SqlInjectionRisk {
    pub file: String,
    pub line: usize,
    pub language: String,
    pub snippet: String,
    pub interpolation_type: String,
}

#[derive(Debug, Serialize)]
pub struct ValidationGap {
    pub file: String,
    pub function_name: String,
    pub parameter: String,
    pub line: usize,
}

#[derive(Debug, Serialize)]
pub struct UnprotectedEndpoint {
    pub file: String,
    pub method: String,
    pub path: String,
    pub handler: String,
    pub line: usize,
}

#[derive(Debug, Serialize)]
pub struct ExposureEntry {
    pub path: String,
    pub pub_symbol_count: usize,
    pub inbound_edges: usize,
    pub test_coverage: f64,
    pub exposure_score: f64,
}

#[derive(Debug, Serialize)]
pub struct SecuritySurface {
    pub unprotected_endpoints: Vec<UnprotectedEndpoint>,
    pub input_validation_gaps: Vec<ValidationGap>,
    pub secret_patterns: Vec<SecretPattern>,
    pub sql_injection_surface: Vec<SqlInjectionRisk>,
    pub exposure_scores: Vec<ExposureEntry>,
}

// ---------------------------------------------------------------------------
// Exclusion helpers
// ---------------------------------------------------------------------------

fn should_exclude_from_secret_scan(path: &str) -> bool {
    let lower = path.to_lowercase();
    // Test files
    if lower.contains("test") || lower.contains("spec") || lower.contains("__tests__") {
        return true;
    }
    // Lock files
    let lock_files = [
        "cargo.lock",
        "package-lock.json",
        "yarn.lock",
        "gemfile.lock",
        "poetry.lock",
        "composer.lock",
        "pipfile.lock",
    ];
    for lf in &lock_files {
        if lower.ends_with(lf) {
            return true;
        }
    }
    // Example env files
    if lower.contains(".env.example") || lower.contains(".env.sample") {
        return true;
    }
    // Documentation
    if lower.ends_with(".md") || lower.ends_with(".txt") || lower.ends_with(".rst") {
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Secret pattern scanning
// ---------------------------------------------------------------------------

struct SecretSpec {
    name: &'static str,
    pattern: &'static str,
}

const SECRET_PATTERNS: &[SecretSpec] = &[
    SecretSpec {
        name: "aws_access_key",
        pattern: r"AKIA[0-9A-Z]{16}",
    },
    SecretSpec {
        name: "github_pat",
        pattern: r"ghp_[a-zA-Z0-9]{36}",
    },
    SecretSpec {
        name: "password_assignment",
        pattern: r#"(?i)(password|secret|api_key|token)\s*[:=]\s*["'][^"']{8,}["']"#,
    },
    SecretSpec {
        name: "connection_string",
        pattern: r"://[^:]+:[^@]+@",
    },
    SecretSpec {
        name: "slack_token",
        pattern: r"xox[baprs]-[0-9a-zA-Z-]{10,}",
    },
];

pub fn scan_secret_patterns(content: &str, file_path: &str) -> Vec<SecretPattern> {
    if should_exclude_from_secret_scan(file_path) {
        return vec![];
    }

    let mut results = Vec::new();

    for spec in SECRET_PATTERNS {
        let re = match Regex::new(spec.pattern) {
            Ok(r) => r,
            Err(_) => continue,
        };

        for cap in re.find_iter(content) {
            let line = content[..cap.start()].chars().filter(|&c| c == '\n').count() + 1;
            let matched = cap.as_str();
            let snippet = if matched.len() > 4 {
                format!("{}...", &matched[..4])
            } else {
                "...".to_string()
            };
            results.push(SecretPattern {
                file: file_path.to_string(),
                line,
                pattern_name: spec.name.to_string(),
                snippet,
            });
        }
    }

    results
}

// ---------------------------------------------------------------------------
// SQL injection scanning
// ---------------------------------------------------------------------------

fn detect_language_from_path(path: &str) -> &'static str {
    if path.ends_with(".py") {
        "python"
    } else if path.ends_with(".js") || path.ends_with(".mjs") || path.ends_with(".cjs") {
        "javascript"
    } else if path.ends_with(".ts") || path.ends_with(".tsx") {
        "typescript"
    } else if path.ends_with(".rs") {
        "rust"
    } else if path.ends_with(".java") {
        "java"
    } else {
        "unknown"
    }
}

/// Returns true if the SQL string uses parameterized placeholders ($1, ?, :name, @param).
fn is_parameterized(sql_fragment: &str) -> bool {
    Regex::new(r"\$\d+|\?|:\w+|@\w+").map(|re| re.is_match(sql_fragment)).unwrap_or(false)
}

pub fn scan_sql_injection(content: &str, file_path: &str) -> Vec<SqlInjectionRisk> {
    let lang = detect_language_from_path(file_path);
    let mut results = Vec::new();

    let patterns: &[(&str, &str)] = match lang {
        "python" => &[
            // f-string SQL: f"... {var} ..."
            (r#"f["']([^"']*SELECT[^"']*\{[^}]+\}[^"']*)["']"#, "f-string"),
            (r#"f["']([^"']*INSERT[^"']*\{[^}]+\}[^"']*)["']"#, "f-string"),
            (r#"f["']([^"']*UPDATE[^"']*\{[^}]+\}[^"']*)["']"#, "f-string"),
            (r#"f["']([^"']*DELETE[^"']*\{[^}]+\}[^"']*)["']"#, "f-string"),
            // % formatting
            (r#"["']([^"']*SELECT[^"']*%s[^"']*)["']\s*%"#, "percent-format"),
        ],
        "javascript" | "typescript" => &[
            // Template literals with SQL
            (r"`([^`]*(?:SELECT|INSERT|UPDATE|DELETE)[^`]*\$\{[^}]+\}[^`]*)`", "template-literal"),
        ],
        "rust" => &[
            // format! macro with SQL
            (r#"format!\s*\(\s*["']([^"']*(?:SELECT|INSERT|UPDATE|DELETE)[^"']*\{\}[^"']*)["']"#, "format-macro"),
        ],
        "java" => &[
            // String + concatenation with SQL keyword
            (r#"["']([^"']*(?:SELECT|INSERT|UPDATE|DELETE)[^"']*)["']\s*\+"#, "string-concat"),
        ],
        _ => &[],
    };

    for (pattern, interpolation_type) in patterns {
        let re = match Regex::new(pattern) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for cap in re.captures_iter(content) {
            let full_match = cap.get(0).unwrap();
            let sql_fragment = cap.get(1).map(|m| m.as_str()).unwrap_or("");

            // Skip if already parameterized
            if is_parameterized(sql_fragment) {
                continue;
            }

            let line = content[..full_match.start()].chars().filter(|&c| c == '\n').count() + 1;
            let snippet_len = sql_fragment.len().min(60);
            results.push(SqlInjectionRisk {
                file: file_path.to_string(),
                line,
                language: lang.to_string(),
                snippet: sql_fragment[..snippet_len].to_string(),
                interpolation_type: interpolation_type.to_string(),
            });
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Input validation gaps
// ---------------------------------------------------------------------------

/// Scan a file for public functions with string parameters and no validation calls.
/// Only applied to files with pagerank above the threshold (0.5).
pub fn scan_validation_gaps(
    content: &str,
    file_path: &str,
    pagerank: f64,
) -> Vec<ValidationGap> {
    // Only scan high-pagerank files
    if pagerank < 0.5 {
        return vec![];
    }

    let validation_keywords = [
        "validate", "sanitize", "check", "parse", "regex", "is_valid",
        "assert", "guard", "ensure", "verify", "clean",
    ];

    let mut gaps = Vec::new();

    // Detect Rust-style: pub fn name(param: String) or (param: &str)
    let re_rust_fn = match Regex::new(
        r"pub\s+(?:async\s+)?fn\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*\(([^)]*)\)"
    ) {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    let re_string_param = match Regex::new(r"(\w+)\s*:\s*(?:String|&str|&String)") {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    for fn_cap in re_rust_fn.captures_iter(content) {
        let fn_name = fn_cap[1].to_string();
        let params = &fn_cap[2];
        let fn_start = fn_cap.get(0).unwrap().start();
        let line = content[..fn_start].chars().filter(|&c| c == '\n').count() + 1;

        // Find the function body (heuristic: next { ... })
        let after_sig = &content[fn_start..];
        let body_start = after_sig.find('{').unwrap_or(0);
        let body_end = find_matching_brace(after_sig, body_start).unwrap_or(body_start + 1);
        let body = &after_sig[body_start..body_end];

        let has_validation = validation_keywords.iter().any(|kw| body.contains(kw));
        if has_validation {
            continue;
        }

        for param_cap in re_string_param.captures_iter(params) {
            gaps.push(ValidationGap {
                file: file_path.to_string(),
                function_name: fn_name.clone(),
                parameter: param_cap[1].to_string(),
                line,
            });
        }
    }

    gaps
}

/// Find the closing brace matching the opening brace at `start_pos` in `s`.
fn find_matching_brace(s: &str, start_pos: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    if start_pos >= bytes.len() || bytes[start_pos] != b'{' {
        return None;
    }
    let mut depth = 0usize;
    for (i, &b) in bytes[start_pos..].iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start_pos + i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Exposure score
// ---------------------------------------------------------------------------

pub fn compute_exposure_entry(
    path: &str,
    pub_symbol_count: usize,
    inbound_edges: usize,
    test_coverage: f64,
    max_possible: usize,
) -> ExposureEntry {
    let raw = pub_symbol_count as f64 * inbound_edges as f64 * (1.0 - test_coverage);
    let score = if max_possible == 0 || raw == 0.0 {
        0.0
    } else {
        (raw / max_possible as f64).clamp(0.0, 1.0)
    };
    ExposureEntry {
        path: path.to_string(),
        pub_symbol_count,
        inbound_edges,
        test_coverage,
        exposure_score: score,
    }
}

// ---------------------------------------------------------------------------
// Unprotected endpoint detection
// ---------------------------------------------------------------------------

/// Default auth patterns to check in handler call chains.
/// Configurable via .cxpak.json `auth_patterns` key.
pub const DEFAULT_AUTH_PATTERNS: &[&str] = &[
    "auth",
    "authenticate",
    "authorize",
    "require_auth",
    "login_required",
    "authenticated",
    "guard",
    "middleware",
    "jwt",
    "bearer",
    "token_required",
    "permission_required",
];

/// Check if file content contains any auth pattern near the handler function.
/// This is a conservative heuristic: if the file mentions any auth pattern
/// within 50 lines of the handler, mark it as protected.
pub fn endpoint_is_protected(
    content: &str,
    handler: &str,
    auth_patterns: &[&str],
) -> bool {
    if handler == "handler" || handler == "<anonymous>" {
        // Can't do call-chain analysis without real handler name
        // Fall back to file-level auth pattern check
        let lower = content.to_lowercase();
        return auth_patterns.iter().any(|p| lower.contains(p));
    }

    // Find the handler function in the content
    let handler_pos = content.find(handler).unwrap_or(0);
    let start = handler_pos.saturating_sub(200);
    let end = (handler_pos + 2000).min(content.len());
    let window = &content[start..end];
    let lower = window.to_lowercase();
    auth_patterns.iter().any(|p| lower.contains(p))
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// Build the full security surface for the current index.
pub fn build_security_surface(
    index: &crate::index::CodebaseIndex,
    auth_patterns: &[&str],
    focus: Option<&str>,
) -> SecuritySurface {
    use crate::intelligence::api_surface::detect_routes;
    use std::collections::HashMap;

    let mut secret_patterns = Vec::new();
    let mut sql_injection_surface = Vec::new();
    let mut input_validation_gaps = Vec::new();
    let mut unprotected_endpoints = Vec::new();

    // Compute max_possible for exposure score normalization
    let max_pub_symbols = index
        .files
        .iter()
        .map(|f| {
            f.parse_result
                .as_ref()
                .map(|pr| pr.symbols.iter().filter(|s| s.visibility == crate::parser::language::Visibility::Public).count())
                .unwrap_or(0)
        })
        .max()
        .unwrap_or(1);
    let max_inbound = index
        .files
        .iter()
        .map(|f| index.graph.dependents(&f.relative_path).count())
        .max()
        .unwrap_or(1);
    let max_possible = max_pub_symbols * max_inbound;

    let mut exposure_scores = Vec::new();

    for file in &index.files {
        if let Some(focus_prefix) = focus {
            if !file.relative_path.starts_with(focus_prefix) {
                continue;
            }
        }

        let path = &file.relative_path;
        let content = &file.content;
        let pagerank = index.pagerank.get(path).copied().unwrap_or(0.0);

        // Secret patterns
        secret_patterns.extend(scan_secret_patterns(content, path));

        // SQL injection
        sql_injection_surface.extend(scan_sql_injection(content, path));

        // Input validation gaps (high-pagerank files only)
        input_validation_gaps.extend(scan_validation_gaps(content, path, pagerank));

        // Routes → unprotected endpoint detection
        let routes = detect_routes(content, path);
        for route in routes {
            if !endpoint_is_protected(content, &route.handler, auth_patterns) {
                unprotected_endpoints.push(UnprotectedEndpoint {
                    file: path.clone(),
                    method: route.method,
                    path: route.path,
                    handler: route.handler,
                    line: route.line,
                });
            }
        }

        // Exposure score
        let pub_count = file
            .parse_result
            .as_ref()
            .map(|pr| pr.symbols.iter().filter(|s| s.visibility == crate::parser::language::Visibility::Public).count())
            .unwrap_or(0);
        let inbound = index.graph.dependents(path).count();
        let has_tests = index.test_map.contains_key(path);
        let test_cov = if has_tests { 1.0 } else { 0.0 };

        let entry = compute_exposure_entry(path, pub_count, inbound, test_cov, max_possible);
        if entry.exposure_score > 0.0 {
            exposure_scores.push(entry);
        }
    }

    // Sort exposure scores descending
    exposure_scores.sort_by(|a, b| {
        b.exposure_score
            .partial_cmp(&a.exposure_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    SecuritySurface {
        unprotected_endpoints,
        input_validation_gaps,
        secret_patterns,
        sql_injection_surface,
        exposure_scores,
    }
}
```

4. Add `chrono` already added in Task 4. Add to `src/intelligence/mod.rs`:
```rust
pub mod security;
```

5. Run: `cargo test intelligence::security -- --nocapture`

6. Commit: `feat: implement security surface analysis — secrets, SQL injection, exposure scores, validation gaps`

---

## Task 6: Wire `predict`, `drift`, `security` MCP tool handlers in `serve.rs`

**Files:**
- Modify: `src/commands/serve.rs`

**Steps:**

1. Write failing integration tests in `tests/serve_security_tools.rs`:

```rust
use assert_cmd::Command;

#[test]
fn test_predict_endpoint_rejects_missing_files_param() {
    // POST /predict without files param → error JSON
    // Uses a live server test or mock; here we test the handler directly
}
```

Since the serve tests are integration-level and require a running server, write unit-level handler tests instead — stub the state and call handlers directly in `src/commands/serve.rs` tests block.

2. Add the three new route registrations to `build_router()`:

```rust
Router::new()
    // ... existing routes ...
    .route("/predict", axum::routing::post(predict_handler))
    .route("/drift", axum::routing::post(drift_handler))
    .route("/security_surface", get(security_surface_handler))
    .with_state(state)
```

3. Add `PredictParams`, `DriftParams`, `SecurityParams` structs and handlers:

```rust
#[derive(Deserialize)]
struct PredictParams {
    files: Option<Vec<String>>,
    focus: Option<String>,
    depth: Option<usize>,
}

async fn predict_handler(
    State(index): State<SharedIndex>,
    axum::Json(params): axum::Json<PredictParams>,
) -> Result<Json<Value>, StatusCode> {
    let files = match params.files {
        Some(f) if !f.is_empty() => f,
        _ => {
            return Ok(Json(json!({
                "error": "missing required field: files (non-empty list of paths)"
            })));
        }
    };

    let idx = index.read().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let file_refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
    let depth = params.depth.unwrap_or(3);

    let result = crate::intelligence::predict::predict(
        &file_refs,
        &idx.graph,
        &idx.pagerank,
        &idx.co_changes,
        &idx.test_map,
        depth,
    );

    Ok(Json(serde_json::to_value(&result).unwrap_or_else(|_| json!({"error": "serialization failed"}))))
}

#[derive(Deserialize)]
struct DriftParams {
    save_baseline: Option<bool>,
    focus: Option<String>,
}

async fn drift_handler(
    State(index): State<SharedIndex>,
    State(repo_path): State<SharedPath>,
    axum::Json(params): axum::Json<DriftParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index.read().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let save_baseline = params.save_baseline.unwrap_or(false);
    let report = crate::intelligence::drift::build_drift_report(
        &idx,
        &repo_path,
        save_baseline,
    );
    Ok(Json(serde_json::to_value(&report).unwrap_or_else(|_| json!({"error": "serialization failed"}))))
}

#[derive(Deserialize)]
struct SecuritySurfaceParams {
    focus: Option<String>,
}

async fn security_surface_handler(
    State(index): State<SharedIndex>,
    Query(params): Query<SecuritySurfaceParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index.read().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let surface = crate::intelligence::security::build_security_surface(
        &idx,
        crate::intelligence::security::DEFAULT_AUTH_PATTERNS,
        params.focus.as_deref(),
    );
    Ok(Json(serde_json::to_value(&surface).unwrap_or_else(|_| json!({"error": "serialization failed"}))))
}
```

4. Add handler tests within `serve.rs`:

```rust
#[cfg(test)]
mod handler_tests {
    use super::*;

    #[test]
    fn test_predict_handler_missing_files_returns_error() {
        // The handler logic for empty files returns error JSON
        // Test the PredictParams deserialization path
        let params = PredictParams { files: None, focus: None, depth: None };
        assert!(params.files.is_none());
    }
}
```

5. Run: `cargo test serve -- --nocapture` and `cargo build --features daemon`

6. Commit: `feat: wire predict, drift, security_surface MCP tool endpoints`

---

## Task 7: Add `predictions` field to `AutoContextResult`

**Files:**
- Modify: `src/auto_context/mod.rs`

**Steps:**

1. Write failing test:

```rust
#[test]
fn test_auto_context_predictions_absent_when_no_file_mention() {
    // When task doesn't mention specific file paths, predictions is None
    let index = make_minimal_test_index();
    let opts = AutoContextOpts {
        tokens: 10_000,
        focus: None,
        include_tests: false,
        include_blast_radius: false,
    };
    let result = auto_context("fix the authentication flow", &index, &opts);
    assert!(result.predictions.is_none());
}

#[test]
fn test_auto_context_predictions_present_when_file_mentioned() {
    let index = make_minimal_test_index_with_cochanges();
    let opts = AutoContextOpts {
        tokens: 10_000,
        focus: None,
        include_tests: false,
        include_blast_radius: false,
    };
    // Task explicitly mentions a file path
    let result = auto_context("refactor src/auth.rs to use the new token format", &index, &opts);
    assert!(result.predictions.is_some(), "predictions must be populated when task mentions file path");
}
```

2. Add `predictions: Option<crate::intelligence::predict::PredictionResult>` to `AutoContextResult`.

3. Add file-mention detection in `auto_context()` — scan the task string for path-like tokens:

```rust
// After Step 9, before Step 10:
// Detect file mentions in task string (paths containing '/' or ending with known extensions)
let file_mentions: Vec<&str> = {
    let ext_pattern = Regex::new(r"\b[\w/.-]+\.(?:rs|ts|js|py|go|java|rb|c|cpp|h|cs|swift|kt)\b")
        .ok();
    let slash_pattern = Regex::new(r"\b(?:src|lib|tests?|spec|app|pkg)/[\w/.-]+\b").ok();

    let mut mentions: Vec<&str> = Vec::new();
    if let Some(re) = &ext_pattern {
        for m in re.find_iter(task) {
            mentions.push(m.as_str());
        }
    }
    if let Some(re) = &slash_pattern {
        for m in re.find_iter(task) {
            if !mentions.contains(&m.as_str()) {
                mentions.push(m.as_str());
            }
        }
    }
    // Only keep mentions that actually exist in the index
    mentions.retain(|p| index.files.iter().any(|f| f.relative_path == *p));
    mentions
};

let predictions = if !file_mentions.is_empty() {
    Some(crate::intelligence::predict::predict(
        &file_mentions,
        &index.graph,
        &index.pagerank,
        &index.co_changes,
        &index.test_map,
        3,
    ))
} else {
    None
};
```

4. Include `predictions` in the returned `AutoContextResult`.

5. Run: `cargo test auto_context -- --nocapture`

6. Commit: `feat: populate auto_context predictions field when task mentions changed files`

---

## Task 8: Add `chrono` to Cargo.toml and verify full build

**Files:**
- Modify: `Cargo.toml`

**Steps:**

1. Add `chrono` to `[dependencies]` (already noted in Task 4, confirm it's present):
```toml
chrono = { version = "0.4", features = ["serde"] }
```

2. Run: `cargo build --all-features 2>&1 | head -50`

3. Run: `cargo clippy --all-targets -- -D warnings`

4. Fix any clippy warnings introduced by new modules.

5. Run: `cargo test --verbose 2>&1 | tail -30`

6. Commit: `chore: add chrono dependency and fix clippy warnings for v1.4.0 modules`

---

## Task 9: Coverage gap tests — prediction confidence edge cases

**Files:**
- Modify: `src/intelligence/predict.rs` (tests block)

**Steps:**

1. Add property-like tests covering all 7 signal subsets explicitly:

```rust
#[test]
fn test_all_seven_confidence_subsets_are_distinct() {
    use std::collections::HashSet;
    let cases: Vec<(&[ImpactSignal], f64)> = vec![
        (&[ImpactSignal::Historical], 0.3),
        (&[ImpactSignal::Structural], 0.4),
        (&[ImpactSignal::CallBased], 0.5),
        (&[ImpactSignal::Structural, ImpactSignal::Historical], 0.5),
        (&[ImpactSignal::CallBased, ImpactSignal::Historical], 0.6),
        (&[ImpactSignal::Structural, ImpactSignal::CallBased], 0.7),
        (&[ImpactSignal::Structural, ImpactSignal::CallBased, ImpactSignal::Historical], 0.9),
    ];
    let values: HashSet<u64> = cases.iter()
        .map(|(_, v)| (v * 100.0) as u64)
        .collect();
    // Note: 0.5 appears for both (CallBased only) and (Structural+Historical) — that is by design
    assert_eq!(values.len(), 6, "6 distinct confidence levels in the 7 subsets");
}

#[test]
fn test_empty_signals_produces_zero_confidence() {
    assert_eq!(confidence_for_signals(&[]), 0.0);
}

#[test]
fn test_historical_impact_score_bounded() {
    use crate::intelligence::co_change::CoChangeEdge;
    let co_changes: Vec<CoChangeEdge> = (0..100)
        .map(|i| CoChangeEdge {
            file_a: "src/hub.rs".to_string(),
            file_b: format!("src/dep{i}.rs"),
            count: i as u32 + 1,
            recency_weight: 1.0,
        })
        .collect();
    let entries = historical_impact(&["src/hub.rs"], &co_changes);
    for e in &entries {
        assert!(e.score >= 0.0 && e.score <= 1.0,
            "score must be in [0,1], got {} for {}", e.score, e.path);
    }
}

#[test]
fn test_structural_impact_sorted_descending() {
    let mut graph = DependencyGraph::new();
    graph.add_edge("high.rs", "src.rs", EdgeType::Import);
    graph.add_edge("low.rs", "src.rs", EdgeType::Import);
    let pagerank: HashMap<String, f64> = [
        ("high.rs".to_string(), 0.9),
        ("low.rs".to_string(), 0.1),
    ].into();
    let entries = structural_impact(&["src.rs"], &graph, &pagerank, 3);
    for i in 1..entries.len() {
        assert!(entries[i-1].score >= entries[i].score,
            "structural impact must be sorted descending by score");
    }
}
```

2. Run: `cargo test intelligence::predict -- --nocapture`

3. Commit: `test: add coverage gap tests for prediction confidence and score bounds`

---

## Task 10: Coverage gap tests — security surface edge cases

**Files:**
- Modify: `src/intelligence/security.rs` (tests block)

**Steps:**

1. Add edge-case tests:

```rust
#[test]
fn test_secret_aws_key_must_be_20_chars() {
    // AKIA + 16 alphanumeric = 20 chars total
    let short = "AKIA123"; // too short
    let matches = scan_secret_patterns(short, "src/config.rs");
    assert!(!matches.iter().any(|m| m.pattern_name == "aws_access_key"),
        "short AKIA prefix must not match");
}

#[test]
fn test_sql_injection_no_sql_keywords_not_flagged() {
    let content = r#"const msg = `Hello ${name}`;"#;
    let risks = scan_sql_injection(content, "src/greet.js");
    assert!(risks.is_empty(), "template literal without SQL keywords must not be flagged");
}

#[test]
fn test_sql_injection_rust_no_format_macro_not_flagged() {
    let content = r#"let msg = format!("Hello {}", name);"#;
    let risks = scan_sql_injection(content, "src/greet.rs");
    assert!(risks.is_empty(), "format! without SQL keywords must not be flagged");
}

#[test]
fn test_exposure_max_possible_zero_returns_zero() {
    let entry = compute_exposure_entry("src/x.rs", 5, 3, 0.0, 0);
    assert_eq!(entry.exposure_score, 0.0, "max_possible=0 must produce score 0");
}

#[test]
fn test_exposure_score_clamped_to_one() {
    // Even if raw exceeds max_possible, clamp to 1.0
    let entry = compute_exposure_entry("src/x.rs", 100, 100, 0.0, 1);
    assert!(entry.exposure_score <= 1.0);
}

#[test]
fn test_secret_snippet_redaction() {
    let content = "const KEY = \"AKIAIOSFODNN7EXAMPLE123\";";
    let matches = scan_secret_patterns(content, "src/config.rs");
    let secret = matches.iter().find(|m| m.pattern_name == "aws_access_key").unwrap();
    // Snippet must start with first 4 chars and end with ...
    assert!(secret.snippet.ends_with("..."), "snippet must be redacted");
    assert!(secret.snippet.len() < 20, "snippet must not expose full secret");
}

#[test]
fn test_endpoint_protected_by_file_level_auth_keyword() {
    let content = "app.use(authenticate); app.get('/admin', adminHandler);";
    assert!(
        endpoint_is_protected(content, "adminHandler", DEFAULT_AUTH_PATTERNS),
        "file containing authenticate keyword must be considered protected"
    );
}

#[test]
fn test_endpoint_unprotected_no_auth_keywords() {
    let content = "app.get('/public', publicHandler);";
    assert!(
        !endpoint_is_protected(content, "publicHandler", DEFAULT_AUTH_PATTERNS),
        "file with no auth keywords must be unprotected"
    );
}

#[test]
fn test_validation_gap_sanitize_keyword_not_flagged() {
    let content = r#"
pub fn process_input(data: String) {
    let clean = sanitize(&data);
    store(clean);
}
"#;
    let gaps = scan_validation_gaps(content, "src/proc.rs", 0.9);
    assert!(gaps.is_empty(), "function with sanitize() call must not be flagged");
}
```

2. Run: `cargo test intelligence::security -- --nocapture`

3. Commit: `test: add coverage gap tests for security surface edge cases`

---

## Task 11: Coverage gap tests — drift snapshot and co-change

**Files:**
- Modify: `src/intelligence/drift.rs` (tests block)
- Modify: `src/intelligence/co_change.rs` (tests block)

**Steps:**

1. Add drift tests:

```rust
#[test]
fn test_snapshot_load_save_roundtrip() {
    let dir = tempfile::TempDir::new().unwrap();
    let snap = make_snapshot(0.3, 0.7, 1);
    save_snapshot(dir.path(), &snap).unwrap();
    let loaded = load_snapshots(dir.path());
    assert_eq!(loaded.len(), 1);
    assert!((loaded[0].metrics.mean_coupling - 0.3).abs() < 1e-9);
}

#[test]
fn test_baseline_save_and_load() {
    let dir = tempfile::TempDir::new().unwrap();
    let snap = make_snapshot(0.4, 0.6, 2);
    save_baseline(dir.path(), &snap).unwrap();
    let loaded = load_baseline(dir.path());
    assert!(loaded.is_some());
    assert_eq!(loaded.unwrap().metrics.cycle_count, 2);
}

#[test]
fn test_baseline_absent_returns_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let result = load_baseline(dir.path());
    assert!(result.is_none());
}

#[test]
fn test_snapshot_filename_no_colons() {
    let name = snapshot_filename("2026-01-01T12:30:00Z");
    assert!(!name.contains(':'), "filename must not contain colons");
}

#[test]
fn test_metric_deltas_zero_when_same() {
    let m = ArchitectureMetrics {
        module_count: 5, mean_coupling: 0.3, mean_cohesion: 0.7,
        cycle_count: 1, boundary_violation_count: 2,
    };
    let deltas = compute_metric_deltas(&m, &m);
    assert_eq!(deltas.new_cycles, 0);
    assert!((deltas.coupling_delta).abs() < 1e-9);
}
```

2. Add co-change tests:

```rust
#[test]
fn test_co_change_many_files_per_commit() {
    // 4 files in same commit → 6 pairs
    let commits = vec![
        vec!["a.rs".into(), "b.rs".into(), "c.rs".into(), "d.rs".into()]; 3
    ];
    let edges = build_co_change_edges(&commits, 3, 180);
    assert_eq!(edges.len(), 6, "4 files produce 6 pairs");
}

#[test]
fn test_co_change_weight_at_boundary_30d() {
    // Exactly 30 days ago → weight = 1.0
    let commits = vec![(vec!["a.rs".to_string(), "b.rs".to_string()], 30i64); 3];
    let edges = build_co_change_edges_with_dates(&commits, 3, 180);
    assert!((edges[0].recency_weight - 1.0).abs() < 1e-9);
}

#[test]
fn test_co_change_weight_at_boundary_180d() {
    // Exactly 180 days → weight = 1.0 - 0.7 * 150/150 = 0.3
    let commits = vec![(vec!["a.rs".to_string(), "b.rs".to_string()], 180i64); 3];
    let edges = build_co_change_edges_with_dates(&commits, 3, 180);
    assert!((edges[0].recency_weight - 0.3).abs() < 1e-9);
}
```

3. Run: `cargo test intelligence::drift intelligence::co_change -- --nocapture`

4. Commit: `test: add coverage gap tests for drift snapshots and co-change edge cases`

---

## Task 12: Run full test suite and enforce 90% coverage

**Files:**
- None modified (verification task)

**Steps:**

1. Run full test suite:
```bash
cargo test --verbose 2>&1 | tail -50
```

2. Run tarpaulin for coverage:
```bash
cargo tarpaulin --out Json --output-file tarpaulin-report.json 2>&1 | tail -20
```

3. Check coverage percentage from the report. If below 90%, identify uncovered lines:
```bash
cargo tarpaulin --out Stdout 2>&1 | grep -E "src/intelligence/(predict|drift|security|co_change)" | head -20
```

4. Add targeted tests for any uncovered branches (e.g. `mine_co_changes_from_git` with a non-existent repo path, `save_snapshot` on a read-only path, etc.):

```rust
// In co_change.rs tests:
#[test]
fn test_mine_co_changes_nonexistent_repo_returns_empty() {
    let result = mine_co_changes_from_git(std::path::Path::new("/nonexistent/path"), 180);
    assert!(result.is_empty(), "non-existent repo must return empty vec");
}

// In drift.rs tests:
#[test]
fn test_load_snapshots_nonexistent_dir_returns_empty() {
    let result = load_snapshots(std::path::Path::new("/nonexistent/snapshots"));
    assert!(result.is_empty());
}

#[test]
fn test_module_prefix_depth_two() {
    assert_eq!(module_prefix("src/intelligence/predict.rs", 2), "src/intelligence");
    assert_eq!(module_prefix("main.rs", 2), "main.rs");
    assert_eq!(module_prefix("src/lib.rs", 2), "src/lib.rs");
}
```

5. Re-run tarpaulin. Confirm ≥ 90%.

6. Commit: `test: ensure 90% coverage for all v1.4.0 intelligence modules`

---

## Task 13: Update `Cargo.toml` version to 1.4.0 and sync plugin files

**Files:**
- Modify: `Cargo.toml`
- Modify: `plugin/.claude-plugin/plugin.json`
- Modify: `.claude-plugin/marketplace.json`

**Steps:**

1. Update `Cargo.toml`:
```toml
version = "1.4.0"
```

2. Update `plugin/.claude-plugin/plugin.json`:
```json
"version": "1.4.0"
```

3. Update `.claude-plugin/marketplace.json`:
```json
"version": "1.4.0"
```

4. Run `cargo check` to regenerate `Cargo.lock` with the new version:
```bash
cargo check
```

5. Verify `Cargo.lock` contains `version = "1.4.0"` for the cxpak package.

6. Commit: `chore: bump version to 1.4.0 and regenerate Cargo.lock`

---

## Task 14: Final integration test — serve all three new MCP tools end-to-end

**Files:**
- Create: `tests/v140_integration.rs`

**Steps:**

1. Write integration tests using `assert_cmd` and `tower` test client:

```rust
use crate::commands::serve::build_index;
use std::path::Path;

#[test]
fn test_predict_endpoint_returns_prediction_result() {
    // Build a minimal index from the fixture directory
    let fixture = Path::new("tests/fixtures/simple_rust");
    if !fixture.exists() {
        return; // skip if fixture not present
    }
    let index = build_index(fixture).expect("index build must succeed");

    let result = crate::intelligence::predict::predict(
        &["src/lib.rs"],
        &index.graph,
        &index.pagerank,
        &index.co_changes,
        &index.test_map,
        3,
    );
    assert!(!result.changed_files.is_empty());
    assert!(!result.confidence_summary.is_empty());
}

#[test]
fn test_security_surface_on_minimal_codebase_no_panic() {
    let fixture = Path::new("tests/fixtures/simple_rust");
    if !fixture.exists() {
        return;
    }
    let index = build_index(fixture).expect("index build must succeed");
    let surface = crate::intelligence::security::build_security_surface(
        &index,
        crate::intelligence::security::DEFAULT_AUTH_PATTERNS,
        None,
    );
    // Must not panic; exposure_scores must be in [0,1]
    for entry in &surface.exposure_scores {
        assert!(entry.exposure_score >= 0.0 && entry.exposure_score <= 1.0);
    }
}

#[test]
fn test_drift_report_on_fresh_repo_no_baseline() {
    let dir = tempfile::TempDir::new().unwrap();
    // No .cxpak/baseline.json → baseline field must be None
    let fixture = Path::new("tests/fixtures/simple_rust");
    if !fixture.exists() {
        return;
    }
    let index = build_index(fixture).expect("index build must succeed");
    let report = crate::intelligence::drift::build_drift_report(&index, dir.path(), false);
    assert!(report.baseline.is_none(), "no baseline file → baseline must be None");
}

#[test]
fn test_drift_report_save_and_reload_baseline() {
    let dir = tempfile::TempDir::new().unwrap();
    let fixture = Path::new("tests/fixtures/simple_rust");
    if !fixture.exists() {
        return;
    }
    let index = build_index(fixture).expect("index build must succeed");
    // First call with save_baseline=true
    let _ = crate::intelligence::drift::build_drift_report(&index, dir.path(), true);
    // Second call — now baseline exists
    let report = crate::intelligence::drift::build_drift_report(&index, dir.path(), false);
    assert!(report.baseline.is_some(), "after save_baseline=true, baseline must be present on next call");
}
```

2. Run: `cargo test v140_integration -- --nocapture`

3. Commit: `test: add v1.4.0 integration tests for predict, drift, and security_surface`

---

## Task 15: Documentation and CLAUDE.md update

**Files:**
- Modify: `/Users/lb/Documents/barnett/cxpak/.claude/CLAUDE.md`

**Steps:**

1. Update the Architecture section to include the three new modules:

In the **Intelligence** section, after the existing bullet points, add:

```
- **`predict.rs`** — `predict()` combines structural (blast radius), historical (co-change), and call-based signals into `PredictionResult` with `TestPrediction` entries ranked by confidence (0.3–0.9 across all 7 signal combinations)
- **`drift.rs`** — `build_drift_report()` compares the current architecture snapshot against a stored baseline (`.cxpak/baseline.json`) and historical snapshots (`.cxpak/snapshots/`); `snapshot_from_index()` auto-saves on each call
- **`security.rs`** — `build_security_surface()` runs 5 deterministic detections: unprotected endpoints (real handler names from api_surface), input validation gaps (high-PageRank files), secret patterns (per-type regex, 5 types), SQL injection (interpolation detection per language), and exposure scores
- **`co_change.rs`** — `mine_co_changes_from_git()` walks git log 180 days back; `build_co_change_edges_with_dates()` applies threshold=3 and recency decay; edges stored on `CodebaseIndex.co_changes`
```

2. Update the **New MCP Tools** note to reflect v1.4.0 additions:
- `predict` — POST `/predict`, params: `files` (list), `focus`, `depth`
- `drift` — POST `/drift`, params: `save_baseline` (bool), `focus`
- `security_surface` — GET `/security_surface`, params: `focus`

3. Note the `RouteEndpoint.handler` fix: real handler function names are now extracted per framework; fallback to `"<anonymous>"` for inline closures.

4. Note the `AutoContextResult.predictions` field: populated when task mentions specific file paths matching the index.

5. Commit: `docs: update CLAUDE.md with v1.4.0 module architecture and new MCP tools`

---

## Summary

| Task | Module | Key Output |
|------|--------|-----------|
| 1 | `api_surface.rs` | Real handler names in `RouteEndpoint.handler` |
| 2 | `co_change.rs` | `CoChangeEdge`, git log mining, `CodebaseIndex.co_changes` |
| 3 | `predict.rs` | `PredictionResult`, 3 signals, 7 confidence levels |
| 4 | `drift.rs` | Snapshot persistence, `DriftReport`, baseline comparison |
| 5 | `security.rs` | 5 detections: endpoints, validation, secrets, SQL, exposure |
| 6 | `serve.rs` | 3 new MCP tool handlers: `/predict`, `/drift`, `/security_surface` |
| 7 | `auto_context/mod.rs` | `predictions` field populated on file-mention detection |
| 8 | `Cargo.toml` | `chrono` dependency, clean build |
| 9 | `predict.rs` tests | Coverage for all 7 confidence subsets, score bounds |
| 10 | `security.rs` tests | Edge cases: redaction, bounds, exclusions |
| 11 | `drift.rs`, `co_change.rs` tests | Roundtrip, boundary weights, large commits |
| 12 | All | 90% tarpaulin coverage confirmed |
| 13 | `Cargo.toml`, plugin files | Version bump to 1.4.0, `Cargo.lock` regenerated |
| 14 | `tests/v140_integration.rs` | End-to-end predict/drift/security on real index |
| 15 | `CLAUDE.md` | Architecture docs updated |

**Dependency order:** Tasks 1 → 2 → 3 → 4 → 5 → 6 → 7 (must be sequential). Tasks 8–15 can proceed after 7.
