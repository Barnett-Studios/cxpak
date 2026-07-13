//! Shared test scaffolding (plan node N0).
//!
//! Compiled only under `cfg(test)` (crate-internal unit tests) or the
//! `test-support` feature (integration tests, enabled via the self
//! dev-dependency in `Cargo.toml`). It is never present in a release binary
//! and is never touched by any render path, so it cannot change the emitted
//! SPA bytes or the golden fixture.
//!
//! Every RED acceptance test in the 3.1.0 UI-overhaul MVP compiles against
//! [`index_with`]; the SPA-render nodes additionally share
//! [`render_fixture_spa`].

use crate::conventions::git_health::ChurnEntry;
use crate::index::graph::EdgeType;
use crate::index::{CodebaseIndex, IndexedFile};
use crate::intelligence::co_change::CoChangeEdge;

/// Start a fluent [`IndexBuilder`] that produces a real [`CodebaseIndex`]
/// wired with graph edges, co-change edges, and a rankable risk set — without
/// touching disk or git.
pub fn index_with() -> IndexBuilder {
    IndexBuilder::default()
}

/// Fluent builder for a test [`CodebaseIndex`].
///
/// Edges are added directly to `index.graph` (bypassing import-string
/// resolution) so tests can assert against an exact topology. `build()` never
/// calls `rebuild_graph` (which would discard these manual edges, since the
/// synthetic files carry no `parse_result`).
#[derive(Default)]
pub struct IndexBuilder {
    files: Vec<String>,
    last: Option<String>,
    imports: Vec<(String, String)>,
    cochanges: Vec<(String, String, f64)>,
    churn: Vec<(String, usize)>,
}

impl IndexBuilder {
    fn ensure_file(&mut self, name: &str) {
        if !self.files.iter().any(|f| f == name) {
            self.files.push(name.to_string());
        }
    }

    /// Add a file and make it the target of the next `.imports(..)`.
    pub fn file(mut self, name: &str) -> Self {
        self.ensure_file(name);
        self.last = Some(name.to_string());
        self
    }

    /// Record an Import edge from the most recently `.file(..)`d node to
    /// `target` (auto-creating `target`). Does not change the current file, so
    /// `.file("a").imports("b").imports("c")` gives a→b and a→c.
    pub fn imports(mut self, target: &str) -> Self {
        let from = self
            .last
            .clone()
            .expect("call .file(..) before .imports(..)");
        self.ensure_file(target);
        self.imports.push((from, target.to_string()));
        self
    }

    /// Record a co-change edge (`recency_weight = score`) between `a` and `b`,
    /// auto-creating both files. Does NOT add an Import edge — this is exactly
    /// the raw material for the surprising-connections insight (co-change minus
    /// import).
    pub fn co_change(mut self, a: &str, b: &str, score: f64) -> Self {
        self.ensure_file(a);
        self.ensure_file(b);
        self.cochanges.push((a.to_string(), b.to_string(), score));
        self
    }

    /// Add `a`⇄`b` mutual Import edges, forming a 2-node strongly-connected
    /// component (a cycle) for architecture/cycle tests.
    pub fn with_cycle(mut self, a: &str, b: &str) -> Self {
        self.ensure_file(a);
        self.ensure_file(b);
        self.imports.push((a.to_string(), b.to_string()));
        self.imports.push((b.to_string(), a.to_string()));
        self
    }

    /// Add `n` files with distinct churn and one dependent each, so
    /// `compute_risk_ranking` yields `n` entries with a spread of non-zero
    /// risk scores (`risk = norm_churn × norm_blast × test_penalty`, all files
    /// untested).
    pub fn n_risky_files(mut self, n: usize) -> Self {
        for i in 0..n {
            let f = format!("risky/f{i}.rs");
            let consumer = format!("risky/c{i}.rs");
            self.ensure_file(&f);
            self.ensure_file(&consumer);
            // consumer depends on f → f has a reverse-edge (blast > 0).
            self.imports.push((consumer, f.clone()));
            // distinct churn → distinct norm_churn percentile → risk spread.
            self.churn.push((f, i + 1));
        }
        self
    }

