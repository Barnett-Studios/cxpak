//! Regression test: Clojure files must produce call-graph edges.
//!
//! Deployment evidence (ikc3 corpus, 2026-10-30): the daemon indexed 4,136
//! files cleanly yet `POST /call_graph` returned `edges: []` for every
//! Clojure symbol probed, and `/blast_radius` returned `total_affected: 0`
//! for `src/ui/version.clj` — a file with known callers. Two defects
//! combined: `.clj` imports fell through to the legacy resolver (which never
//! tried `.clj` extensions, so the file-level import graph had no Clojure
//! edges), and call-site extraction fell through to the C-shaped
//! `def name(` / `ident(` regex, which can never match Lisp
//! `(defn name …)` / `(callee …)` forms.
//!
//! `file.content` carries the real `.clj` sources so the call-site
//! extraction runs the actual tree-sitter-clojure path; the parse results
//! mirror what `parser::languages::clojure` emits (ns-qualified symbol
//! names, `ns` import sources).
#![cfg(feature = "lang-clojure")]

use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use cxpak::intelligence::call_graph::{build_call_graph, CallConfidence};
use cxpak::parser::language::{Export, Import, ParseResult, Symbol, SymbolKind, Visibility};
use cxpak::scanner::ScannedFile;
use std::collections::HashMap;

fn sym(name: &str, kind: SymbolKind, visibility: Visibility) -> Symbol {
    Symbol {
        name: name.to_string(),
        kind,
        visibility,
        signature: String::new(),
        body: String::new(),
        start_line: 1,
        end_line: 1,
    }
}

fn clojure_index() -> CodebaseIndex {
    let counter = TokenCounter::new();
    let version_src = "(ns ui.version)\n\n(defn server-build-id []\n  (str \"v\" 1))\n";
    let attach_src = "(ns ui.attach\n  (:require [ui.version :as version]))\n\n(defn mount []\n  (version/server-build-id))\n";

    let files = vec![
        ScannedFile {
            relative_path: "src/ui/version.clj".into(),
            absolute_path: "/tmp/src/ui/version.clj".into(),
            language: Some("clojure".into()),
            size_bytes: version_src.len() as u64,
        },
        ScannedFile {
            relative_path: "src/ui/attach.clj".into(),
            absolute_path: "/tmp/src/ui/attach.clj".into(),
            language: Some("clojure".into()),
            size_bytes: attach_src.len() as u64,
        },
    ];

    let mut parse_results = HashMap::new();
    parse_results.insert(
        "src/ui/version.clj".to_string(),
        ParseResult {
            symbols: vec![
                sym("ui.version", SymbolKind::Class, Visibility::Public),
                sym(
                    "ui.version/server-build-id",
                    SymbolKind::Function,
                    Visibility::Public,
                ),
            ],
            imports: vec![],
            exports: vec![
                Export {
                    name: "ui.version".into(),
                    kind: SymbolKind::Class,
                },
                Export {
                    name: "ui.version/server-build-id".into(),
                    kind: SymbolKind::Function,
                },
            ],
        },
    );
    parse_results.insert(
        "src/ui/attach.clj".to_string(),
        ParseResult {
            symbols: vec![
                sym("ui.attach", SymbolKind::Class, Visibility::Public),
                sym("ui.attach/mount", SymbolKind::Function, Visibility::Public),
            ],
            imports: vec![Import {
                source: "ui.version".into(),
                names: vec!["version".into()],
            }],
            exports: vec![
                Export {
                    name: "ui.attach".into(),
                    kind: SymbolKind::Class,
                },
                Export {
                    name: "ui.attach/mount".into(),
                    kind: SymbolKind::Function,
                },
            ],
        },
    );

    let mut content = HashMap::new();
    content.insert("src/ui/version.clj".to_string(), version_src.to_string());
    content.insert("src/ui/attach.clj".to_string(), attach_src.to_string());

    CodebaseIndex::build_with_content(files, parse_results, &counter, content)
}

#[test]
fn clojure_alias_qualified_call_produces_exact_edge() {
    let idx = clojure_index();
    // The namespace import resolves to a file-level edge — the import graph
    // must contain attach → version, or Exact-tier resolution can never fire.
    let cg = build_call_graph(&idx);
    let edge = cg
        .edges
        .iter()
        .find(|e| {
            e.caller_file == "src/ui/attach.clj"
                && e.callee_file == "src/ui/version.clj"
                && e.callee_symbol == "ui.version/server-build-id"
        })
        .unwrap_or_else(|| {
            panic!(
                "expected attach/mount → version/server-build-id edge, got: {:?}",
                cg.edges
            )
        });
    assert_eq!(
        edge.confidence,
        CallConfidence::Exact,
        "import-resolved call should be Exact, not Approximate"
    );
}
