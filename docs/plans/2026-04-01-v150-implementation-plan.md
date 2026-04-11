# v1.5.0 "Deep Flow" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add structural data flow analysis and cross-language symbol resolution.

**Architecture:** `EdgeType` and `TypedEdge` migrate from `src/schema/mod.rs` to `src/index/graph.rs`, breaking the circular import that would occur when adding `CrossLanguage(BridgeType)`. A new `src/intelligence/data_flow.rs` module walks the v1.3.0 call graph to trace named parameters from source to sink (max depth 10), classifying each hop as Exact/Approximate/Speculative. A parallel `src/intelligence/cross_lang.rs` module detects six bridge types (HTTP, FFI, gRPC, GraphQL, SharedSchema, CommandExec) by pattern-matching against existing `api_surface` routes, schema tables, and proto definitions, injecting the results as `EdgeType::CrossLanguage(BridgeType)` edges into the dependency graph.

**Tech Stack:** Rust, tree-sitter, regex, serde

---

## Task 1 — Move `EdgeType` and `TypedEdge` into `src/index/graph.rs`

**Why first:** Every subsequent task depends on `CrossLanguage(BridgeType)` being a variant of `EdgeType`. The migration must land before any new variant is added, so all import paths are fixed in one atomic commit.

**Files:**
- `src/schema/mod.rs`
- `src/index/graph.rs`

**Steps:**
1. Write a test in `src/index/graph.rs` asserting that `EdgeType::Import` and `EdgeType::ForeignKey` are in scope from the local module (not via `crate::schema`). This test fails until the move lands.
2. Cut the `EdgeType` and `TypedEdge` definitions (with all derives) from `src/schema/mod.rs` and paste them into `src/index/graph.rs`, above the `DependencyGraph` struct.
3. Add re-exports to `src/schema/mod.rs`: `pub use crate::index::graph::{EdgeType, TypedEdge};`
4. Fix the import in `src/index/graph.rs` — remove the now-redundant `use crate::schema::{EdgeType, TypedEdge};` line at the top of that file.
5. Run `cargo check` to confirm zero compile errors; the re-export preserves all callsites in `src/schema/link.rs`, `src/commands/serve.rs`, and elsewhere.

**Code — additions to `src/index/graph.rs` (before `DependencyGraph`):**
```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeType {
    Import,
    ForeignKey,
    ViewReference,
    TriggerTarget,
    IndexTarget,
    FunctionReference,
    EmbeddedSql,
    OrmModel,
    MigrationSequence,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TypedEdge {
    pub target: String,
    pub edge_type: EdgeType,
}
```

**Code — replacement for the removed definitions in `src/schema/mod.rs`:**
```rust
pub use crate::index::graph::{EdgeType, TypedEdge};
```

**Commands:**
```
cargo test --verbose 2>&1 | head -40
```

---

## Task 2 — Add `BridgeType` enum and `EdgeType::CrossLanguage` variant

**Why now:** `BridgeType` is the payload of the new variant and must exist before `EdgeType` gains `CrossLanguage`. Cache version bump also belongs here so the serialized index format is immediately invalidated.

**Files:**
- `src/index/graph.rs`

**Steps:**
1. Write a test that constructs a `TypedEdge { target: "b.ts".into(), edge_type: EdgeType::CrossLanguage(BridgeType::HttpCall) }`, inserts it into a `HashSet`, and asserts `len() == 1`. Fails until the variant exists.
2. Add `BridgeType` above `EdgeType` in `src/index/graph.rs` with all required derives.
3. Add `CrossLanguage(BridgeType)` as the last variant of `EdgeType`.
4. Add `use serde::{Deserialize, Serialize};` to `src/index/graph.rs` if not already present (it currently imports from `crate::schema` which brought those traits in via the moved types).
5. Bump `CACHE_VERSION` in `src/cache/mod.rs` (line 8) from its current value to the next integer. Do NOT create a duplicate constant in `graph.rs` — the authoritative location is `src/cache/mod.rs`.
6. Add a match arm for `EdgeType::CrossLanguage(_)` in every existing `match edge_type` pattern. The 5 known sites are:
   - `src/intelligence/blast_radius.rs:62` — exhaustive match, add `CrossLanguage(_) => 0.5` (moderate edge weight)
   - `src/commands/overview.rs:440` — inner exhaustive match, add `CrossLanguage(bt) => format!("cross_language:{bt:?}")`
   - `src/commands/serve.rs:1275` — inner exhaustive match, same pattern as overview
   - `src/commands/trace.rs:340` (`edge_type_display`) — add `CrossLanguage(bt) => format!("cross_language:{bt:?}")`
   - `src/commands/trace.rs:388` — inner exhaustive match, same pattern
   Run `cargo check` to confirm no remaining sites.

