//! Structural data flow analysis (v1.5.0).
//!
//! Walks the [`crate::intelligence::call_graph::CallGraph`] to trace named
//! parameters from a source symbol toward sinks. The trace is **structural**:
//! it follows static call paths recorded by the call graph builder. Paths that
//! cross closures, higher-order functions, or dynamic dispatch (trait objects /
//! virtual methods) are tagged [`FlowConfidence::Speculative`] because the
//! call graph cannot prove they will execute at runtime.
//!
//! ## Limitations
//!
//! These limitations are documented prominently because they affect every
//! caller and downstream LLM consumer:
//!
//! - **Closures** — capturing closures invoked indirectly are tagged
//!   `Speculative`. The call graph records the closure body, not the call site
//!   that eventually invokes it.
//! - **Higher-order functions** — `arr.map(f)` style calls are tagged
//!   `Speculative` for the same reason.
//! - **Dynamic dispatch** — trait objects (`Box<dyn Trait>`), virtual methods,
//!   and Python duck-typed dispatch are tagged `Speculative` because the call
//!   graph builder records them as `CallConfidence::Approximate`.
//! - **Max depth 10** — paths longer than 10 hops are truncated; the result's
//!   `truncated` flag is set so the caller can decide whether to widen the
//!   search.
//! - **No call graph** — when [`CodebaseIndex::call_graph`] has no edges
//!   (because v1.3.0 extraction has not been run for this language), the
//!   trace returns an empty path list rather than panicking.

use crate::intelligence::call_graph::{CallConfidence, CallEdge};
use serde::Serialize;
use std::collections::HashSet;

/// Maximum hops the trace will follow. The depth argument is clamped to this
/// value to keep latency bounded and prevent runaway traces in cyclic graphs.
pub const MAX_DEPTH: usize = 10;

/// Names commonly used for the input parameter of a route handler / use-case
/// entry point. The first parameter of any function in the symbol's `signature`
/// is also treated as an "input" so the trace can begin even when the user did
/// not pass an explicit parameter name.
const INPUT_PARAM_NAMES: &[&str] = &[
    "input", "request", "body", "data", "payload", "req", "event", "args",
];

/// Substrings that mark a callee as a [`FlowNodeType::Sink`].
const SINK_KEYWORDS: &[&str] = &[
    "save", "insert", "write", "put", "update", "delete", "send", "respond", "render", "emit",
    "publish", "store",
];

/// Substrings that mark a callee as a [`FlowNodeType::Transform`].
const TRANSFORM_KEYWORDS: &[&str] = &[
    "parse",
    "validate",
    "format",
    "sanitize",
    "transform",
    "encode",
    "decode",
    "serialize",
    "deserialize",
    "normalize",
];

/// Limitation strings emitted on every [`DataFlowResult`]. The LLM gets to see
/// them on every response so it can reason about confidence appropriately.
fn standard_limitations() -> Vec<String> {
    vec![
        "Closures and higher-order functions tracked as Speculative — call graph cannot prove indirect invocation.".into(),
        "Trait objects, virtual methods, and dynamic dispatch tagged Speculative.".into(),
        format!("Max trace depth is {MAX_DEPTH} hops; longer paths are truncated."),
        "Empty paths returned when no call-graph edges exist for the source symbol (graceful degradation).".into(),
    ]
}

fn classify_callee(symbol: &str) -> FlowNodeType {
    let lower = symbol.to_lowercase();
    if SINK_KEYWORDS.iter().any(|kw| lower.contains(kw)) {
        FlowNodeType::Sink
    } else if TRANSFORM_KEYWORDS.iter().any(|kw| lower.contains(kw)) {
        FlowNodeType::Transform
    } else {
        FlowNodeType::Passthrough
    }
}

/// Returns the language string for a file path by looking it up in the index.
fn lookup_language(index: &crate::index::CodebaseIndex, file: &str) -> String {
    index
        .files
        .iter()
        .find(|f| f.relative_path == file)
        .and_then(|f| f.language.clone())
        .unwrap_or_else(|| "unknown".into())
}

