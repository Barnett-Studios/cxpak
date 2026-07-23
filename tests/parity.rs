//! Parity tests: the incremental / warm-started / cached index paths must
//! produce structurally identical results to a full rebuild (within float
//! epsilon for PageRank). These are the definition of done for W1 — never
//! weaken an assertion to make one pass.

use cxpak::index::graph::{DependencyGraph, EdgeType};
use cxpak::intelligence::pagerank::{compute_pagerank, compute_pagerank_seeded};
use std::collections::HashMap;

/// Build a small dependency graph from `(from, to)` import edges.
fn graph_with_edges(edges: &[(&str, &str)]) -> DependencyGraph {
    let mut g = DependencyGraph::new();
    for (from, to) in edges {
        g.add_edge(from, to, EdgeType::Import);
    }
    g
}

fn top_k(m: &HashMap<String, f64>, k: usize) -> Vec<String> {
    let mut v: Vec<_> = m.iter().map(|(k, &s)| (k.clone(), s)).collect();
    // score desc, path asc as a deterministic tiebreak
    v.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    v.into_iter().take(k).map(|(k, _)| k).collect()
}

#[test]
fn warm_pagerank_matches_cold() {
    let g = graph_with_edges(&[("a", "b"), ("b", "c"), ("c", "a"), ("a", "c")]);
    let cold = compute_pagerank(&g, 0.85, 100);
    // Seeding the iteration from the converged cold result must reach the same
    // fixed point (PageRank's stationary distribution is unique for a graph).
    let warm = compute_pagerank_seeded(&g, 0.85, 100, &cold);
    for (k, &cv) in &cold {
        assert!(
            (warm[k] - cv).abs() <= 2e-6,
            "node {k}: warm {} cold {}",
            warm[k],
            cv
        );
    }
    // identical top-K ordering
    assert_eq!(top_k(&cold, 3), top_k(&warm, 3));
}

#[test]
fn empty_seed_equals_cold() {
    // An empty seed must reproduce exactly today's 1/N-initialised behaviour.
    let g = graph_with_edges(&[("a", "b"), ("b", "c"), ("c", "a")]);
    let cold = compute_pagerank(&g, 0.85, 100);
    let seeded_empty = compute_pagerank_seeded(&g, 0.85, 100, &HashMap::new());
    for (k, &cv) in &cold {
        assert!((seeded_empty[k] - cv).abs() <= 1e-12, "node {k} diverged");
    }
}

// ---------------------------------------------------------------------------
// Task 2: edge-delta graph rebuild == full rebuild (ADR-0166)
// ---------------------------------------------------------------------------

use cxpak::index::{CodebaseIndex, IndexedFile};
use cxpak::parser::language::{Import, ParseResult};
use cxpak::schema::{ColumnSchema, ForeignKeyRef, SchemaIndex, TableSchema};
use std::collections::HashSet;

/// A Rust `IndexedFile` at `src/{stem}.rs` importing each `imports` stem via
/// `crate::{stem}` (which `resolve_rust_import` maps back to `src/{stem}.rs`).
fn rust_file(stem: &str, imports: &[&str]) -> std::sync::Arc<IndexedFile> {
    std::sync::Arc::new(IndexedFile {
        relative_path: format!("src/{stem}.rs"),
        language: Some("rust".to_string()),
        size_bytes: 0,
        token_count: 0,
        parse_result: Some(ParseResult {
            symbols: vec![],
            imports: imports
                .iter()
                .map(|t| Import {
                    source: format!("crate::{t}"),
                    names: vec![],
                })
                .collect(),
            exports: vec![],
        }),
        content: String::new(),
        mtime_secs: None,
    })
}

/// Build a fresh `CodebaseIndex` over `files` with a full graph (the oracle).
fn index_from_files(files: Vec<std::sync::Arc<IndexedFile>>) -> CodebaseIndex {
    let mut idx = CodebaseIndex::empty();
    idx.total_files = files.len();
    idx.files = files;
    idx.rebuild_graph();
    idx
}

const UNIVERSE: [&str; 5] = ["f0", "f1", "f2", "f3", "f4"];

