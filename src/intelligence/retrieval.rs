//! Deterministic LSP-first iterative retrieval core (cxpak 3.0.0 Task C1).
//!
//! Three primitives — [`search`], [`references`], [`expand`] — compose into the
//! agent retrieval loop `search → pick a hit → references → expand → repeat`,
//! all answered over cxpak's OWN index (never an external language server). This
//! is the SINGLE source of truth for retrieval: every surface (MCP
//! `cxpak_context` `op=retrieval`, LSP `cxpak/retrieval`, CLI `search`, HTTP
//! `/v1/retrieval`) calls [`execute`] and reshapes the result for transport — no
//! surface re-derives (ADR-0153 single-source invariant; the catalog's
//! `retrieval` capability projects through `capability::adapter`; ADR-0180).
//!
//! # The three ops (each reuses existing machinery — nothing reinvented)
//!
//! * [`search`] — symbol- and content-matching over the index, reusing
//!   [`CodebaseIndex::find_symbol`]-style symbol iteration and
//!   [`CodebaseIndex::find_content_matches`]. Hits are ranked by a match tier,
//!   PageRank-boosted, and returned in an explicit TOTAL ORDER.
//! * [`references`] — where a symbol is referenced across files, reusing
//!   [`build_symbol_cross_refs`] (the `symbol → files` inverted index) with the
//!   SAME [`normalize_identifier`] key `symbol_importance` uses.
//! * [`expand`] — widen a seed set to its bounded neighbourhood, reusing the B1
//!   [`graph_query::subgraph`] primitive verbatim.
//!
//! # Determinism (hard contract, ADR-0180)
//!
//! Every output is byte-deterministic. No `HashMap`/`HashSet` iteration order
//! leaks into any output:
//!
//! * [`search`] iterates `index.files` (a `Vec`, insertion-ordered) and sorts
//!   the collected hits by the total order `(score desc, path, symbol,
//!   start_line, match_kind)`. `score` ties are broken by the string/line keys,
//!   which are themselves total, so equal scores never reorder.
//! * [`references`] collects the matching file set into a `Vec`, then `sort` +
//!   `dedup` — the backing `HashSet` is never iterated into the output.
//! * [`expand`] delegates to `graph_query::subgraph`, already byte-deterministic
//!   (BTree-backed graph, sorted nodes and induced edges).

use crate::core_graph::index::normalize_identifier;
use crate::index::CodebaseIndex;
use crate::intelligence::graph_query::{self, Subgraph};
use crate::intelligence::pagerank::build_symbol_cross_refs;
use crate::parser::language::SymbolKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// Default number of search hits returned when the caller does not pass `limit`.
pub const DEFAULT_SEARCH_LIMIT: usize = 20;

/// PageRank contributes at most this much to a hit's score, so it only ever
/// re-orders WITHIN a match tier and never lets a weaker match overtake a
/// stronger one. Kept `< 1.0` (the gap between adjacent tiers).
const PAGERANK_WEIGHT: f64 = 0.5;

// Match-tier bases. The gaps are `>= 1.0` so `tier + PAGERANK_WEIGHT * pr`
// (with `pr ∈ [0, 1]`) can never cross a tier boundary.
const TIER_EXACT: f64 = 3.0;
const TIER_PREFIX: f64 = 2.0;
const TIER_SUBSTRING: f64 = 1.0;
const TIER_CONTENT: f64 = 0.5;

/// One search hit: either a matching symbol or a content-only file match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    /// Repo-relative path of the file the hit is in.
    pub path: String,
    /// The matched symbol's name, or `None` for a content-only file match.
    pub symbol: Option<String>,
    /// The matched symbol's kind label (lowercased), or `None` for content.
    pub kind: Option<String>,
    /// 1-based line the symbol starts on, or `None` for content-only matches.
    pub start_line: Option<usize>,
    /// `symbol` if a symbol name matched, `content` if only the file body did.
    pub match_kind: String,
    /// Ranking score: match tier + PageRank boost. Higher is more relevant.
    pub score: f64,
}

/// Result of [`search`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    pub query: String,
    pub hits: Vec<SearchHit>,
}

/// Result of [`references`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceResult {
    /// The queried symbol, echoed back verbatim.
    pub symbol: String,
    /// Files that reference the symbol, sorted and de-duplicated.
    pub files: Vec<String>,
}

/// Error from [`execute`] when a request is malformed. Surfaces map these to
/// their own transport errors (HTTP 400, LSP `-32603`, MCP error text) — the
/// same contract as [`graph_query::GraphQueryError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetrievalError {
    /// A required parameter was absent, empty, or the wrong JSON type.
    MissingParam(String),
    /// The `op` selector did not name a known primitive.
    UnknownOp(String),
}