    /// Materialize the [`CodebaseIndex`].
    pub fn build(self) -> CodebaseIndex {
        let mut index = CodebaseIndex::empty();
        index.files = self
            .files
            .iter()
            .map(|name| IndexedFile {
                relative_path: name.clone(),
                language: Some("rust".to_string()),
                size_bytes: 0,
                token_count: 0,
                parse_result: None,
                // No `#[cfg(test)]` marker → `has_inline_tests` is false →
                // every synthetic file is untested (test_penalty = 1.0).
                content: String::new(),
                mtime_secs: None,
            })
            .collect();
        index.total_files = index.files.len();

        for (from, to) in &self.imports {
            index.graph.add_edge(from, to, EdgeType::Import);
        }

        index.conventions.git_health.churn_30d = self
            .churn
            .iter()
            .map(|(path, modifications)| ChurnEntry {
                path: path.clone(),
                modifications: *modifications,
                last_commit_epoch: None,
            })
            .collect();

        index.co_changes = self
            .cochanges
            .iter()
            .map(|(a, b, score)| CoChangeEdge {
                file_a: a.clone(),
                file_b: b.clone(),
                count: 1,
                recency_weight: *score,
            })
            .collect();

        index
    }
}

/// Render the SPA for a small, deterministic fixture repo carrying a real risk
/// spread (files with and without dependents). Shared by the SPA-render RED
/// tests (N5 treemap percentile, N6 palette) so they assert against one
/// fixture instead of redefining ad-hoc ones. Byte-identical across calls (no
/// disk, no clock).
#[cfg(feature = "visual")]
pub fn render_fixture_spa() -> String {
    use crate::budget::counter::TokenCounter;
    use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use crate::visual::render::RenderMetadata;
    use std::collections::HashMap;

    let counter = TokenCounter::new();
    let paths = ["src/main.rs", "src/db.rs", "src/util.rs", "src/leaf.rs"];
    let files: Vec<ScannedFile> = paths
        .iter()
        .map(|p| ScannedFile {
            relative_path: (*p).to_string(),
            absolute_path: format!("/tmp/cxpak-fixture/{p}").into(),
            language: Some("rust".to_string()),
            size_bytes: 64,
        })
        .collect();

    // main → db, util; util → leaf. Gives db/util/leaf non-zero blast and a
    // percentile spread across the four files.
    let mut parse = HashMap::new();
    let import = |src: &str| crate::parser::language::Import {
        source: src.to_string(),
        names: vec![],
    };
    let sym = |name: &str| Symbol {
        name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: Visibility::Public,
        signature: format!("fn {name}()"),
        body: format!("fn {name}() {{}}"),
        start_line: 1,
        end_line: 2,
    };
    parse.insert(
        "src/main.rs".to_string(),
        ParseResult {
            symbols: vec![sym("main")],
            imports: vec![import("crate::db"), import("crate::util")],
            exports: vec![],
        },
    );
    parse.insert(
        "src/util.rs".to_string(),
        ParseResult {
            symbols: vec![sym("util")],
            imports: vec![import("crate::leaf")],
            exports: vec![],
        },
    );

    let content: HashMap<String, String> = paths
        .iter()
        .map(|p| ((*p).to_string(), format!("// {p}\n")))
        .collect();

    let index = CodebaseIndex::build_with_content(files, parse, &counter, content);

    let metadata = RenderMetadata {
        repo_name: "fixture".to_string(),
        generated_at: "2026-07-11T00:00:00Z".to_string(),
        health_score: Some(7.0),
        node_count: index.total_files,
        edge_count: index.graph.edge_count(),
        cxpak_version: "3.1.0".to_string(),
    };

    crate::visual::spa::render_spa(&index, &metadata).expect("fixture SPA renders")
}