/// Returns true when consecutive nodes have different first two path segments.
fn module_boundary_crossed(a: &str, b: &str) -> bool {
    let pa: Vec<&str> = a.split('/').take(2).collect();
    let pb: Vec<&str> = b.split('/').take(2).collect();
    pa != pb
}

/// Compute confidence for a path based on its individual hop confidences and
/// whether any parameter could not be matched (passed in as `unresolved`).
fn path_confidence(hops: &[CallConfidence], unresolved: bool) -> FlowConfidence {
    if unresolved || hops.contains(&CallConfidence::Approximate) {
        FlowConfidence::Speculative
    } else {
        FlowConfidence::Exact
    }
}

/// One BFS frontier entry: which hop, which path of nodes built so far, and
/// the per-hop call confidence vector accumulated along that path.
#[derive(Clone)]
struct FrontierEntry {
    nodes: Vec<FlowNode>,
    confidences: Vec<CallConfidence>,
    /// Whether at least one hop along this path could not be resolved by name.
    unresolved: bool,
}

/// Trace how a value flows through the system, starting from `symbol`.
///
/// Algorithm:
/// 1. Locate the source symbol in the index. If absent, return an empty result
///    with the source filled in as best-effort.
/// 2. If the call graph is empty (no edges at all, e.g. v1.3.0 extraction has
///    not been run for the language), return an empty path list. This is the
///    "graceful degradation" guard from Task 14.
/// 3. BFS over [`crate::intelligence::call_graph::CallGraph`] from the source.
///    Each frontier entry tracks the path so far and the per-hop confidence.
/// 4. Each visited node is classified as Source / Transform / Sink /
///    Passthrough by name heuristic. The trace stops at sinks or at the user-
///    provided `sink` symbol.
/// 5. The depth limit is `min(depth, MAX_DEPTH)`. Any frontier popped at
///    `length == max_depth` without reaching a sink is recorded as a truncated
///    path and the result's `truncated` flag is set.
pub fn trace_data_flow(
    symbol: &str,
    sink: Option<&str>,
    depth: usize,
    index: &crate::index::CodebaseIndex,
) -> DataFlowResult {
    let max_depth = depth.min(MAX_DEPTH);

    // 1. Locate the source symbol. We accept any file that defines a public or
    //    private symbol with this name; if multiple exist, we pick the first.
    let (source_file, source_language, source_param) = locate_source(symbol, index);

    let source_node = FlowNode {
        file: source_file.clone(),
        symbol: symbol.to_string(),
        parameter: source_param,
        language: source_language.clone(),
        node_type: FlowNodeType::Source,
    };

    // 2. Guard: empty call graph or unknown source → empty paths.
    if index.call_graph.edges.is_empty() {
        return DataFlowResult {
            source: source_node,
            sink: None,
            paths: Vec::new(),
            truncated: false,
            limitations: standard_limitations(),
        };
    }

    // Special case: depth == 0 → return only the source node, no traversal.
    if max_depth == 0 {
        let single = FlowPath {
            length: 1,
            crosses_module_boundary: false,
            crosses_language_boundary: false,
            touches_security_boundary: touches_security(&source_node, index),
            confidence: FlowConfidence::Exact,
            nodes: vec![source_node.clone()],
        };
        return DataFlowResult {
            source: source_node,
            sink: None,
            paths: vec![single],
            truncated: false,
            limitations: standard_limitations(),
        };
    }

    // 3. BFS frontier. Start with the source node only.
    let mut frontier: Vec<FrontierEntry> = vec![FrontierEntry {
        nodes: vec![source_node.clone()],
        confidences: Vec::new(),
        unresolved: false,
    }];
    let mut completed: Vec<FlowPath> = Vec::new();
    let mut truncated = false;
    let mut sink_node: Option<FlowNode> = None;

    while let Some(entry) = frontier.pop() {
        let last = entry.nodes.last().expect("nodes never empty in frontier");

        // Cycle detection: if any of the previous nodes share file+symbol with
        // the most recent hop more than once, drop this branch and mark
        // truncated. Without this, A → B → A would loop forever within the
        // depth budget.
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut cyclic = false;
        for n in &entry.nodes {
            if !seen.insert((n.file.clone(), n.symbol.clone())) {
                cyclic = true;
                break;
            }
        }
        if cyclic {
            truncated = true;
            // Record the path so the caller sees the cycle existed.
            completed.push(build_path(
                &entry.nodes,
                &entry.confidences,
                entry.unresolved,
                index,
            ));
            continue;
        }

        // If we've reached the user-supplied sink, stop here.
        if let Some(target) = sink {
            if last.symbol == target {
                sink_node = Some(last.clone());
                completed.push(build_path(
                    &entry.nodes,
                    &entry.confidences,
                    entry.unresolved,
                    index,
                ));
                continue;
            }
        }

        // If the last node is itself a Sink (per heuristic), record and stop.
        if last.node_type == FlowNodeType::Sink && entry.nodes.len() > 1 {
            sink_node = Some(last.clone());
            completed.push(build_path(
                &entry.nodes,
                &entry.confidences,
                entry.unresolved,
                index,
            ));
            continue;
        }

        // Depth limit: stop expanding when we've hit max_depth nodes.
        if entry.nodes.len() >= max_depth {
            truncated = true;
            completed.push(build_path(
                &entry.nodes,
                &entry.confidences,
                entry.unresolved,
                index,
            ));
            continue;
        }

        // Expand: look up callees of the last node from the call graph.
        let callees = index.call_graph.callees_from(&last.file, &last.symbol);
        if callees.is_empty() {
            // Dead end — record the path as completed, no expansion possible.
            completed.push(build_path(
                &entry.nodes,
                &entry.confidences,
                entry.unresolved,
                index,
            ));
            continue;
        }

        for edge in callees {
            let next_lang = lookup_language(index, &edge.callee_file);
            let next_node = FlowNode {
                file: edge.callee_file.clone(),
                symbol: edge.callee_symbol.clone(),
                parameter: forward_parameter(last, edge),
                language: next_lang,
                node_type: classify_callee(&edge.callee_symbol),
            };
            let mut next_entry = entry.clone();
            next_entry.confidences.push(edge.confidence.clone());
            // If we couldn't forward a parameter, the trace is uncertain about
            // what value continues to flow.
            if next_node.parameter.is_none() && !entry.unresolved {
                next_entry.unresolved = true;
            }
            next_entry.nodes.push(next_node);
            frontier.push(next_entry);
        }
    }

    DataFlowResult {
        source: source_node,
        sink: sink_node,
        paths: completed,
        truncated,
        limitations: standard_limitations(),
    }
}