**Code:**
```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BridgeType {
    HttpCall,
    FfiBinding,
    GrpcCall,
    GraphqlCall,
    SharedSchema,
    CommandExec,
}

// In EdgeType:
CrossLanguage(BridgeType),
```

**Commands:**
```
cargo test --verbose 2>&1 | grep -E "^(test |FAILED|error)"
```

---

## Task 3 — Unit tests for `EdgeType::CrossLanguage` round-trip serialization

**Why now:** Serialization is observable behaviour. Verifying JSON round-trips before any detection code lands ensures the schema is stable.

**Files:**
- `src/index/graph.rs` (test module)

**Steps:**
1. Add `test_cross_language_edge_hash` — inserts `CrossLanguage(BridgeType::HttpCall)` and `CrossLanguage(BridgeType::FfiBinding)` into a `HashSet<TypedEdge>` with the same target and asserts `len() == 2`.
2. Add `test_edge_type_cross_language_serialization` — `serde_json::to_string` then `serde_json::from_str` round-trip for all six `BridgeType` variants; assert structural equality.
3. Add `test_add_cross_language_edge` — `graph.add_edge("a.ts", "b.rs", EdgeType::CrossLanguage(BridgeType::FfiBinding))`, then `graph.dependencies("a.ts").unwrap()` contains an edge with that type.

**Code:**
```rust
#[test]
fn test_cross_language_edge_hash() {
    use std::collections::HashSet;
    let mut set = HashSet::new();
    set.insert(TypedEdge { target: "b.rs".into(), edge_type: EdgeType::CrossLanguage(BridgeType::HttpCall) });
    set.insert(TypedEdge { target: "b.rs".into(), edge_type: EdgeType::CrossLanguage(BridgeType::FfiBinding) });
    assert_eq!(set.len(), 2);
}

#[test]
fn test_edge_type_cross_language_serialization() {
    let variants = [
        BridgeType::HttpCall, BridgeType::FfiBinding, BridgeType::GrpcCall,
        BridgeType::GraphqlCall, BridgeType::SharedSchema, BridgeType::CommandExec,
    ];
    for bt in variants {
        let edge = TypedEdge { target: "x.py".into(), edge_type: EdgeType::CrossLanguage(bt.clone()) };
        let json = serde_json::to_string(&edge).unwrap();
        let back: TypedEdge = serde_json::from_str(&json).unwrap();
        assert_eq!(back.edge_type, edge.edge_type);
    }
}

#[test]
fn test_add_cross_language_edge() {
    let mut graph = DependencyGraph::new();
    graph.add_edge("a.ts", "b.rs", EdgeType::CrossLanguage(BridgeType::FfiBinding));
    let deps = graph.dependencies("a.ts").unwrap();
    assert!(deps.iter().any(|e| e.target == "b.rs"
        && e.edge_type == EdgeType::CrossLanguage(BridgeType::FfiBinding)));
}
```

**Commands:**
```
cargo test --verbose -p cxpak -- index::graph 2>&1
```

---

## Task 4 — Create `src/intelligence/data_flow.rs` — public types

**Why before logic:** Define all public structs/enums first so the implementation in Task 5 can reference them. This also ensures the test scaffolding compiles independently.

**Files:**
- `src/intelligence/data_flow.rs` (new)
- `src/intelligence/mod.rs`

**Steps:**
1. Create `src/intelligence/data_flow.rs` with only the public type definitions and `mod tests`.
2. Add `pub mod data_flow;` to `src/intelligence/mod.rs`.
3. Write a test `test_flow_node_type_variants` that constructs one `FlowNode` of each `FlowNodeType` variant and asserts `matches!` on each. Passes immediately after types exist.
4. Write a test `test_flow_confidence_ordering` that verifies `FlowConfidence::Exact != FlowConfidence::Speculative`.