proptest::proptest! {
    /// For an arbitrary sequence of content-modifications and removals over a
    /// fixed 5-file universe, the incrementally-maintained graph must equal a
    /// from-scratch full rebuild — edges AND reverse_edges, exactly.
    #[test]
    fn graph_delta_equals_full_rebuild(
        // Each op: (file index, Some(import target indices) = set imports, None = remove).
        ops in proptest::collection::vec(
            (0usize..5, proptest::option::of(proptest::collection::vec(0usize..5, 0..4))),
            1..9,
        )
    ) {
        // state[stem] = Some(import target stems) when present, None when removed.
        let mut state: std::collections::BTreeMap<&str, Option<Vec<&str>>> =
            UNIVERSE.iter().map(|s| (*s, Some(Vec::new()))).collect();

        // Delta index starts as the full base and is maintained incrementally.
        let base_files: Vec<std::sync::Arc<IndexedFile>> =
            UNIVERSE.iter().map(|s| rust_file(s, &[])).collect();
        let mut delta_idx = index_from_files(base_files);

        for (i, action) in &ops {
            let stem = UNIVERSE[*i];
            let path = format!("src/{stem}.rs");
            let mut changed: HashSet<String> = HashSet::new();
            let mut removed: HashSet<String> = HashSet::new();

            match action {
                Some(targets) => {
                    let target_stems: Vec<&str> =
                        targets.iter().map(|t| UNIVERSE[*t]).collect();
                    let file = rust_file(stem, &target_stems);
                    // upsert into delta_idx.files
                    if let Some(slot) =
                        delta_idx.files.iter_mut().find(|f| f.relative_path == path)
                    {
                        *slot = file;
                    } else {
                        delta_idx.files.push(file);
                    }
                    changed.insert(path.clone());
                    state.insert(stem, Some(target_stems));
                }
                None => {
                    let was_present = delta_idx.files.iter().any(|f| f.relative_path == path);
                    delta_idx.files.retain(|f| f.relative_path != path);
                    if was_present {
                        removed.insert(path.clone());
                    }
                    state.insert(stem, None);
                }
            }
            delta_idx.total_files = delta_idx.files.len();
            delta_idx.rebuild_graph_delta(&changed, &removed);
        }

        // Oracle: full rebuild over the final state.
        let final_files: Vec<std::sync::Arc<IndexedFile>> = UNIVERSE
            .iter()
            .filter_map(|s| state[*s].as_ref().map(|imps| rust_file(s, imps)))
            .collect();
        let full_idx = index_from_files(final_files);

        prop_assert_eq!(&delta_idx.graph.edges, &full_idx.graph.edges);
        prop_assert_eq!(&delta_idx.graph.reverse_edges, &full_idx.graph.reverse_edges);
    }
}

use proptest::prelude::*;

/// `incremental_rebuild` (edge-delta + warm PageRank) over a content
/// modification must equal a from-scratch full build — graph edges exactly,
/// PageRank within epsilon. This pins the Task 3 wiring end-to-end.
#[test]
fn incremental_rebuild_matches_full_build() {
    use cxpak::budget::counter::TokenCounter;
    use cxpak::scanner::ScannedFile;

    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let write = |rel: &str, content: &str| -> ScannedFile {
        let abs = dir.path().join(rel.replace('/', "_"));
        std::fs::write(&abs, content).unwrap();
        ScannedFile {
            relative_path: rel.to_string(),
            absolute_path: abs,
            language: Some("rust".to_string()),
            size_bytes: content.len() as u64,
        }
    };
    let pr = |imports: &[&str]| ParseResult {
        symbols: vec![],
        imports: imports
            .iter()
            .map(|s| Import {
                source: (*s).to_string(),
                names: vec![],
            })
            .collect(),
        exports: vec![],
    };

    // Initial state: a imports b; b and c are plain.
    let a = write("src/a.rs", "use crate::b;");
    let b = write("src/b.rs", "pub fn b() {}");
    let c = write("src/c.rs", "pub fn c() {}");
    let mut parses = HashMap::new();
    parses.insert("src/a.rs".to_string(), pr(&["crate::b"]));
    parses.insert("src/b.rs".to_string(), pr(&[]));
    parses.insert("src/c.rs".to_string(), pr(&[]));

    let mut inc = CodebaseIndex::build(
        vec![a.clone(), b.clone(), c.clone()],
        parses.clone(),
        &counter,
    );

    // Modify a to import c instead of b (different size → needs_update fires;
    // a is already a graph node + no schema → exact per-file delta path).
    let a2 = write(
        "src/a.rs",
        "use crate::c; // modified: now imports c instead",
    );
    let mut parses2 = parses.clone();
    parses2.insert("src/a.rs".to_string(), pr(&["crate::c"]));
    inc.incremental_rebuild(&[a2.clone(), b.clone(), c.clone()], &parses2, &counter);

    let full = CodebaseIndex::build(vec![a2, b, c], parses2, &counter);

    assert_eq!(
        inc.graph.edges, full.graph.edges,
        "incremental graph edges must equal full rebuild"
    );
    assert_eq!(inc.graph.reverse_edges, full.graph.reverse_edges);
    // the modified edge actually moved a→b ⇒ a→c
    assert!(inc
        .graph
        .dependencies("src/a.rs")
        .map(|s| s.iter().any(|e| e.target == "src/c.rs"))
        .unwrap_or(false));
    assert_eq!(inc.pagerank.len(), full.pagerank.len());
    for (k, &v) in &full.pagerank {
        assert!(
            (inc.pagerank[k] - v).abs() <= 2e-6,
            "pagerank {k}: inc {} full {}",
            inc.pagerank[k],
            v
        );
    }
}