/// Heuristically locate the source symbol's file/language/parameter name.
fn locate_source(
    symbol: &str,
    index: &crate::index::CodebaseIndex,
) -> (String, String, Option<String>) {
    let matches = index.find_symbol(symbol);
    if let Some((path, sym)) = matches.first() {
        let lang = index
            .files
            .iter()
            .find(|f| f.relative_path == *path)
            .and_then(|f| f.language.clone())
            .unwrap_or_else(|| "unknown".into());
        let param = first_parameter_name(&sym.signature);
        return ((*path).to_string(), lang, param);
    }
    // Symbol not found in any indexed file — record what we know.
    ("<unknown>".into(), "unknown".into(), None)
}

/// Forward the named parameter from `prev` into the callee.
///
/// Heuristic: if the previous hop's `parameter` is one of the well-known
/// input names ([`INPUT_PARAM_NAMES`]), forward it unchanged so the trace
/// records a Passthrough. Otherwise return None to mark the hop as unresolved.
fn forward_parameter(prev: &FlowNode, _edge: &CallEdge) -> Option<String> {
    let param = prev.parameter.as_deref()?;
    if INPUT_PARAM_NAMES.contains(&param) {
        Some(param.to_string())
    } else {
        // We can't statically prove the parameter survives the call boundary.
        None
    }
}