impl fmt::Display for RetrievalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RetrievalError::MissingParam(p) => {
                write!(f, "missing or invalid required parameter: {p}")
            }
            RetrievalError::UnknownOp(op) => write!(
                f,
                "unknown retrieval op `{op}`; expected one of search|references|expand"
            ),
        }
    }
}

impl std::error::Error for RetrievalError {}

/// Lowercased, stable label for a symbol kind (deterministic, from `Debug`).
fn kind_label(kind: &SymbolKind) -> String {
    format!("{kind:?}").to_lowercase()
}

/// PageRank of a file, clamped to `[0, 1]` (defensive; scores are already in
/// range) and `0.0` when the file is absent from the map.
fn file_pagerank(index: &CodebaseIndex, path: &str) -> f64 {
    index
        .pagerank
        .get(path)
        .copied()
        .unwrap_or(0.0)
        .clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

/// `search(query, limit)` — rank symbols and content matching `query` over the
/// index, newest-relevant first, in a fully deterministic total order.
///
/// A symbol matches when its (lowercased) name CONTAINS the (lowercased) query;
/// the tier is `exact` > `prefix` > `substring`. A file additionally yields one
/// `content` hit when its body contains the query but it produced NO symbol hit
/// (so a file never both symbol- and content-matches for the same query). Every
/// hit's score is `tier + PAGERANK_WEIGHT * pagerank(file)`.
pub fn search(index: &CodebaseIndex, query: &str, limit: usize) -> SearchResult {
    let q = query.to_lowercase();
    if q.is_empty() {
        return SearchResult {
            query: query.to_string(),
            hits: Vec::new(),
        };
    }

    let mut hits: Vec<SearchHit> = Vec::new();
    for file in &index.files {
        let pr = file_pagerank(index, &file.relative_path);
        let mut file_had_symbol_hit = false;

        if let Some(parsed) = &file.parse_result {
            for sym in &parsed.symbols {
                let name_lower = sym.name.to_lowercase();
                let Some(tier) = match_tier(&name_lower, &q) else {
                    continue;
                };
                file_had_symbol_hit = true;
                hits.push(SearchHit {
                    path: file.relative_path.clone(),
                    symbol: Some(sym.name.clone()),
                    kind: Some(kind_label(&sym.kind)),
                    start_line: Some(sym.start_line),
                    match_kind: "symbol".to_string(),
                    score: tier + PAGERANK_WEIGHT * pr,
                });
            }
        }

        // Content-only fallback: a relevant file with no symbol hit still
        // surfaces once, so the loop can `expand` from it.
        if !file_had_symbol_hit && file.content.to_lowercase().contains(&q) {
            hits.push(SearchHit {
                path: file.relative_path.clone(),
                symbol: None,
                kind: None,
                start_line: None,
                match_kind: "content".to_string(),
                score: TIER_CONTENT + PAGERANK_WEIGHT * pr,
            });
        }
    }

    // Explicit TOTAL ORDER: score desc (via f64::total_cmp — a genuine total
    // order, so ties are byte-stable), then path, symbol, start_line,
    // match_kind. Every tiebreak key is total, so equal scores never reorder.
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.symbol.cmp(&b.symbol))
            .then_with(|| a.start_line.cmp(&b.start_line))
            .then_with(|| a.match_kind.cmp(&b.match_kind))
    });
    hits.truncate(limit);

    SearchResult {
        query: query.to_string(),
        hits,
    }
}

/// Classify how `name` matches `query` (both already lowercased), or `None`.
fn match_tier(name: &str, query: &str) -> Option<f64> {
    if name == query {
        Some(TIER_EXACT)
    } else if name.starts_with(query) {
        Some(TIER_PREFIX)
    } else if name.contains(query) {
        Some(TIER_SUBSTRING)
    } else {
        None
    }
}

/// `references(symbol)` — files that reference `symbol`, from the index's
/// inverted `symbol → files` map, sorted and de-duplicated.
///
/// Uses the SAME [`normalize_identifier`] key that `symbol_importance` uses, so
/// a lookup here and a cross-ref weight there can never diverge. Results come
/// purely from cxpak's own `term_frequencies` — no external process.
pub fn references(index: &CodebaseIndex, symbol: &str) -> ReferenceResult {
    let cross_refs = build_symbol_cross_refs(&index.term_frequencies);
    let key = normalize_identifier(symbol);
    let mut files: Vec<String> = cross_refs
        .get(&key)
        .map(|set| set.iter().cloned().collect())
        .unwrap_or_default();
    files.sort();
    files.dedup();
    ReferenceResult {
        symbol: symbol.to_string(),
        files,
    }
}

/// `expand(seeds, depth)` — the bounded neighbourhood of `seeds`, delegated
/// verbatim to the B1 [`graph_query::subgraph`] primitive (already
/// byte-deterministic: sorted seeds, nodes, and induced edges).
pub fn expand(index: &CodebaseIndex, seeds: &[&str], depth: usize) -> Subgraph {
    graph_query::subgraph(&index.graph, seeds, depth)
}