/// Init a git repo at `dir` with `files` (relative_path, content), committed.
fn init_repo_with_files(dir: &std::path::Path, files: &[(&str, &str)]) {
    let repo = git2::Repository::init(dir).unwrap();
    let sig = git2::Signature::now("Test", "test@test.com").unwrap();
    for (rel, content) in files {
        let abs = dir.join(rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, content).unwrap();
    }
    let mut idx = repo.index().unwrap();
    idx.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    idx.write().unwrap();
    let tree_id = idx.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
        .unwrap();
}

/// Recursively copy a directory tree (used to simulate a clone to a new path).
fn copy_tree(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let target = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

#[test]
fn derived_cache_hit_equals_full_build() {
    use cxpak::commands::serve::build_index;

    let dir = tempfile::TempDir::new().unwrap();
    init_repo_with_files(
        dir.path(),
        &[
            ("src/a.rs", "use crate::b;\npub fn a() {}\n"),
            ("src/b.rs", "pub fn b() {}\n"),
        ],
    );

    let full = build_index(dir.path()).unwrap(); // miss → builds + saves derived cache
    let cached = build_index(dir.path()).unwrap(); // hit → restores from derived cache

    assert_eq!(
        full.graph.edges, cached.graph.edges,
        "cache-hit graph must equal the full build"
    );
    assert_eq!(full.graph.reverse_edges, cached.graph.reverse_edges);
    assert_eq!(full.pagerank.len(), cached.pagerank.len());
    for (k, &v) in &full.pagerank {
        assert!((cached.pagerank[k] - v).abs() <= 2e-6, "pagerank {k}");
    }
    // ConventionProfile / CoChangeEdge have no PartialEq; compare as serde_json
    // Values (whose object Map is a sorted BTreeMap, so HashMap key-ordering in
    // the conventions — e.g. churn_trend — is normalised, comparing data, not
    // serialization order).
    assert_eq!(
        serde_json::to_value(&full.conventions).unwrap(),
        serde_json::to_value(&cached.conventions).unwrap(),
        "restored conventions must equal the mined conventions"
    );
    assert_eq!(
        serde_json::to_value(&full.co_changes).unwrap(),
        serde_json::to_value(&cached.co_changes).unwrap(),
    );
    // The derived cache file was actually written.
    assert!(dir.path().join(".cxpak/cache/root/derived.json").exists());
}

#[test]
fn cache_is_portable_across_paths() {
    use cxpak::cache::{content_fingerprint, DerivedCache};
    use cxpak::commands::serve::build_index;

    // Build in A (writes the derived cache).
    let dir_a = tempfile::TempDir::new().unwrap();
    init_repo_with_files(
        dir_a.path(),
        &[
            ("src/a.rs", "use crate::b;\npub fn a() {}\n"),
            ("src/b.rs", "pub fn b() {}\n"),
        ],
    );
    let index_a = build_index(dir_a.path()).unwrap();

    // The fingerprint A used (content + HEAD), recomputed independently.
    let head_oid = {
        let repo = git2::Repository::discover(dir_a.path()).unwrap();
        let oid = repo.head().unwrap().target().unwrap().to_string();
        oid
    };
    let fp_files: Vec<(String, String)> = index_a
        .files
        .iter()
        .map(|f| (f.relative_path.clone(), f.content.clone()))
        .collect();
    let fingerprint = content_fingerprint(&fp_files, &head_oid);

    // Copy the entire tree (source + .git + .cxpak) to B — a different absolute
    // path, identical content and HEAD, as a clone to another machine would be.
    let dir_b = tempfile::TempDir::new().unwrap();
    copy_tree(dir_a.path(), dir_b.path());

    // The cache written by A must HIT in B under the same content fingerprint —
    // the fingerprint is content-based, so it is path-independent.
    let cache_dir_b = dir_b.path().join(".cxpak/cache/root");
    assert!(
        DerivedCache::load(&cache_dir_b, &fingerprint).is_some(),
        "derived cache must be portable: same content+HEAD must hit at a new path"
    );

    // And a full build in B equals the build in A.
    let index_b = build_index(dir_b.path()).unwrap();
    assert_eq!(index_a.graph.edges, index_b.graph.edges);
}

/// A changed file with a data layer present MUST fall back to a full rebuild so
/// schema-derived edges (here a foreign key) survive — a naive import-only
/// delta would silently drop them. Mandated by the W1 plan's schema boundary.
#[test]
fn delta_with_schema_falls_back_and_preserves_fk_edge() {
    // orders.sql has an FK to customers.sql; app.rs is an unrelated Rust file.
    let files = vec![
        std::sync::Arc::new(IndexedFile {
            relative_path: "src/orders.sql".to_string(),
            language: Some("sql".to_string()),
            size_bytes: 0,
            token_count: 0,
            parse_result: None,
            content: String::new(),
            mtime_secs: None,
        }),
        std::sync::Arc::new(IndexedFile {
            relative_path: "src/customers.sql".to_string(),
            language: Some("sql".to_string()),
            size_bytes: 0,
            token_count: 0,
            parse_result: None,
            content: String::new(),
            mtime_secs: None,
        }),
        rust_file("app", &[]),
    ];

    let mut schema = SchemaIndex {
        tables: std::collections::HashMap::new(),
        views: std::collections::HashMap::new(),
        functions: std::collections::HashMap::new(),
        orm_models: std::collections::HashMap::new(),
        migrations: Vec::new(),
    };
    schema.tables.insert(
        "customers".to_string(),
        TableSchema {
            name: "customers".to_string(),
            columns: vec![],
            primary_key: None,
            indexes: vec![],
            file_path: "src/customers.sql".to_string(),
            start_line: 1,
        },
    );
    schema.tables.insert(
        "orders".to_string(),
        TableSchema {
            name: "orders".to_string(),
            columns: vec![ColumnSchema {
                name: "customer_id".to_string(),
                data_type: "int".to_string(),
                nullable: false,
                default: None,
                constraints: vec![],
                foreign_key: Some(ForeignKeyRef {
                    target_table: "customers".to_string(),
                    target_column: "id".to_string(),
                }),
            }],
            primary_key: None,
            indexes: vec![],
            file_path: "src/orders.sql".to_string(),
            start_line: 1,
        },
    );

    let mut delta_idx = CodebaseIndex::empty();
    delta_idx.total_files = files.len();
    delta_idx.files = files.clone();
    delta_idx.schema = Some(schema.clone());
    delta_idx.rebuild_graph();

    // Sanity: the FK edge exists.
    let fk_present = |idx: &CodebaseIndex| {
        idx.graph
            .dependencies("src/orders.sql")
            .map(|set| set.iter().any(|e| e.target == "src/customers.sql"))
            .unwrap_or(false)
    };
    assert!(
        fk_present(&delta_idx),
        "FK edge must exist after full build"
    );

    // Modify the unrelated Rust file → schema present → must fall back to full
    // rebuild, preserving the FK edge.
    let mut changed = HashSet::new();
    changed.insert("src/app.rs".to_string());
    delta_idx.rebuild_graph_delta(&changed, &HashSet::new());

    let mut full_idx = CodebaseIndex::empty();
    full_idx.total_files = files.len();
    full_idx.files = files;
    full_idx.schema = Some(schema);
    full_idx.rebuild_graph();

    assert!(
        fk_present(&delta_idx),
        "FK edge dropped by delta — fallback failed"
    );
    assert_eq!(delta_idx.graph.edges, full_idx.graph.edges);
    assert_eq!(delta_idx.graph.reverse_edges, full_idx.graph.reverse_edges);
}