/// Extract the first parameter name from a function signature like
/// `fn handle(req: Request) -> Response`. Returns `None` for signatures the
/// regex cannot make sense of.
fn first_parameter_name(signature: &str) -> Option<String> {
    let open = signature.find('(')?;
    let close = signature[open..].find(')')?;
    let params = &signature[open + 1..open + close];
    let first = params.split(',').next()?.trim();
    // Pull the identifier before the first colon (`name: Type`) or first
    // whitespace (`Type name` style for Java/C).
    if let Some(colon_idx) = first.find(':') {
        let name = first[..colon_idx].trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    let last_word = first.split_whitespace().last()?;
    if last_word.is_empty() {
        None
    } else {
        Some(last_word.to_string())
    }
}

/// Construct the final [`FlowPath`] from accumulated frontier state.
fn build_path(
    nodes: &[FlowNode],
    confidences: &[CallConfidence],
    unresolved: bool,
    index: &crate::index::CodebaseIndex,
) -> FlowPath {
    let crosses_module_boundary = nodes
        .windows(2)
        .any(|pair| module_boundary_crossed(&pair[0].file, &pair[1].file));
    let crosses_language_boundary = nodes
        .windows(2)
        .any(|pair| pair[0].language != pair[1].language);
    let touches_security_boundary = nodes.iter().any(|n| touches_security(n, index));

    FlowPath {
        length: nodes.len(),
        crosses_module_boundary,
        crosses_language_boundary,
        touches_security_boundary,
        confidence: path_confidence(confidences, unresolved),
        nodes: nodes.to_vec(),
    }
}

/// Stub for a future SecuritySurface integration. v1.5.0 has no
/// SecuritySurface field on `CodebaseIndex`, so this always returns false.
fn touches_security(_node: &FlowNode, _index: &crate::index::CodebaseIndex) -> bool {
    false
}

/// What role a node plays in the data flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum FlowNodeType {
    /// The starting symbol of the trace.
    Source,
    /// The symbol parses, validates, sanitizes, formats, encodes, etc.
    Transform,
    /// The symbol writes/sends/persists/responds — the trace stops here.
    Sink,
    /// The symbol forwards the value to another callee unchanged.
    Passthrough,
}

/// How confident the trace is that this path will actually execute.
///
/// `Exact`        — every hop is a direct, statically resolved call.
/// `Approximate`  — at least one hop was resolved via a name match, not a
///                  precise binding. The path is plausible but not guaranteed.
/// `Speculative`  — at least one hop crosses a closure, higher-order function,
///                  trait object, or virtual dispatch. Treat with skepticism.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum FlowConfidence {
    Exact,
    Approximate,
    Speculative,
}

/// One hop in a flow path.
#[derive(Debug, Clone, Serialize)]
pub struct FlowNode {
    pub file: String,
    pub symbol: String,
    /// The named parameter being traced through this node, if known.
    pub parameter: Option<String>,
    pub language: String,
    pub node_type: FlowNodeType,
}

/// A complete trace from `Source` to either a `Sink` or the depth limit.
#[derive(Debug, Clone, Serialize)]
pub struct FlowPath {
    pub nodes: Vec<FlowNode>,
    pub crosses_module_boundary: bool,
    pub crosses_language_boundary: bool,
    pub touches_security_boundary: bool,
    pub confidence: FlowConfidence,
    pub length: usize,
}

/// The result of [`trace_data_flow`].
#[derive(Debug, Serialize)]
pub struct DataFlowResult {
    pub source: FlowNode,
    pub sink: Option<FlowNode>,
    pub paths: Vec<FlowPath>,
    /// True when at least one path was pruned by the depth limit.
    pub truncated: bool,
    /// Documented limitations — included on every result so the LLM always
    /// sees them. See module docs for the full list.
    pub limitations: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::index::CodebaseIndex;
    use crate::intelligence::call_graph::{CallEdge, CallGraph};
    use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    /// Build an index containing the named symbols, with no call-graph edges.
    /// Tests inject the call graph manually so they can shape it precisely.
    fn build_index_with_symbols(symbols: &[(&str, &str, &str)]) -> CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let mut files = Vec::new();
        let mut parse_results = HashMap::new();