**Code — `src/intelligence/data_flow.rs`:**
```rust
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum FlowNodeType {
    Source,
    Transform,
    Sink,
    Passthrough,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum FlowConfidence {
    Exact,
    Approximate,
    Speculative,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlowNode {
    pub file: String,
    pub symbol: String,
    pub parameter: Option<String>,
    pub language: String,
    pub node_type: FlowNodeType,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlowPath {
    pub nodes: Vec<FlowNode>,
    pub crosses_module_boundary: bool,
    pub crosses_language_boundary: bool,
    pub touches_security_boundary: bool,
    pub confidence: FlowConfidence,
    pub length: usize,
}

#[derive(Debug, Serialize)]
pub struct DataFlowResult {
    pub source: FlowNode,
    pub sink: Option<FlowNode>,
    pub paths: Vec<FlowPath>,
    pub truncated: bool,
}
```

**Commands:**
```
cargo test --verbose -p cxpak -- intelligence::data_flow 2>&1
```

---

## Task 5 — Implement `trace_data_flow()` in `src/intelligence/data_flow.rs`

**Files:**
- `src/intelligence/data_flow.rs`

**Steps:**
1. Write tests first for each classification branch:
   - `test_trace_source_to_sink_direct` — a 2-node graph where the source symbol calls the sink directly; asserts `paths.len() == 1`, `paths[0].confidence == FlowConfidence::Exact`, `paths[0].length == 2`.
   - `test_trace_max_depth_truncates` — a linear chain of 12 hops; asserts `result.truncated == true` and no path has `length > 10`.
   - `test_trace_passthrough_classification` — a hop where the parameter name is forwarded unchanged; asserts the middle node has `node_type == FlowNodeType::Passthrough`.
   - `test_trace_dynamic_dispatch_speculative` — a hop tagged with `CallConfidence::Approximate` in the call graph; asserts `confidence == FlowConfidence::Speculative`.
2. Implement `pub fn trace_data_flow(symbol: &str, sink: Option<&str>, depth: usize, index: &CodebaseIndex) -> DataFlowResult`.
3. Algorithm:
   - Locate `symbol` in `index.call_graph.edges` (from v1.3.0 `CallGraph` on `CodebaseIndex`).
   - Identify "input parameters" by name heuristic: first parameter of a route handler, or parameter named one of `{input, request, body, data, payload, req, event}`.
   - BFS over call graph edges from the symbol, tracking which parameter each hop receives. Depth limit: `depth` (default 10, maximum 10 enforced by clamping).
   - Classify each node: if the callee name contains `{save, insert, write, put, update, delete, send, respond, render, emit}` → `Sink`; if it contains `{parse, validate, format, sanitize, transform, encode, decode, serialize}` → `Transform`; otherwise `Passthrough`.
   - Set `FlowConfidence::Speculative` when the `CallEdge.confidence == CallConfidence::Approximate` OR when the parameter could not be matched by name/position.
   - Set `crosses_module_boundary` when consecutive nodes have different first two path segments.
   - Set `crosses_language_boundary` when consecutive nodes have different `language` fields.
   - Set `touches_security_boundary` when any node's file appears in the v1.4.0 [`crate::intelligence::security::build_security_surface`] output (unprotected endpoints, input-validation gaps, secret patterns, or SQL injection risks). Compute the surface **once per trace** and thread a `HashSet<String>` of risky paths through BFS via `build_path()` — avoids N×M rescanning. Since v1.4.0 is already shipped, this is a real signal, not a placeholder.
   - Stop BFS at `sink` symbol if provided.
   - Return `truncated: true` if any path was pruned by the depth limit.

**Code — function signature:**
```rust
pub fn trace_data_flow(
    symbol: &str,
    sink: Option<&str>,
    depth: usize,
    index: &crate::index::CodebaseIndex,
) -> DataFlowResult
```

**Commands:**
```
cargo test --verbose -p cxpak -- intelligence::data_flow 2>&1
```

---

## Task 6 — Create `src/intelligence/cross_lang.rs` — public types

**Files:**
- `src/intelligence/cross_lang.rs` (new)
- `src/intelligence/mod.rs`

**Steps:**
1. Create `src/intelligence/cross_lang.rs` with only the `CrossLangEdge` struct.
2. Add `pub mod cross_lang;` to `src/intelligence/mod.rs`.
3. Write a test `test_cross_lang_edge_fields` that constructs a `CrossLangEdge` and asserts all fields are accessible.

**Code — `src/intelligence/cross_lang.rs`:**
```rust
use crate::index::graph::BridgeType;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CrossLangEdge {
    pub source_file: String,
    pub source_symbol: String,
    pub source_language: String,
    pub target_file: String,
    pub target_symbol: String,
    pub target_language: String,
    pub bridge_type: BridgeType,
}
```