// ---------------------------------------------------------------------------
// Single dispatch entry point — every surface calls this.
// ---------------------------------------------------------------------------

/// Execute a retrieval `op` with JSON `params` against `index`, returning the
/// deterministic JSON result. This is the one core all four surfaces invoke.
///
/// * `search`     — params: `{ "query": string, "limit"?: number }`
/// * `references` — params: `{ "symbol": string }`
/// * `expand`     — params: `{ "seeds": [string], "depth"?: number }`
pub fn execute(index: &CodebaseIndex, op: &str, params: &Value) -> Result<Value, RetrievalError> {
    match op {
        "search" => {
            let query = req_str(params, "query")?;
            let limit = params
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_SEARCH_LIMIT);
            Ok(to_json(&search(index, query, limit)))
        }
        "references" => {
            let symbol = req_str(params, "symbol")?;
            Ok(to_json(&references(index, symbol)))
        }
        "expand" => {
            let seeds_val = params
                .get("seeds")
                .and_then(|v| v.as_array())
                .ok_or_else(|| RetrievalError::MissingParam("seeds".to_string()))?;
            let seeds: Vec<&str> = seeds_val.iter().filter_map(|v| v.as_str()).collect();
            if seeds.is_empty() {
                return Err(RetrievalError::MissingParam("seeds".to_string()));
            }
            let depth = params
                .get("depth")
                .and_then(|v| v.as_u64())
                .map(|d| d as usize)
                .unwrap_or(1);
            Ok(to_json(&expand(index, &seeds, depth)))
        }
        other => Err(RetrievalError::UnknownOp(other.to_string())),
    }
}

/// Extract a required non-empty string parameter.
fn req_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, RetrievalError> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RetrievalError::MissingParam(key.to_string()))
}