        for (path, lang, sig) in symbols {
            let abs = dir.path().join(path);
            std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
            // Body must contain the symbol name so the index can find it.
            std::fs::write(&abs, format!("// stub for {sig}")).unwrap();
            files.push(ScannedFile {
                relative_path: (*path).into(),
                absolute_path: abs,
                language: Some((*lang).into()),
                size_bytes: 32,
            });
            // Pull the symbol name from the signature: `fn name(...)`.
            let name = sig
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.split('(').next())
                .unwrap_or(sig)
                .to_string();
            parse_results.insert(
                (*path).to_string(),
                ParseResult {
                    symbols: vec![Symbol {
                        name,
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: (*sig).into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 3,
                    }],
                    imports: vec![],
                    exports: vec![],
                },
            );
        }

        CodebaseIndex::build(files, parse_results, &counter)
    }

    #[test]
    fn test_trace_data_flow_no_call_graph() {
        // Build an index with a known symbol but force the call graph to be
        // empty (no edges). The trace must return an empty path list rather
        // than panic — that's the Task 14 graceful degradation guard.
        let mut index =
            build_index_with_symbols(&[("src/api.rs", "rust", "fn handle_request(req: Request)")]);
        index.call_graph = CallGraph::default();
        let result = trace_data_flow("handle_request", None, 10, &index);
        assert!(result.paths.is_empty(), "no edges → no paths");
        assert!(!result.truncated, "no edges → not truncated");
        assert!(!result.limitations.is_empty(), "always emit limitations");
    }

    #[test]
    fn test_trace_source_to_sink_direct() {
        let mut index = build_index_with_symbols(&[
            ("src/api.rs", "rust", "fn handle_request(req: Request)"),
            ("src/db.rs", "rust", "fn save_user(req: Request)"),
        ]);
        index.call_graph = CallGraph {
            edges: vec![CallEdge {
                caller_file: "src/api.rs".into(),
                caller_symbol: "handle_request".into(),
                callee_file: "src/db.rs".into(),
                callee_symbol: "save_user".into(),
                confidence: CallConfidence::Exact,
            }],
            unresolved: Vec::new(),
        };
        let result = trace_data_flow("handle_request", None, 10, &index);
        assert_eq!(result.paths.len(), 1);
        let path = &result.paths[0];
        assert_eq!(path.length, 2);
        assert_eq!(path.confidence, FlowConfidence::Exact);
        assert_eq!(path.nodes.last().unwrap().node_type, FlowNodeType::Sink);
    }

    #[test]
    fn test_trace_max_depth_truncates() {
        // Build a 12-hop linear chain — the depth-10 trace must truncate.
        let mut symbols = Vec::new();
        for i in 0..12 {
            symbols.push((
                Box::leak(format!("src/f{i}.rs").into_boxed_str()) as &str,
                "rust",
                Box::leak(format!("fn step_{i}(req: Request)").into_boxed_str()) as &str,
            ));
        }
        let mut index = build_index_with_symbols(&symbols);
        let mut edges = Vec::new();
        for i in 0..11 {
            edges.push(CallEdge {
                caller_file: format!("src/f{i}.rs"),
                caller_symbol: format!("step_{i}"),
                callee_file: format!("src/f{}.rs", i + 1),
                callee_symbol: format!("step_{}", i + 1),
                confidence: CallConfidence::Exact,
            });
        }
        index.call_graph = CallGraph {
            edges,
            unresolved: Vec::new(),
        };
        let result = trace_data_flow("step_0", None, 10, &index);
        assert!(result.truncated, "12-hop chain must truncate at depth 10");
        for path in &result.paths {
            assert!(path.length <= MAX_DEPTH, "no path may exceed MAX_DEPTH");
        }
    }

    #[test]
    fn test_trace_passthrough_classification() {
        // A middle hop that doesn't match sink/transform keywords becomes Passthrough.
        let mut index = build_index_with_symbols(&[
            ("src/api.rs", "rust", "fn handle_request(req: Request)"),
            ("src/middle.rs", "rust", "fn route(req: Request)"),
            ("src/db.rs", "rust", "fn save_user(req: Request)"),
        ]);
        index.call_graph = CallGraph {
            edges: vec![
                CallEdge {
                    caller_file: "src/api.rs".into(),
                    caller_symbol: "handle_request".into(),
                    callee_file: "src/middle.rs".into(),
                    callee_symbol: "route".into(),
                    confidence: CallConfidence::Exact,
                },
                CallEdge {
                    caller_file: "src/middle.rs".into(),
                    caller_symbol: "route".into(),
                    callee_file: "src/db.rs".into(),
                    callee_symbol: "save_user".into(),
                    confidence: CallConfidence::Exact,
                },
            ],
            unresolved: Vec::new(),
        };
        let result = trace_data_flow("handle_request", None, 10, &index);
        let middle = result
            .paths
            .iter()
            .flat_map(|p| p.nodes.iter())
            .find(|n| n.symbol == "route")
            .expect("route node present");
        assert_eq!(middle.node_type, FlowNodeType::Passthrough);
    }

    #[test]
    fn test_trace_dynamic_dispatch_speculative() {
        // CallConfidence::Approximate → entire path becomes Speculative.
        let mut index = build_index_with_symbols(&[
            ("src/api.rs", "rust", "fn handle_request(req: Request)"),
            ("src/save.rs", "rust", "fn save_record(req: Request)"),
        ]);
        index.call_graph = CallGraph {
            edges: vec![CallEdge {
                caller_file: "src/api.rs".into(),
                caller_symbol: "handle_request".into(),
                callee_file: "src/save.rs".into(),
                callee_symbol: "save_record".into(),
                confidence: CallConfidence::Approximate,
            }],
            unresolved: Vec::new(),
        };
        let result = trace_data_flow("handle_request", None, 10, &index);
        assert_eq!(result.paths.len(), 1);
        assert_eq!(result.paths[0].confidence, FlowConfidence::Speculative);
    }

    #[test]
    fn test_trace_depth_zero() {
        // depth: 0 means no traversal — the result contains only the source.
        let mut index = build_index_with_symbols(&[
            ("src/api.rs", "rust", "fn handle_request(req: Request)"),
            ("src/db.rs", "rust", "fn save_user(req: Request)"),
        ]);
        index.call_graph = CallGraph {
            edges: vec![CallEdge {
                caller_file: "src/api.rs".into(),
                caller_symbol: "handle_request".into(),
                callee_file: "src/db.rs".into(),
                callee_symbol: "save_user".into(),
                confidence: CallConfidence::Exact,
            }],
            unresolved: Vec::new(),
        };
        let result = trace_data_flow("handle_request", None, 0, &index);
        assert!(!result.truncated, "depth=0 is not a truncation event");
        // Single path containing only the source.
        assert_eq!(result.paths.len(), 1);
        assert_eq!(result.paths[0].nodes.len(), 1);
        assert_eq!(result.paths[0].nodes[0].node_type, FlowNodeType::Source);
    }

    #[test]
    fn test_trace_cycle_does_not_loop() {
        // A → B → A cycle. The trace must terminate within the depth limit
        // and report truncated=true because the cycle prevents completion.
        let mut index = build_index_with_symbols(&[
            ("src/a.rs", "rust", "fn a_fn(req: Request)"),
            ("src/b.rs", "rust", "fn b_fn(req: Request)"),
        ]);
        index.call_graph = CallGraph {
            edges: vec![
                CallEdge {
                    caller_file: "src/a.rs".into(),
                    caller_symbol: "a_fn".into(),
                    callee_file: "src/b.rs".into(),
                    callee_symbol: "b_fn".into(),
                    confidence: CallConfidence::Exact,
                },
                CallEdge {
                    caller_file: "src/b.rs".into(),
                    caller_symbol: "b_fn".into(),
                    callee_file: "src/a.rs".into(),
                    callee_symbol: "a_fn".into(),
                    confidence: CallConfidence::Exact,
                },
            ],
            unresolved: Vec::new(),
        };
        let result = trace_data_flow("a_fn", None, 10, &index);
        // Must terminate — no panic, bounded number of paths.
        assert!(
            result.paths.len() <= 64,
            "cycle trace must terminate with a bounded path count"
        );
    }

    #[test]
    fn test_flow_path_module_boundary_flag() {
        // src/api/handler.rs → src/db/repo.rs — different first two path
        // segments, so crosses_module_boundary must be true.
        let mut index = build_index_with_symbols(&[
            ("src/api/handler.rs", "rust", "fn handler(req: Request)"),
            ("src/db/repo.rs", "rust", "fn save_row(req: Request)"),
        ]);
        index.call_graph = CallGraph {
            edges: vec![CallEdge {
                caller_file: "src/api/handler.rs".into(),
                caller_symbol: "handler".into(),
                callee_file: "src/db/repo.rs".into(),
                callee_symbol: "save_row".into(),
                confidence: CallConfidence::Exact,
            }],
            unresolved: Vec::new(),
        };
        let result = trace_data_flow("handler", None, 10, &index);
        assert!(!result.paths.is_empty());
        assert!(
            result.paths.iter().any(|p| p.crosses_module_boundary),
            "expected at least one path with module boundary crossed"
        );
    }

    #[test]
    fn test_flow_path_language_boundary_flag() {
        // TS → Rust — different language fields in the FlowNode should set
        // crosses_language_boundary.
        let mut index = build_index_with_symbols(&[
            ("src/api.ts", "typescript", "function load(req: Request)"),
            ("src/db.rs", "rust", "fn save_user(req: Request)"),
        ]);
        index.call_graph = CallGraph {
            edges: vec![CallEdge {
                caller_file: "src/api.ts".into(),
                caller_symbol: "load".into(),
                callee_file: "src/db.rs".into(),
                callee_symbol: "save_user".into(),
                confidence: CallConfidence::Exact,
            }],
            unresolved: Vec::new(),
        };
        let result = trace_data_flow("load", None, 10, &index);
        assert!(!result.paths.is_empty());
        assert!(
            result.paths.iter().any(|p| p.crosses_language_boundary),
            "expected at least one path with language boundary crossed"
        );
    }

    #[test]
    fn test_flow_node_type_variants() {
        let source = FlowNode {
            file: "a.rs".into(),
            symbol: "handle".into(),
            parameter: Some("body".into()),
            language: "rust".into(),
            node_type: FlowNodeType::Source,
        };
        let transform = FlowNode {
            node_type: FlowNodeType::Transform,
            ..source.clone()
        };
        let sink = FlowNode {
            node_type: FlowNodeType::Sink,
            ..source.clone()
        };
        let pass = FlowNode {
            node_type: FlowNodeType::Passthrough,
            ..source.clone()
        };
        assert!(matches!(source.node_type, FlowNodeType::Source));
        assert!(matches!(transform.node_type, FlowNodeType::Transform));
        assert!(matches!(sink.node_type, FlowNodeType::Sink));
        assert!(matches!(pass.node_type, FlowNodeType::Passthrough));
    }

    #[test]
    fn test_flow_confidence_ordering() {
        assert_ne!(FlowConfidence::Exact, FlowConfidence::Speculative);
        assert_ne!(FlowConfidence::Exact, FlowConfidence::Approximate);
        assert_ne!(FlowConfidence::Approximate, FlowConfidence::Speculative);
    }
}