**Commands:**
```
cargo test --verbose -p cxpak -- intelligence::cross_lang 2>&1
```

---

## Task 7 — Implement HTTP bridge detection in `cross_lang.rs`

**Files:**
- `src/intelligence/cross_lang.rs`

**Steps:**
1. Write `test_detect_http_bridge` — build a minimal `CodebaseIndex` with a TypeScript file containing `fetch("/api/users")` and a Python file with a `@app.get("/api/users")` route (already detectable via `detect_routes`). Assert `detect_cross_lang_edges` returns one edge with `bridge_type == BridgeType::HttpCall`, `source_language == "typescript"`, `target_language == "python"`.
2. Write `test_detect_http_bridge_no_match` — a fetch with a URL that doesn't appear in any detected route produces no edge.
3. Implement `detect_http_bridges(index: &CodebaseIndex) -> Vec<CrossLangEdge>`:
   - Build a `HashMap<String, RouteEndpoint>` from `extract_api_surface`'s route list (keyed by path string, e.g. `"/api/users"`).
   - Scan all non-route files for regex `fetch\s*\(\s*["'`](?P<url>/[^"'`\s]+)` and `axios\.(get|post|put|delete|patch)\s*\(\s*["'](?P<url>/[^"']+)` and `reqwest::.*get\s*\(\s*["'](?P<url>/[^"']+)`.
   - For each URL match, strip query strings (split at `?`), look up the route map.
   - If found, emit a `CrossLangEdge`.

**Commands:**
```
cargo test --verbose -p cxpak -- intelligence::cross_lang::tests::test_detect_http 2>&1
```

---

## Task 8 — Implement FFI, gRPC, GraphQL, SharedSchema, and CommandExec bridge detection

**Files:**
- `src/intelligence/cross_lang.rs`

**Steps:**
1. Write one test per bridge type before implementing:
   - `test_detect_ffi_binding` — a Rust file with `extern "C" { fn my_c_func(); }` and a C file with `void my_c_func() {}`. Asserts `BridgeType::FfiBinding`.
   - `test_detect_grpc_call` — a Go file with `userServiceClient.GetUser(` and a `.proto` file defining `service UserService { rpc GetUser`. Asserts `BridgeType::GrpcCall`.
   - `test_detect_graphql_call` — a TypeScript file with `query GetUser {` and a `.graphql` file defining `type Query { GetUser`. Asserts `BridgeType::GraphqlCall`.
   - `test_detect_shared_schema` — a Python file with `embedded_sql` edge to table `users` and a TypeScript file also with `embedded_sql` edge to `users`. Asserts `BridgeType::SharedSchema`.
   - `test_detect_command_exec` — a Python file with `subprocess.run(["my-binary"])` and `my-binary` exists as a known file in the index. Asserts `BridgeType::CommandExec`.
2. Implement each sub-detector function, then a single public `detect_cross_lang_edges(index: &CodebaseIndex) -> Vec<CrossLangEdge>` that chains all five.
3. Detection per type:
   - **FFI:** regex `extern\s+"C"\s*\{[^}]*fn\s+(?P<name>\w+)` (Rust), `CDLL|ctypes\.CFUNCTYPE|ctypes\.WINFUNCTYPE` (Python), `@JvmStatic|native\s+fun` (Kotlin), `napi::bindgen_prelude` (Rust/Node); match against known symbol names in the target language.
   - **gRPC:** match gRPC client call patterns `\w+Client\.\w+\(` against `GrpcService.methods` from `api_surface.extract_grpc_services`.
   - **GraphQL:** match `\bquery\s+(?P<name>\w+)\s*\{|\bmutation\s+(?P<name>\w+)\s*\{` against `GraphqlType` names from `api_surface.extract_graphql_types`.
   - **SharedSchema:** walk `index.graph.edges`, collect files with `EdgeType::EmbeddedSql` or `EdgeType::OrmModel` edges to the same target table file; pair files with different languages.
   - **CommandExec:** regex `subprocess\.run\(\s*\[["'](?P<cmd>[^"']+)["']|exec\.Command\("(?P<cmd>[^"]+)"|std::process::Command::new\("(?P<cmd>[^"]+)"` against known binary/script names in the index.

**Commands:**
```
cargo test --verbose -p cxpak -- intelligence::cross_lang 2>&1
```

---

## Task 9 — Inject `CrossLanguage` edges into `DependencyGraph` during index build

**Files:**
- `src/index/graph.rs` (`build_dependency_graph`)
- `src/index/mod.rs` (`CodebaseIndex::build` and `build_with_content`)

**Steps:**
1. Write `test_build_dependency_graph_cross_lang_edges` in `src/index/graph.rs` — constructs a two-file index (TypeScript fetching a Python route), calls `build_dependency_graph`, and asserts a `CrossLanguage(BridgeType::HttpCall)` edge is present.
2. Use post-build injection (do NOT change `build_dependency_graph` signature): after `index.graph = build_dependency_graph(...)` completes in `build_with_content()`, call `let cross_edges = crate::intelligence::cross_lang::detect_cross_lang_edges(&index); for e in &cross_edges { index.graph.add_edge(&e.source_file, &e.target_file, EdgeType::CrossLanguage(e.bridge_type.clone())); }`. Cross-language detection requires the fully built index (api_surface, schema edges, etc.) so it cannot happen during graph construction.

**Commands:**
```
cargo test --verbose -p cxpak -- index 2>&1
```

---

## Task 10 — Add `cross_lang_edges` field to `CodebaseIndex` and persist detected edges

**Files:**
- `src/index/mod.rs`

**Steps:**
1. Write `test_codebase_index_cross_lang_field` — builds a `CodebaseIndex` from a two-language fixture, asserts `index.cross_lang_edges` is accessible and has type `Vec<CrossLangEdge>`.
2. Add `pub cross_lang_edges: Vec<crate::intelligence::cross_lang::CrossLangEdge>` to `CodebaseIndex`.
3. Populate it during `build_with_content` after cross-lang detection: `index.cross_lang_edges = cross_edges;`
4. Initialize to `Vec::new()` in both `CodebaseIndex::build()` (at `src/index/mod.rs` ~line 136) and `build_with_content()` (~line 287) where the struct is constructed. There is no `empty()` method — these are the only two construction paths.
5. Verify `cargo check` compiles cleanly.

**Commands:**
```
cargo test --verbose -p cxpak -- index::tests 2>&1
```

---

## Task 11 — auto_context integration: dedicated cross-language section

**Design note (post-implementation):** the original draft plan proposed
annotating each edge in the architecture map with a `cross_language: true`
flag. During implementation this was replaced by a dedicated top-level
`cross_language_edges` section on `PackedSections`, which is functionally
equivalent but cleaner: the LLM gets a focused list of bridges instead of
having to re-scan the architecture map for tagged edges, and the existing
architecture map stays untouched so downstream tooling doesn't need to
know about v1.5.0.

**Files:**
- `src/auto_context/mod.rs`
- `src/auto_context/briefing.rs`

**Steps:**
1. Write `test_auto_context_includes_cross_lang` — builds an index with cross-lang edges, runs `auto_context`, and asserts the JSON result contains a `cross_language_edges` key in the `sections` field with at least one entry.
2. Add `pub cross_language_edges: Option<serde_json::Value>` field to `PackedSections` in `src/auto_context/briefing.rs` (after `blast_radius`). Use `#[serde(skip_serializing_if = "Option::is_none")]` so the field is omitted from output when empty.
3. In `auto_context::auto_context`, after Step 9 (API surface), add Step 9.5: serialize `index.cross_lang_edges` (filtered by focus) into a `cross_lang_json: Option<Value>`.
4. Filter cross-lang edges by focus: keep only edges where `source_file.starts_with(focus)` or `target_file.starts_with(focus)`.
5. Add a new `allocate_and_pack_with_cross_lang` wrapper alongside `allocate_and_pack` so existing callers stay binary compatible. The wrapper takes an extra `cross_lang_json: Option<serde_json::Value>` parameter.
6. In `allocate_and_pack_with_cross_lang`, cap the cross-language section at `min(remaining, 500)` tokens. Structured JSON is kept intact (no truncation) so routing information is never partial.

**Commands:**
```
cargo test --verbose -p cxpak -- auto_context 2>&1
```

---

## Task 12 — `cxpak_data_flow` MCP tool

**Files:**
- `src/commands/serve.rs`

**Steps:**
1. Write `test_mcp_data_flow_tool` — sends a `tools/call` JSON-RPC message with `name: "cxpak_data_flow"` and `symbol: "handle_request"` to `mcp_stdio_loop_with_io`, asserts response contains `source` field.
2. Write `test_mcp_data_flow_missing_symbol` — omits `symbol` parameter, asserts response contains `error`.
3. Add `cxpak_data_flow` to the `tools/list` response in `mcp_stdio_loop_with_io` (in the large JSON array of tool definitions).
4. Add a match arm `"cxpak_data_flow"` in `handle_tool_call`.
5. Extract `symbol` (required), `sink` (optional string), `depth` (optional integer, default 10, clamped to 10), `focus` (optional) from `args`.
6. Call `crate::intelligence::data_flow::trace_data_flow(symbol, sink.as_deref(), depth, index)`.
7. Prepend the documented limitations as a `"limitations"` field in the JSON response so the LLM always sees them.

**Code — tool schema entry:**
```json
{
    "name": "cxpak_data_flow",
    "description": "Trace how a value flows through the system from source to sink(s). Structural analysis — follows static call paths, not runtime dispatch. Paths crossing closures or trait objects are tagged Speculative.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "symbol": { "type": "string", "description": "Starting symbol to trace from (e.g. 'handle_request')" },
            "sink":   { "type": "string", "description": "Optional target symbol to stop at" },
            "depth":  { "type": "number", "description": "Max hops to follow (default 10, max 10)", "default": 10 },
            "focus":  { "type": "string", "description": "Path prefix to scope" }
        },
        "required": ["symbol"]
    }
}
```

**Commands:**
```
cargo test --verbose -p cxpak -- commands::serve::tests 2>&1 | grep -E "data_flow|FAILED|ok"
```

---

## Task 13 — `cxpak_cross_lang` MCP tool

**Files:**
- `src/commands/serve.rs`

**Steps:**
1. Write `test_mcp_cross_lang_tool` — sends `tools/call` with `name: "cxpak_cross_lang"` and no arguments; asserts response contains `edges` array.
2. Write `test_mcp_cross_lang_file_filter` — sends with `file: "src/api.ts"`, asserts only edges where `source_file == "src/api.ts"` or `target_file == "src/api.ts"` appear.
3. Add `cxpak_cross_lang` to the `tools/list` JSON array.
4. Add match arm `"cxpak_cross_lang"` in `handle_tool_call`.
5. Extract `file` (optional string) and `focus` (optional string) from `args`.
6. Filter `index.cross_lang_edges` by `file` (exact match on `source_file` or `target_file`) and by `focus` prefix if provided.
7. Return `json!({ "edges": filtered_edges, "total": filtered_edges.len() })`.

**Code — tool schema entry:**
```json
{
    "name": "cxpak_cross_lang",
    "description": "List all detected cross-language boundaries: HTTP calls, FFI bindings, gRPC calls, GraphQL queries, shared DB schemas, and command exec bridges.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "file":  { "type": "string", "description": "Filter to edges touching this file path" },
            "focus": { "type": "string", "description": "Path prefix to scope results" }
        }
    }
}
```

**Commands:**
```
cargo test --verbose -p cxpak -- commands::serve::tests 2>&1 | grep -E "cross_lang|FAILED|ok"
```

---

## Task 14 — Call graph prerequisite guard and graceful degradation

**Why:** `trace_data_flow` depends on `CodebaseIndex.call_graph` added in v1.3.0. If v1.3.0 has not landed (the field is absent), the data flow tool must degrade gracefully rather than panic.

**Files:**
- `src/intelligence/data_flow.rs`
- `src/index/mod.rs`

**Steps:**
1. Write `test_trace_data_flow_no_call_graph` — call `trace_data_flow` on an index where `call_graph` is `None`. Assert result contains `paths: []` and `truncated: false`, not a panic.
2. In `src/index/mod.rs`, verify that `call_graph: Option<crate::intelligence::call_graph::CallGraph>` exists on `CodebaseIndex` (added in v1.3.0). If the field does not yet exist, add a stub: `pub call_graph: Option<crate::intelligence::call_graph::CallGraph>` with `call_graph: None` in all constructors. Create a minimal `src/intelligence/call_graph.rs` stub if it does not exist with just the `CallGraph`, `CallEdge`, and `CallConfidence` types (no extraction logic — that is v1.3.0 work; leave a `// TODO(v1.3.0): implement extraction` comment).
3. In `trace_data_flow`, wrap the call graph access in `let Some(cg) = &index.call_graph else { return DataFlowResult { source, sink: None, paths: vec![], truncated: false }; }`.

**Code — stub `src/intelligence/call_graph.rs` (only if absent):**
```rust
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum CallConfidence {
    Exact,
    Approximate,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallEdge {
    pub caller_file: String,
    pub caller_symbol: String,
    pub callee_file: String,
    pub callee_symbol: String,
    pub confidence: CallConfidence,
}

#[derive(Debug, Default, Serialize)]
pub struct CallGraph {
    pub edges: Vec<CallEdge>,
    pub unresolved: Vec<String>,
}
```

**Commands:**
```
cargo test --verbose -p cxpak -- intelligence::data_flow::tests::test_trace_data_flow_no_call_graph 2>&1
```

---

## Task 15 — HTTP route handler for `/data_flow` and `/cross_lang` (HTTP serve mode)

**Files:**
- `src/commands/serve.rs`

**Steps:**
1. Write `test_http_data_flow_handler` — calls `build_router`, sends a POST to `/data_flow` with `{"symbol": "handle_request"}`, asserts HTTP 200 with `source` key in body.
2. Write `test_http_cross_lang_handler` — sends GET to `/cross_lang`, asserts HTTP 200 with `edges` array.
3. Add `use axum::routing::post;` import if not present.
4. Add two new async handler functions: `data_flow_handler` and `cross_lang_handler`.
5. Register both in `build_router`: `.route("/data_flow", post(data_flow_handler))` and `.route("/cross_lang", get(cross_lang_handler))`.
6. `data_flow_handler` deserializes `{ symbol: String, sink: Option<String>, depth: Option<usize>, focus: Option<String> }`, calls `trace_data_flow`, returns JSON.
7. `cross_lang_handler` deserializes `{ file: Option<String>, focus: Option<String> }` from query params, filters `index.cross_lang_edges`, returns JSON.

**Commands:**
```
cargo test --verbose -p cxpak -- commands::serve 2>&1 | grep -E "data_flow|cross_lang|FAILED|ok"
```

---

## Task 16 — Cache version bump and `.cxpak/cache/` invalidation on startup

**Files:**
- `src/index/mod.rs` (or wherever cache read/write lives)

**Steps:**
1. Write `test_cache_version_mismatch_forces_rebuild` — writes a cache file with version 1, then calls the cache-load function, asserts it returns `None` (miss) and does not panic.
2. Locate the cache serialization code (search for `CACHE_VERSION` or `bincode` usage in the codebase).
3. Verify the version sentinel in `src/cache/mod.rs` was bumped in Task 2. The cache format includes `EdgeType` variants; adding `CrossLanguage` changes serialization so old caches are invalid.
4. On a version mismatch during cache load, delete the stale cache file and return `None` so the caller falls through to a full rebuild.

**Commands:**
```
cargo test --verbose -p cxpak -- cache 2>&1
```

---

## Task 17 — Integration test: full pipeline with cross-language fixture

**Files:**
- `tests/integration_cross_lang.rs` (new)

**Steps:**
1. Create a `tempfile::TempDir` containing:
   - `frontend/api.ts` — a TypeScript file with `fetch("/api/users")` and `fetch("/api/posts")`.
   - `backend/users.py` — a Python file with `@app.get("/api/users")` and `def get_users():`.
   - `backend/posts.py` — a Python file with `@app.post("/api/posts")` and `def create_post():`.
2. Build a `CodebaseIndex` from these files using `build_index` or `CodebaseIndex::build_with_content`.
3. Assert `index.cross_lang_edges.len() >= 2`.
4. Assert each edge has `bridge_type == BridgeType::HttpCall`.
5. Assert `source_language == "typescript"` and `target_language == "python"` for all edges.
6. Run `auto_context("add error handling to the API", &index, &opts)` and assert the result's JSON contains the string `"cross_language"`.

**Commands:**
```
cargo test --verbose -p cxpak -- integration_cross_lang 2>&1
```

---

## Task 18 — Property-based tests and coverage gap closure

**Files:**
- `src/intelligence/data_flow.rs` (test module)
- `src/intelligence/cross_lang.rs` (test module)

**Steps:**
1. Add `test_trace_depth_zero` — `depth: 0` returns a result with `paths` containing at most the single-node source path; asserts `truncated == false` (depth zero means no traversal, source itself is the only node).
2. Add `test_trace_cycle_does_not_loop` — a call graph with a cycle (A → B → A); asserts `trace_data_flow` terminates and returns `truncated: true` (cycle triggers the depth limit).
3. Add `test_detect_cross_lang_empty_index` — `detect_cross_lang_edges` on an empty `CodebaseIndex` returns `vec![]` without panicking.
4. Add `test_cross_lang_focus_filter` — builds an index with two sets of cross-lang edges in different directories; asserts focus filter returns only the scoped subset.
5. Add `test_flow_path_module_boundary_flag` — constructs a two-hop path where `nodes[0].file == "src/api/handler.rs"` and `nodes[1].file == "src/db/repo.rs"`; asserts `crosses_module_boundary == true`.
6. Add `test_flow_path_language_boundary_flag` — path from a `.ts` file to a `.rs` file; asserts `crosses_language_boundary == true`.
7. Run coverage with `cargo tarpaulin` and confirm `src/intelligence/data_flow.rs` and `src/intelligence/cross_lang.rs` are both at ≥ 90%.

**Commands:**
```
cargo test --verbose -p cxpak 2>&1 | tail -5
cargo tarpaulin --out Stdout -- --test-threads=1 2>&1 | grep -E "data_flow|cross_lang|Coverage"
```

---

## Task 19 — Version bump and Cargo.lock regeneration

**Files:**
- `Cargo.toml`
- `plugin/.claude-plugin/plugin.json`
- `.claude-plugin/marketplace.json`
- `Cargo.lock` (auto-regenerated)

**Steps:**
1. In `Cargo.toml`, change `version = "1.1.0"` to `version = "1.5.0"`.
2. Update `plugin/.claude-plugin/plugin.json` version field to `"1.5.0"`.
3. Update `.claude-plugin/marketplace.json` version field to `"1.5.0"`.
4. Run `cargo check` to regenerate `Cargo.lock` with the new version.
5. Run the full test suite one final time; confirm all tests pass and there are no warnings under `cargo clippy --all-targets -- -D warnings`.

**Commands:**
```
cargo check 2>&1 | tail -5
cargo test --verbose 2>&1 | tail -10
cargo clippy --all-targets -- -D warnings 2>&1 | grep -E "^error|^warning" | head -20
cargo fmt -- --check 2>&1
```

---

## Dependency Order

```
Task 1 (move EdgeType/TypedEdge)
  └─ Task 2 (add BridgeType + CrossLanguage variant)
       └─ Task 3 (serialization tests)
            ├─ Task 4 (DataFlow types)
            │    └─ Task 14 (call graph guard — MUST precede Task 5)
            │         └─ Task 5 (trace_data_flow logic)
            │              └─ Task 12 (MCP data_flow tool)
            │                   └─ Task 15 (HTTP handlers)
            └─ Task 6 (CrossLangEdge type)
                 ├─ Task 7 (HTTP bridge detection)
                 └─ Task 8 (FFI/gRPC/GraphQL/Schema/Exec detection)
                      └─ Task 9 (inject into DependencyGraph)
                           └─ Task 10 (field on CodebaseIndex)
                                ├─ Task 11 (auto_context integration)
                                ├─ Task 13 (MCP cross_lang tool)
                                └─ Task 17 (integration test)

Task 16 (cache version bump) — can run any time after Task 2
Task 18 (property tests + coverage) — runs after Tasks 5, 7, 8
Task 19 (version bump) — runs last
```

---

## Test Checklist (≥90% coverage requirement)

| Module | Required tests |
|---|---|
| `src/index/graph.rs` | `CrossLanguage` hash, serialization, `add_edge`, `build_dependency_graph` with cross-lang slice |
| `src/intelligence/data_flow.rs` | direct trace, depth truncation, passthrough classification, speculative confidence, no call graph guard, cycle termination, module boundary flag, language boundary flag, depth zero |
| `src/intelligence/cross_lang.rs` | HTTP detection match, HTTP no-match, FFI, gRPC, GraphQL, SharedSchema, CommandExec, empty index, focus filter |
| `src/intelligence/call_graph.rs` (stub) | types compile; `CallGraph::default()` is accessible |
| `src/auto_context/mod.rs` | cross-lang edges appear in packed result |
| `src/commands/serve.rs` | MCP `data_flow` happy path, missing symbol error, MCP `cross_lang` happy path, file filter |
| `tests/integration_cross_lang.rs` | full pipeline, edge count, bridge type, auto_context output |