/// Serialize a retrieval result struct to JSON. These structs contain only
/// strings, numbers, bools, options, and arrays thereof, so serialization is
/// infallible.
fn to_json<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("retrieval result structs always serialize")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::parser::language::{ParseResult, Symbol, Visibility};
    use crate::scanner::ScannedFile;
    use serde_json::json;
    use std::collections::HashMap;

    /// Two-file Rust index with real symbols and a `main.rs → lib.rs` import so
    /// search, references, and expand all have something to bite on.
    fn sample_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let files = vec![
            ScannedFile {
                relative_path: "src/main.rs".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/src/main.rs"),
                language: Some("rust".to_string()),
                size_bytes: 64,
            },
            ScannedFile {
                relative_path: "src/lib.rs".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/src/lib.rs"),
                language: Some("rust".to_string()),
                size_bytes: 64,
            },
        ];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/main.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "run_search".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn run_search()".to_string(),
                    body: "fn run_search() { helper(); }".to_string(),
                    start_line: 3,
                    end_line: 9,
                }],
                imports: vec![crate::parser::language::Import {
                    source: "crate::lib".to_string(),
                    names: vec!["lib".to_string()],
                }],
                exports: vec![],
            },
        );
        parse_results.insert(
            "src/lib.rs".to_string(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "search".to_string(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn search()".to_string(),
                        body: "fn search() {}".to_string(),
                        start_line: 1,
                        end_line: 2,
                    },
                    Symbol {
                        name: "helper".to_string(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn helper()".to_string(),
                        body: "fn helper() {}".to_string(),
                        start_line: 4,
                        end_line: 5,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let mut content = HashMap::new();
        content.insert(
            "src/main.rs".to_string(),
            "mod lib;\nuse crate::lib;\nfn run_search() { helper(); }".to_string(),
        );
        content.insert(
            "src/lib.rs".to_string(),
            "pub fn search() {}\npub fn helper() {}".to_string(),
        );
        CodebaseIndex::build_with_content(files, parse_results, &counter, content)
    }

    #[test]
    fn search_ranks_exact_above_prefix_above_substring() {
        let idx = sample_index();
        let r = search(&idx, "search", 20);
        // `search` (exact, lib.rs) ranks above `run_search` (substring, main.rs).
        let names: Vec<Option<&str>> = r
            .hits
            .iter()
            .map(|h| h.symbol.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(names.first(), Some(&Some("search")));
        // The exact match outranks the substring match.
        let exact = r
            .hits
            .iter()
            .find(|h| h.symbol.as_deref() == Some("search"));
        let sub = r
            .hits
            .iter()
            .find(|h| h.symbol.as_deref() == Some("run_search"));
        assert!(exact.unwrap().score > sub.unwrap().score);
    }

    #[test]
    fn search_hit_carries_kind_and_line() {
        let idx = sample_index();
        let r = search(&idx, "helper", 20);
        let hit = r
            .hits
            .iter()
            .find(|h| h.symbol.as_deref() == Some("helper"))
            .expect("helper present");
        assert_eq!(hit.kind.as_deref(), Some("function"));
        assert_eq!(hit.start_line, Some(4));
        assert_eq!(hit.match_kind, "symbol");
        assert_eq!(hit.path, "src/lib.rs");
    }

    #[test]
    fn search_respects_limit() {
        let idx = sample_index();
        let r = search(&idx, "search", 1);
        assert_eq!(r.hits.len(), 1);
        assert_eq!(r.hits[0].symbol.as_deref(), Some("search"));
    }

    #[test]
    fn search_empty_query_is_empty() {
        let idx = sample_index();
        assert!(search(&idx, "", 20).hits.is_empty());
    }

    #[test]
    fn search_total_order_is_byte_deterministic() {
        let idx = sample_index();
        let first = serde_json::to_string(&search(&idx, "search", 20)).unwrap();
        for _ in 0..50 {
            let again = serde_json::to_string(&search(&idx, "search", 20)).unwrap();
            assert_eq!(
                again, first,
                "search output must be byte-identical every run"
            );
        }
    }

    #[test]
    fn references_are_sorted_and_from_own_index() {
        let idx = sample_index();
        let r = references(&idx, "helper");
        assert_eq!(r.symbol, "helper");
        // helper appears in both files (defined in lib, called in main).
        assert_eq!(r.files, vec!["src/lib.rs", "src/main.rs"]);
        // Sorted ascending (no HashSet order leak).
        let mut sorted = r.files.clone();
        sorted.sort();
        assert_eq!(r.files, sorted);
    }

    #[test]
    fn references_unknown_symbol_is_empty() {
        let idx = sample_index();
        assert!(references(&idx, "nonexistent_symbol_xyz").files.is_empty());
    }

    #[test]
    fn expand_delegates_to_subgraph() {
        let idx = sample_index();
        let sg = expand(&idx, &["src/main.rs"], 1);
        assert!(sg.nodes.contains(&"src/main.rs".to_string()));
        // main.rs imports lib.rs, so depth-1 expansion reaches it.
        assert!(sg.nodes.contains(&"src/lib.rs".to_string()));
        assert_eq!(sg.depth, 1);
    }

    #[test]
    fn execute_dispatches_all_three_ops() {
        let idx = sample_index();
        let s = execute(&idx, "search", &json!({"query": "search"})).unwrap();
        assert!(!s["hits"].as_array().unwrap().is_empty());
        let refs = execute(&idx, "references", &json!({"symbol": "helper"})).unwrap();
        assert_eq!(refs["files"], json!(["src/lib.rs", "src/main.rs"]));
        let ex = execute(
            &idx,
            "expand",
            &json!({"seeds": ["src/main.rs"], "depth": 1}),
        )
        .unwrap();
        assert_eq!(ex["depth"], json!(1));
    }

    #[test]
    fn execute_missing_params_error() {
        let idx = sample_index();
        assert_eq!(
            execute(&idx, "search", &json!({})),
            Err(RetrievalError::MissingParam("query".into()))
        );
        assert_eq!(
            execute(&idx, "search", &json!({"query": ""})),
            Err(RetrievalError::MissingParam("query".into()))
        );
        assert_eq!(
            execute(&idx, "references", &json!({})),
            Err(RetrievalError::MissingParam("symbol".into()))
        );
        assert!(matches!(
            execute(&idx, "expand", &json!({"depth": 1})),
            Err(RetrievalError::MissingParam(_))
        ));
        assert!(matches!(
            execute(&idx, "expand", &json!({"seeds": []})),
            Err(RetrievalError::MissingParam(_))
        ));
    }

    #[test]
    fn execute_unknown_op_errors() {
        let idx = sample_index();
        assert_eq!(
            execute(&idx, "frobnicate", &json!({})),
            Err(RetrievalError::UnknownOp("frobnicate".into()))
        );
    }

    #[test]
    fn iterative_loop_is_byte_deterministic() {
        // The C1 contract: search → references → expand, chained, must be
        // byte-identical across runs (no HashMap iteration order leak anywhere
        // in the loop).
        let idx = sample_index();
        let run_loop = || -> String {
            let s = execute(&idx, "search", &json!({"query": "helper"})).unwrap();
            let first_path = s["hits"][0]["path"].as_str().unwrap().to_string();
            let refs = execute(&idx, "references", &json!({"symbol": "helper"})).unwrap();
            let seeds: Vec<&str> = refs["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            let ex = execute(&idx, "expand", &json!({"seeds": seeds, "depth": 2})).unwrap();
            format!("{s}|{first_path}|{refs}|{ex}")
        };
        let first = run_loop();
        for _ in 0..50 {
            assert_eq!(run_loop(), first, "retrieval loop must be reproducible");
        }
    }
}
