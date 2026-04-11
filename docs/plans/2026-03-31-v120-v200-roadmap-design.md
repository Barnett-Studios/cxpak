# cxpak v1.2.0 → v2.0.0 Roadmap Design

> **Thesis:** Compound intelligence. cxpak already has 10+ intelligence primitives (PageRank, blast radius, conventions, test mapping, schema awareness, embeddings, progressive degradation, query expansion, noise filtering, relevance scoring). Wire them together to produce insights that would take a senior engineer days to produce manually. Then make it visible.

**Current state:** v1.1.0 shipped. ~54K lines of hand-written Rust (excluding generated tree-sitter grammars), 1,202 tests, 42 languages, 13 MCP tools, 11-stage pipeline.

**Goal:** Become THE code intelligence tool. Not a feature checklist — compound intelligence that no competitor can replicate.

---

## Version Overview

| Version | Codename | Headline | New MCP Tools |
|---|---|---|---|
| v1.2.0 | Codebase Health | Health score, risk map, briefing mode, incremental indexing | health, risks, briefing |
| v1.3.0 | Deep Understanding | Call graph, dead code, architecture quality, monorepo | call_graph, dead_code, architecture |
| v1.4.0 | Prediction | Change impact, architecture drift, security surface | predict, drift, security_surface |
| v1.5.0 | Deep Flow | Data flow analysis, cross-language symbol resolution | data_flow, cross_lang |
| v1.6.0 | The Platform | LSP server, Intelligence API, convention export standard | — |
| **v2.0.0** | **The Experience** | **Visual intelligence dashboard, onboarding map, plugin SDK** | visual, onboard |

Final tool count: 26 MCP tools, 14 LSP custom methods, HTTP Intelligence API, CLI, plugin SDK. One Rust binary.

---

## v1.2.0 — "Codebase Health"

### Goal

auto_context becomes a compound intelligence engine. Same structure, richer data. Every call returns health, risks, architecture, co-changes alongside packed source.

### New Fields on `AutoContextResult`

```rust
pub struct AutoContextResult {
    // existing
    pub task: String,
    pub dna: String,
    pub budget: BudgetSummary,
    pub sections: PackedSections,
    pub filtered_out: Vec<FilteredFile>,

    // NEW — v1.2.0
    pub health: HealthScore,
    pub risks: Vec<RiskEntry>,
    pub architecture: ArchitectureMap,
    pub co_changes: Vec<CoChangeEdge>,
    pub recent_changes: Vec<RecentChange>,
}
```

### Health Score

Compound metric, 6 dimensions, each scored 0.0–10.0:

| Dimension | Source | Computation |
|-----------|--------|-------------|
| `conventions` | conventions module | % adherence across all pattern categories |
| `test_coverage` | test_map | ratio of source files with ≥1 mapped test file |
| `churn_stability` | git_health | inverse of high-churn file ratio (>10 changes/30d) |
| `coupling` | dependency graph | 1 - mean cross-module edge ratio |
| `cycles` | graph DFS (Tarjan's SCCs) | 10.0 / (1.0 + scc_count) — logarithmic, not clamped |
| `dead_code` | null in v1.2, populated in v1.3 | 1 - (dead symbol ratio) |

Composite: weighted average. `conventions: 0.20, tests: 0.20, churn: 0.15, coupling: 0.20, cycles: 0.15, dead_code: 0.10`.

**Handling null `dead_code` in v1.2.0:** When `dead_code` is null (before v1.3.0 adds call graph), the composite uses the remaining 5 dimensions with weights renormalized to sum to 1.0 (each weight / 0.90). This means a repo's composite score may shift slightly when upgrading to v1.3.0, which is acceptable — the new dimension adds real information.

**Module inclusion threshold for coupling dimension:** Only modules with ≥3 files are included in the mean coupling calculation. Single-file "modules" (like `src/main.rs`) would distort the mean. When no modules meet the threshold, the coupling dimension returns 10.0 (healthy). When a qualifying module has 0 total edges (incident from both directions), coupling = 0.0 (fully isolated).

**Cross-version score comparability:** Composite scores from v1.2.0 and v1.3.0 are NOT directly comparable due to the dead_code dimension activation. Document this in tool output.

**Mode behavior:**
- Full mode: composite + all dimension scores
- Briefing mode: composite only

### Risk Ranking

Per-file, sorted descending by risk score:

```rust
pub struct RiskEntry {
    pub path: String,
    pub churn_30d: u32,
    pub blast_radius: usize,
    pub test_coverage: f64,  // 0.0 = no tests, 1.0 = has tests
    pub risk_score: f64,
}
```

Formula: `risk = max(norm_churn, 0.01) × max(norm_blast, 0.01) × max(1.0 - test_coverage, 0.01)`

Multiplicative with floor of 0.01 — no single zero can hide a genuinely risky file, but the floor is small enough to avoid inflating zero-input files. Effective range: [0.000001, 1.0].

**Normalization strategy:**
- `norm_churn`: percentile rank across all files in the repo (robust against outliers — a single file with 100 commits doesn't compress all others to near zero)
- `norm_blast`: `blast_radius_count / total_files` (semantically meaningful — what fraction of the codebase is affected)
- `test_coverage`: binary 0.0 (no mapped test files) or 1.0 (has ≥1 mapped test file). Future versions may use line coverage when available.

Top 10 in auto_context output. Full list via `cxpak_risks` tool.

**Naming distinction:** This is "standing risk" (inherent file-level risk regardless of current changes). The existing `compute_risk()` in blast_radius.rs computes "change risk" (risk to a specific file from a specific set of changes). Both are valid, different concepts. The field names and tool descriptions must distinguish them.

### Co-change Analysis

Git log mining. Files appearing in ≥3 commits together within 180 days. Threshold of ≥3 co-commits filters noise (single co-occurrence is not a pattern).

Decay-weighted by recency. Per-commit weight formula: `weight(days_ago) = 1.0` for days_ago ≤ 30, `weight(days_ago) = 1.0 - 0.7 × (days_ago - 30) / 150` for 30 < days_ago ≤ 180. Commits older than 180 days excluded. Edge `recency_weight` = weight of the most recent co-commit (not average — the latest co-occurrence is the best signal of current relevance).

```rust
pub struct CoChangeEdge {
    pub file_a: String,
    pub file_b: String,
    pub count: u32,
    pub recency_weight: f64,
}
```

**Data storage:** Co-change data is computed during index construction (piggybacking on the existing `git_health` git walk) and stored on `CodebaseIndex` as `pub co_changes: Vec<CoChangeEdge>`. The auto_context pipeline includes it in the result from the index — no separate computation step needed.

### Architecture Map

Module = directory prefix (first two path segments, e.g., `src/api`). For projects with non-standard structure (flat layouts, deep nesting), the module prefix depth is configurable via `.cxpak.json` (`"module_depth": 2`). Default is 2.

```rust
pub struct ArchitectureMap {
    pub modules: Vec<ModuleInfo>,
    pub circular_deps: Vec<Vec<String>>,  // each cycle as ordered path list
}

pub struct ModuleInfo {
    pub prefix: String,
    pub file_count: usize,
    pub aggregate_pagerank: f64,
    pub coupling: f64,  // cross-module edge ratio: 0.0 = isolated, 1.0 = fully coupled
}
```

Circular dependency detection via Tarjan's SCC algorithm on the dependency graph (O(V+E), not full cycle enumeration which can be exponential). Each strongly connected component with >1 node is a circular dependency group. Reported as an ordered list of file paths per SCC. Only forward edges (not reverse_edges) are followed.

### Incremental Indexing

Hybrid approach:
- **File-level invalidation for parsing:** Track mtime/size per file. On re-index, only re-parse changed files via the existing mutation API.
- **Full recompute for graph-derived scores:** Rebuild PageRank, blast radius, coupling from the updated graph. Graph algorithms are fast (milliseconds for 10K files); tree-sitter parsing is the real bottleneck.

**Implementation:** Uses the existing mutation API — NOT a new `build()` parameter. The approach:
1. Scan all files, compare mtime/size against the previous index (stored as `HashMap<String, (u64, SystemTime)>` on `CodebaseIndex` — mtime must be added to `IndexedFile`).
2. Call `upsert_file()` for changed/new files (fresh disk read + tree-sitter parse).
3. Call `remove_file()` for deleted files.
4. Call `rebuild_graph()` once to recompute the dependency graph.
5. Full recompute of PageRank, co-changes, health score, etc. from the updated graph.

This avoids a second-generation memory overlap and eliminates stale-content risks. The mutation API already exists on `CodebaseIndex`.

### Recency Scoring

Signal #5 weight changes from 0.0 to 0.05. Both weight configurations must be updated:

**Without embeddings (6 signals):**
- path_similarity: 0.18, symbol_match: 0.32, import_proximity: 0.14, term_frequency: 0.14 (was 0.19), recency_boost: 0.05 (was 0.0), pagerank: 0.17

**With embeddings (7 signals):**
- path_similarity: 0.15, symbol_match: 0.27, import_proximity: 0.12, term_frequency: 0.11 (was 0.16), recency_boost: 0.05 (was 0.0), pagerank: 0.15, embedding_similarity: 0.15

Both sum to 1.0.

Source: git log, most recent commit touching each file.
Score: `1.0` for files changed today, linearly decaying to `0.0` at 90 days.

### auto_context Mode Parameter

`mode: "full"` (default — current behavior + new compound intelligence fields) or `mode: "briefing"` (new compound intelligence fields + file list with scores instead of packed source content).

Both modes have identical structure. Briefing mode sets `content: None` on packed files (the `content` field on `PackedFile` changes from `String` to `Option<String>` — `Some(content)` in full mode, `None` in briefing mode). The LLM calls `cxpak_pack_context` for files it needs. This is a type-level distinction, not a convention based on empty strings.

### New MCP Tools

- `cxpak_health` — returns health score for the repo (or focus path). Parameters: `focus` (optional).
- `cxpak_risks` — returns full risk-ranked file list. Parameters: `focus` (optional), `limit` (default 20).
- `cxpak_briefing` — alias for `cxpak_auto_context` with `mode: "briefing"`. Parameters: `task`, `tokens`, `focus`.

### New CLI Output

`cxpak overview` gains a `--health` flag that appends the health score summary to the output.

---

## v1.3.0 — "Deep Understanding"

### Goal

Go beyond imports. Cross-file call graph enables dead code detection, architecture quality scoring, and precise dependency understanding.

### Call Graph

New module: `src/intelligence/call_graph.rs`

Hybrid approach:
- **Tier 1 languages (26):** Tree-sitter extraction of call expressions within function bodies. Match call targets against known symbols via import resolution. Produces precise edges.
- **Tier 2 languages (14):** Regex scan of function bodies for references to known symbol names. Higher false positive rate, tagged as `confidence: Approximate`.

```rust
pub struct CallGraph {
    pub edges: Vec<CallEdge>,
    pub unresolved: Vec<UnresolvedCall>,
}

pub struct CallEdge {
    pub caller_file: String,
    pub caller_symbol: String,
    pub callee_file: String,
    pub callee_symbol: String,
    pub confidence: CallConfidence,  // Exact | Approximate
}

pub enum CallConfidence {
    Exact,        // tree-sitter extracted, import-resolved
    Approximate,  // regex-matched against known symbols
}
```

The call graph is computed after index construction, stored on `CodebaseIndex` alongside the existing `DependencyGraph`. Uses the import graph to resolve which file a called symbol lives in.

**Incremental language rollout:** Call graph extraction is a significant new capability per language parser. Initial v1.3.0 ships with call extraction for the top 10 Tier 1 languages (Rust, TypeScript, JavaScript, Python, Java, Go, C, C++, Ruby, C#). Remaining Tier 1 languages added in subsequent patches. Tier 2 languages use regex from day one.

### Dead Code Detection

New module: `src/intelligence/dead_code.rs`

A symbol is dead when ALL of the following hold:
- Zero callers in the call graph
- Not an entry point: main function, HTTP handler (from api_surface route detection), test function, trait implementation, pub export from lib root
- Not referenced in test files (via test_map + call graph)

```rust
pub struct DeadSymbol {
    pub file: String,
    pub symbol: String,
    pub kind: SymbolKind,
    pub liveness_score: f64,
    pub reason: String,
}
```

Output is **deterministic binary classification** — a symbol is dead or it isn't. The `liveness_score` is metadata for SORTING dead symbols by importance, not a threshold. Since all dead symbols have zero callers by definition, the sorting formula uses different factors: `liveness_score = pagerank × (1.0 + test_file_count) × export_weight` where `export_weight` is 2.0 for pub exports, 1.0 otherwise. Higher-scoring dead symbols are more concerning (important file, has test file nearby, publicly exported — yet never called). "47 dead symbols found, sorted by importance."

### Architecture Quality

Extends `ModuleInfo` from v1.2.0 with five metrics:

```rust
pub struct ModuleInfo {
    // from v1.2.0
    pub prefix: String,
    pub file_count: usize,
    pub aggregate_pagerank: f64,
    pub coupling: f64,

    // NEW — v1.3.0
    pub cohesion: f64,
    pub boundary_violations: Vec<BoundaryViolation>,
    pub god_files: Vec<String>,
}

pub struct BoundaryViolation {
    pub source_file: String,
    pub target_file: String,
    pub target_module: String,
    pub edge_type: EdgeType,  // typed, not stringly-typed
}
```

**Five metrics:**

1. **Coupling** (v1.2.0) — ratio of cross-module edges to total edges per module. 0.0 = fully isolated, 1.0 = fully coupled.
2. **Cohesion** — ratio of intra-module edges to total possible intra-module edges. High = module's files talk to each other. Low = bag of unrelated files.
3. **Circular dependency count** — number of import cycles (from v1.2.0 DFS).
4. **Boundary violations** — files that import non-root files from other modules. Root = `mod.rs`, `index.ts`, `__init__.py`, or the barrel file for that module. A file in module A importing `src/db/internal/pool.rs` instead of `src/db/mod.rs` is a violation.
5. **God file detection** — files with inbound edge count (import + call) > mean + 2σ across all files in the module.

### Monorepo Support

New parameter on all commands and MCP tools: `workspace` (optional path prefix).

When set:
- Scanner only walks files under that prefix
- All paths are relative to workspace root
- Each workspace gets its own cache namespace in `.cxpak/cache/`
- Multiple workspaces can share the same `cxpak serve` instance

### Health Score Update

`dead_code` dimension now populated: `10.0 × (1.0 - dead_symbol_ratio)`.

### New MCP Tools

- `cxpak_call_graph` — returns call graph for a file or symbol. Parameters: `target` (file path or symbol name), `depth` (default 1), `focus`.
- `cxpak_dead_code` — returns dead symbol list, sorted by liveness score. Parameters: `focus`, `limit` (default 50).
- `cxpak_architecture` — returns full architecture quality report. Parameters: `focus`.

---

## v1.4.0 — "Prediction"

### Goal

Predict what happens when you change code. Not just "what depends on this" (blast radius) but "what will break, what's drifting, and what's exposed."

### Change Impact Prediction

New module: `src/intelligence/predict.rs`

Given changed files, returns predictions from three independent signals:

```rust
pub struct PredictionResult {
    pub changed_files: Vec<String>,
    pub structural_impact: Vec<ImpactEntry>,   // from blast radius
    pub historical_impact: Vec<ImpactEntry>,   // from co-change
    pub call_impact: Vec<ImpactEntry>,         // from call graph
    pub test_impact: Vec<TestPrediction>,      // merged from all three
    pub confidence_summary: String,
}

pub struct ImpactEntry {
    pub path: String,
    pub signal: ImpactSignal,  // Structural | Historical | CallBased
    pub score: f64,
}

pub struct TestPrediction {
    pub test_file: String,
    pub test_function: Option<String>,
    pub signals: Vec<ImpactSignal>,
    pub confidence: f64,
}
```

**Signal merging for test impact (all 7 non-empty subsets):**
- Co-change alone → confidence 0.3
- Test map alone → confidence 0.4
- Call graph alone → confidence 0.5
- Test map + co-change → confidence 0.5
- Call graph + co-change → confidence 0.6
- Test map + call graph → confidence 0.7
- Test map + call graph + co-change → confidence 0.9

Tests flagged by multiple independent signals are ranked higher. The LLM gets a deterministic ranked list with clear provenance for each prediction.

### Architecture Drift Detection

New module: `src/intelligence/drift.rs`

Dual approach — stored baseline + time-window trend:

```rust
pub struct DriftReport {
    pub baseline: Option<BaselineComparison>,
    pub trend: TrendComparison,
    pub hotspots: Vec<DriftHotspot>,
}

pub struct BaselineComparison {
    pub baseline_date: String,
    pub metrics_then: ArchitectureMetrics,
    pub metrics_now: ArchitectureMetrics,
    pub deltas: MetricDeltas,
}

pub struct TrendComparison {
    pub window_recent: String,   // "last 30 days"
    pub window_baseline: String, // "30-180 days ago"
    pub new_cross_module_imports: Vec<BoundaryViolation>,
    pub coupling_trend: f64,     // positive = getting worse
    pub cohesion_trend: f64,     // negative = getting worse
    pub new_cycles: Vec<Vec<String>>,
}

pub struct DriftHotspot {
    pub module: String,
    pub issue: String,
    pub severity: f64,
    pub contributing_commits: Vec<String>,
}
```

**Baseline storage:** `.cxpak/baseline.json` — saved on first `cxpak_drift` call or explicitly via `cxpak drift --save-baseline`. Reset with `cxpak clean`.

**Time-window trend:** Uses stored architecture snapshots, NOT git diff reconstruction (which is infeasible without re-parsing). On each `cxpak serve` index build or `cxpak overview` run, an architecture snapshot is auto-saved to `.cxpak/snapshots/` (~few KB: module list + edge counts + metric values + timestamp). The trend comparison diffs the most recent snapshot against snapshots from 30 and 180 days ago. If no historical snapshots exist, trend returns null with "Insufficient snapshot history."

**Bootstrap edge case:** For repos younger than 30 days, the trend report returns `null` with a message: "Insufficient history for trend analysis (requires >30 days)." For repos between 30-180 days, the baseline window uses all available history before the 30-day window.

### Security Surface Analysis

New module: `src/intelligence/security.rs`

Five deterministic detections:

```rust
pub struct SecuritySurface {
    pub unprotected_endpoints: Vec<UnprotectedEndpoint>,
    pub input_validation_gaps: Vec<ValidationGap>,
    pub secret_patterns: Vec<SecretPattern>,
    pub sql_injection_surface: Vec<SqlInjectionRisk>,
    pub exposure_scores: Vec<ExposureEntry>,
}
```

**1. Unprotected endpoints:** HTTP routes (from api_surface) where the call chain from handler to route registration does NOT pass through a known auth middleware/decorator. Known patterns per language: `auth`, `authenticate`, `authorize`, `require_auth`, `login_required`, `@authenticated`, `#[guard]`, etc. The auth pattern list is configurable via `.cxpak.json` (`"auth_patterns": [...]`) to support custom middleware names.

**Prerequisite:** The current `RouteEndpoint.handler` field is always the literal `"handler"` — actual handler function names are not extracted. v1.3.0 (call graph) must fix `detect_routes()` to extract real handler names per framework before this detection can work. Detection strategies per auth model: decorator-stack (Python/Flask/Django), middleware-argument (Express/Koa), annotation-based (Spring/Java), guard-extractor (Actix/Rust). The call graph must store both caller→callee and callee→caller directions to support reverse reachability queries.

**2. Input validation gaps:** Public functions accepting String/str parameters where the function body contains no validation calls (regex, parse, validate, check, sanitize). Scoped to files with high PageRank.

**3. Secret patterns:** Per-type regex patterns (not generic entropy matching):
- AWS access key: `AKIA[0-9A-Z]{16}`
- GitHub PAT: `ghp_[a-zA-Z0-9]{36}`
- Generic password assignment: `(password|secret|api_key|token)\s*[:=]\s*["'][^"']{8,}["']`
- Connection strings with credentials: `://[^:]+:[^@]+@`
- Slack token: `xox[baprs]-[0-9a-zA-Z-]{10,}`

Exclude: test files, `.env.example`, documentation, lock files (`Cargo.lock`, `package-lock.json`, `yarn.lock`, `Gemfile.lock`, `poetry.lock`). Each match tagged with file, line, pattern name. Custom patterns configurable via `.cxpak.json` (`"secret_patterns": [...]`).

**4. SQL injection surface:** Files with `embedded_sql` edges where the SQL string uses string interpolation/concatenation rather than parameterized queries. Language-specific: f-strings (Python), template literals (JS/TS), `format!` (Rust), `+` concatenation (Java).

**5. Exposure score:** Per-file: `(pub_symbol_count × inbound_edges × (1 - test_coverage)) / max_possible`. Normalized to [0, 1].

```rust
pub struct ExposureEntry {
    pub path: String,
    pub pub_symbol_count: usize,
    pub inbound_edges: usize,
    pub test_coverage: f64,
    pub exposure_score: f64,
}
```

### auto_context Integration

When `mode: "full"`, auto_context includes a `predictions` field if the task description mentions changing specific files. When `mode: "briefing"`, includes a one-line drift summary and top 3 exposure scores.

### New MCP Tools

- `cxpak_predict` — given changed files, returns impact + test predictions. Parameters: `files` (list of paths), `focus`.
- `cxpak_drift` — returns architecture drift report. Parameters: `save_baseline` (bool), `focus`.
- `cxpak_security_surface` — returns full security surface analysis. Parameters: `focus`.

---

## v1.5.0 — "Deep Flow"

### Goal

Trace how values move through the system. Cross-language symbol resolution for polyglot codebases.

### Data Flow Analysis

New module: `src/intelligence/data_flow.rs`

Structural data flow — traces named values through function parameters, return values, and assignments using the call graph + symbol extraction. NOT full taint analysis.

```rust
pub struct DataFlowResult {
    pub source: FlowNode,
    pub sink: Option<FlowNode>,
    pub paths: Vec<FlowPath>,
    pub truncated: bool,
}

pub struct FlowPath {
    pub nodes: Vec<FlowNode>,
    pub crosses_module_boundary: bool,
    pub crosses_language_boundary: bool,
    pub touches_security_boundary: bool,
    pub confidence: FlowConfidence,  // Exact, Approximate, or Speculative
    pub length: usize,
}

pub enum FlowConfidence {
    Exact,        // all hops resolved via imports + direct calls
    Approximate,  // some hops resolved via regex or name matching
    Speculative,  // path crosses dynamic dispatch, closure, or HOF boundary
}

pub struct FlowNode {
    pub file: String,
    pub symbol: String,
    pub parameter: Option<String>,
    pub language: String,
    pub node_type: FlowNodeType,
}

pub enum FlowNodeType {
    Source,       // API input, file read, env var
    Transform,    // parsed, validated, formatted
    Sink,         // DB write, HTTP response, file write
    Passthrough,  // value passes through unchanged
}
```

**How it works:**
1. Start from a symbol (e.g., `handle_request`)
2. Identify parameters that represent external input (first param of route handlers, params named `input`, `request`, `body`, `data`, `payload`)
3. Follow that parameter through the call graph: if `handle_request` calls `save_user(name)`, and `name` came from the request, trace into `save_user`
4. At each hop, classify the node: transforming the value or passing it through
5. Stop at sinks (DB writes, HTTP responses, file writes) or max depth (default 10)

**Documented limitations (prominently displayed in tool output, not buried):**
- Structural, not runtime — follows static call paths, not dynamic dispatch
- Parameter matching is heuristic — matches by position and name, not type inference
- Doesn't track through collections (value pushed into Vec, iterated later → chain breaks)
- Closures and higher-order functions: if a value is passed to a closure or HOF (`items.map(transform)`), tracing into the closure body is best-effort. Paths crossing closure boundaries are tagged `FlowConfidence::Speculative`
- Trait objects / interfaces / dynamic dispatch: `handler.handle(req)` cannot be resolved to a concrete type. These hops are tagged `Speculative`
- Precise for Tier 1 languages, approximate for Tier 2

### Cross-Language Symbol Resolution

New module: `src/intelligence/cross_lang.rs`

```rust
pub struct CrossLangEdge {
    pub source_file: String,
    pub source_symbol: String,
    pub source_language: String,
    pub target_file: String,
    pub target_symbol: String,
    pub target_language: String,
    pub bridge_type: BridgeType,
}

pub enum BridgeType {
    HttpCall,       // fetch("/api/users") → route handler
    FfiBinding,     // extern "C", ctypes, JNI, napi
    GrpcCall,       // gRPC client → service definition
    GraphqlCall,    // GraphQL query → schema type
    SharedSchema,   // both languages reference same DB table
    CommandExec,    // subprocess/exec calling binary
}
```

**Detection:**
- **HTTP:** Match fetch/axios/reqwest URL patterns against known routes from api_surface
- **FFI:** Detect `extern`, `ctypes`, `@JvmStatic`, `napi::bindgen` patterns per language
- **gRPC:** Match client stubs against service definitions in `.proto` files
- **GraphQL:** Match query/mutation names against schema type definitions
- **Shared schema:** Two files in different languages with `embedded_sql` or `orm_model` edges to the same table
- **Command exec:** Detect `exec`, `spawn`, `subprocess.run` with string arguments matching known binary names

These edges are added to `DependencyGraph` as `EdgeType::CrossLanguage(BridgeType)`.

**EdgeType enum migration:** The current `EdgeType` and `TypedEdge` both live in `src/schema/mod.rs` and derive `PartialEq, Eq, Hash, Serialize, Deserialize`. Adding `CrossLanguage(BridgeType)` changes serialization and hash behavior. Migration path:
1. Move BOTH `EdgeType` AND `TypedEdge` from `src/schema/mod.rs` to `src/index/graph.rs` (where `DependencyGraph` lives). Moving only `EdgeType` creates a circular import: `src/index/mod.rs` imports `crate::schema::SchemaIndex` which references `TypedEdge` which contains `EdgeType`.
2. Re-export both from `src/schema/mod.rs` for backward compatibility: `pub use crate::index::graph::{EdgeType, TypedEdge};`
3. `BridgeType` must derive `PartialEq, Eq, Hash, Serialize, Deserialize` (required by `EdgeType`'s derives).
4. Bump cache version to invalidate `.cxpak/cache/` — force full re-index on first v1.5.0 run.

### auto_context Integration

Data flow paths that cross security boundaries are included in the security surface. Cross-language edges appear in the architecture map with a `cross_language: true` flag.

### New MCP Tools

- `cxpak_data_flow` — trace a value from source to sink(s). Parameters: `symbol`, `sink` (optional), `depth` (default 10), `focus`.
- `cxpak_cross_lang` — list all cross-language boundaries. Parameters: `file` (optional), `focus`.

---

## v1.6.0 — "The Platform"

### Goal

cxpak goes from tool to infrastructure. Other tools query its intelligence. Every IDE gets native access. Conventions become a portable standard.

### LSP Server

New module: `src/lsp/`

A **supplementary LSP** — not autocomplete or syntax highlighting, but intelligence no other language server provides.

**Standard LSP methods:**

| Method | Returns |
|--------|---------|
| `textDocument/codeLens` | Health score per file, risk score, dead code markers, security warnings |
| `textDocument/diagnostic` | Convention violations, boundary violations, circular dep participation, dead symbols |
| `textDocument/hover` | On a symbol: PageRank, callers/callees count, blast radius, test coverage, liveness |
| `workspace/symbol` | Augmented with PageRank and liveness — dead symbols flagged |

**Custom LSP methods (14):**

| Method | Returns |
|--------|---------|
| `cxpak/health` | Health score (composite + breakdown) |
| `cxpak/risks` | Top risk files |
| `cxpak/architecture` | Module map, coupling, cohesion, god files |
| `cxpak/callGraph` | Callers/callees for a symbol |
| `cxpak/deadCode` | Dead symbols list |
| `cxpak/predict` | Change impact for dirty files |
| `cxpak/drift` | Architecture drift report |
| `cxpak/securitySurface` | Security surface analysis |
| `cxpak/dataFlow` | Trace a value from source to sink |
| `cxpak/crossLang` | Cross-language boundaries |
| `cxpak/conventions` | Convention profile |
| `cxpak/briefing` | Compact intelligence summary |
| `cxpak/coChanges` | Co-change history for a file |
| `cxpak/blastRadius` | Blast radius for a file |

**Architecture:**
- Runs as `cxpak lsp` command over stdio (standard LSP transport)
- Maintains hot index via file watcher (reuses `cxpak watch` infrastructure)
- Incremental updates (from v1.2.0) keep it responsive
- Can run alongside the language's own LSP
- **Feature flag:** `lsp = ["dep:tower-lsp", "daemon"]` — LSP depends on daemon feature (already gates tokio/axum). Added to `default` features. Verify `tower-lsp` version compatibility with `axum = "0.8"` (both use tower/hyper — check `http` crate version conflicts) before implementation.

### Intelligence API

HTTP serve mode extended with versioned endpoints:

```
POST /v1/health              → HealthScore
POST /v1/risks               → Vec<RiskEntry>
POST /v1/architecture        → ArchitectureMap
POST /v1/call_graph          → CallGraph
POST /v1/dead_code           → Vec<DeadSymbol>
POST /v1/predict             → PredictionResult
POST /v1/drift               → DriftReport
POST /v1/security_surface    → SecuritySurface
POST /v1/data_flow           → DataFlowResult
POST /v1/cross_lang          → Vec<CrossLangEdge>
POST /v1/conventions         → ConventionProfile
POST /v1/briefing            → AutoContextResult (briefing mode)
```

All endpoints accept `workspace` and `focus` parameters. All responses are JSON with stable schemas. Versioned prefix (`/v1/`) — breaking changes require a major version bump.

**Security:** Default bind to `127.0.0.1` (localhost only). `--bind` flag for non-local deployments. Optional `--token` flag enables `Authorization: Bearer <token>` validation on all endpoints. No TLS built-in (use a reverse proxy for HTTPS). File path parameters validated against workspace root — reject paths containing `..` or absolute paths that escape the repo.

### Convention Export Standard

Conventions become a portable, versionable artifact:

```rust
pub struct ConventionExport {
    pub version: String,           // "1.0"
    pub generated_at: String,      // ISO 8601
    pub generator: String,         // "cxpak 1.6.0"
    pub repo: String,
    pub profile: ConventionProfile,
    pub checksum: String,           // SHA256 of profile content
}
```

File: `.cxpak/conventions.json`

**Use cases:**
- Commit to repo — new team members see conventions instantly
- Diff between branches — "this PR changes 3 conventions"
- CI enforcement — compare current vs committed baseline, fail on drift
- Cross-repo comparison

**CLI commands:**
- `cxpak conventions export .` — write `.cxpak/conventions.json`
- `cxpak conventions diff .` — compare current vs exported

### New CLI Commands

- `cxpak lsp` — start LSP server
- `cxpak conventions export` — write convention export
- `cxpak conventions diff` — compare current vs exported

---

## v2.0.0 — "The Experience"

### Goal

Make all the intelligence from v1.2–v1.6 visible in one place. Not a graph viewer — an **intelligence dashboard** that uses visual representations to surface what matters. Every pixel encodes meaning. The capstone.

### Architecture

**Layout pre-computed in Rust** using a simplified Sugiyama algorithm (~500-800 lines of Rust): layer assignment via `petgraph` topological sort, barycenter crossing minimization, Brandes-Kopf coordinate assignment. For intra-module layout, grid/pack. No Rust ELK crate exists — this is the pragmatic approach. The HTML opens instantly — no force simulation in the browser, just rendering pre-computed positions.

**Self-contained HTML.** Custom D3.js bundle (~100KB minified, only `d3-hierarchy`, `d3-zoom`, `d3-transition`, `d3-scale`, `d3-selection`, `d3-shape`, `d3-color`, `d3-interpolate`) inlined via `include_str!`. No CDN, no npm, no build step. Single file. Total: ~200KB-1MB depending on graph data size.

**Scale:** The 7±2 grouping constraint means D3 with SVG never renders more than ~50 visible nodes at any zoom level. No WebGL/Sigma.js needed — D3 handles grouped views comfortably. If a user requests an ungrouped "all files" view for a 10K-file repo, render as a static SVG (poster mode, no interactivity).

**PNG export:** `resvg` crate (pure Rust SVG rasterizer, ~2MB binary size increase) as optional dependency behind `visual` feature flag. `cxpak visual --format png` generates a static SVG (no JS), then rasterizes via `resvg`.

**Feature flags:** `visual = ["dep:resvg"]` and `plugins = ["dep:wasmtime"]` — both optional, both in `default` features.

### Six Views

#### 1. Dashboard

The entry point. Four quadrants, every number clickable:

- **Top left:** Health score (big number + sparkline trend) with dimension breakdown
- **Top right:** Top risks (5 files, colored by severity)
- **Bottom left:** Architecture overview (module graph preview, click to explore)
- **Bottom right:** Alerts (circular deps, dead symbols, unprotected endpoints, coupling trend)

#### 2. Architecture Explorer

C4-inspired semantic zoom with three abstraction levels:

- **Level 1 (zoomed out):** Module groups. Boxes sized by aggregate PageRank. Colored by health sub-score. Arrows = cross-module edge count (thickness). Circular deps in red. Click to zoom.
- **Level 2 (module):** Files within the module. Sized by token count. Colored by risk score. Edges = imports + calls. God files glow. Dead code files dimmed. Click to zoom.
- **Level 3 (file):** Symbols within the file. Sized by symbol importance. Call graph edges to/from other files. Convention violations highlighted. Data flow paths traced.

Smooth animated transitions between levels (D3 transitions). Breadcrumb navigation. Search bar filters across all levels.

**Cognitive load management:** Max 7±2 items per level. Groups the rest into "others" expandable cluster.

#### 3. Risk Heatmap

Treemap layout:
- Rectangle size = blast radius
- Color = risk score (green → yellow → red)
- Nested: modules contain files
- Hover tooltip: churn, blast radius, test count, coupling
- Click: explodes into blast radius view — radial graph showing all affected files

#### 4. Flow Diagram

For a specific data flow trace:
- Nodes = functions in the flow path, laid out left to right
- Color by FlowNodeType: Source (blue), Transform (yellow), Sink (red), Passthrough (gray)
- Cross-language boundaries = dashed vertical divider with language labels
- Security checkpoints = green shield icon
- Missing security = red warning on the edge
- Click node → code snippet

#### 5. Time Machine

Git history animation of architecture evolution:
- Scrubber at bottom: drag through commit history
- Architecture graph animates: files appear/disappear, edges form/break
- Health score sparkline animates alongside
- Key moments highlighted: "circular dependency introduced here" (commit SHA, red flash)
- Drift overlay: baseline vs current delta
- Play/pause/speed controls

**Implementation:** Does NOT re-index at every commit. Pre-computes architecture snapshots at sampled intervals (every 10th commit or weekly) during `cxpak visual --type timeline` generation. Snapshots are file-list + import-graph level only (no full parse), computed from git diff deltas. Cached in `.cxpak/timeline/` (~200KB per snapshot, ~20MB for 100 snapshots).

**Scrubber behavior:** Discrete stepper, not continuous interpolation. Snaps to nearest snapshot. Smooth D3 transitions (~300ms) between adjacent snapshots handle node enter/exit animations. 100 discrete steps with smooth transitions feels continuous to the user while being dramatically simpler to implement than real-time interpolation.

#### 6. Diff View

Before/after for pending changes:
- Split screen: left = current, right = after changes
- Blast radius highlighted on "after" side
- New risks in red overlay
- New circular deps shown
- Convention violations in changed files
- "Impact score" for the overall change

### Cognitive Load Management

Working memory = 4 chunks (cognitive science). Every view enforces this:

- Dashboard: 4 quadrants
- Architecture Explorer: max 7±2 items per level, rest grouped
- Risk Heatmap: top 10 files prominent, rest small
- Flow Diagram: max 10 nodes per path, collapses passthrough chains
- Time Machine: highlights max 3 key moments per range

Smart defaults show only what matters. Filtering always available.

### Multi-Format Export

Every view exports to:

| Format | Use Case |
|--------|----------|
| Interactive HTML | Share, demo, embed |
| Mermaid | Commit to docs, render in GitHub/GitLab |
| SVG | Print, embed in slides |
| PNG | Quick screenshot |
| C4 DSL | Structurizr import |
| JSON | Programmatic consumption |

`cxpak visual --type architecture --format mermaid .` generates a Mermaid diagram committed to your README. Always accurate because generated from actual code.

### Onboarding Map

"New to this codebase? Read these files in this order."

Computes reading order based on:
1. PageRank — start with the most important files
2. Dependency order — read dependencies before dependents (topological sort)
3. Module grouping — complete one module before the next
4. Complexity progression — simpler files first within each module

```rust
pub struct OnboardingMap {
    pub total_files: usize,
    pub estimated_reading_time: String,
    pub phases: Vec<OnboardingPhase>,
}

pub struct OnboardingPhase {
    pub name: String,
    pub module: String,
    pub rationale: String,
    pub files: Vec<OnboardingFile>,
}

pub struct OnboardingFile {
    pub path: String,
    pub pagerank: f64,
    pub symbols_to_focus_on: Vec<String>,
    pub estimated_tokens: usize,
}
```

### Plugin / Extension SDK

WASM plugins loaded from `.cxpak/plugins/`. Third parties extend cxpak with custom analyzers, outputs, and detectors.

```rust
pub trait CxpakPlugin {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn capabilities(&self) -> Vec<PluginCapability>;
    fn analyze(&self, index: &IndexSnapshot) -> Vec<Finding>;
    fn detect(&self, file: &FileSnapshot) -> Vec<Detection>;
}

pub enum PluginCapability {
    Analyzer,
    Detector,
    OutputFormat(String),
}
```

WASM for sandboxing, cross-platform, no FFI. `wasmtime` crate. Plugins registered in `.cxpak/plugins.json` with name + path + checksum + declared file patterns. Local files only.

**Plugin sandboxing:** Plugins declare which file patterns they need access to (e.g., `"*.py"` for a Python-specific analyzer). The host provides only matching files in `IndexSnapshot` — not the full codebase. File contents are stripped by default; plugins that need raw content must declare `"needs_content": true` in their manifest, which cxpak warns the user about on first load. Return values (`Vec<Finding>`) are size-limited (1MB) to prevent data exfiltration via findings.

### New CLI Commands

- `cxpak visual` — generate visualization. `--type dashboard|architecture|risk|flow|timeline|diff`. `--format html|mermaid|svg|png|c4|json`. Default: `--type dashboard --format html`.
- `cxpak onboard` — generate reading order. `--focus` optional.
- `cxpak plugin list|add <path>` — plugin management.

### New MCP Tools

- `cxpak_visual` — generate visualization, returns HTML string or file path. Parameters: `type`, `format`, `focus`, `symbol` (for flow), `files` (for diff).
- `cxpak_onboard` — onboarding reading order. Parameters: `focus`.

---

## Tool Count Summary

| Version | New MCP Tools | Running Total |
|---|---|---|
| v1.1.0 (current) | — | 13 |
| v1.2.0 | health, risks, briefing | 16 |
| v1.3.0 | call_graph, dead_code, architecture | 19 |
| v1.4.0 | predict, drift, security_surface | 22 |
| v1.5.0 | data_flow, cross_lang | 24 |
| v1.6.0 | — (LSP + Intelligence API) | 24 |
| v2.0.0 | visual, onboard | **26** |

26 MCP tools. 14 LSP custom methods. HTTP Intelligence API with 12 versioned endpoints. CLI with 10+ commands. WASM plugin SDK. One Rust binary.

---

## Testing Strategy

Each version maintains the existing 90% coverage requirement. New modules get:

- Unit tests for every public function
- Integration tests for MCP tool wiring
- Property tests for score normalization (health, risk, liveness)
- Snapshot tests for deterministic output (dead code list, architecture report)
- Regression tests for score stability across index rebuilds

### Version-specific test focus:

- **v1.2.0:** Health score computation, risk formula edge cases, co-change threshold, incremental index correctness (same output as full rebuild)
- **v1.3.0:** Call graph accuracy per language, dead code false positive rate, boundary violation detection, monorepo isolation
- **v1.4.0:** Prediction confidence merging, drift baseline persistence, security pattern detection per language
- **v1.5.0:** Data flow path completeness, cross-language bridge detection, parameter tracking accuracy
- **v1.6.0:** LSP protocol compliance, API response schema validation, convention export round-trip
- **v2.0.0:** Visual output validity (valid HTML/SVG/Mermaid), layout determinism, export format correctness, onboarding order stability

---

## Design Decisions

### Why compound intelligence, not feature parity

Incremental indexing, call graphs, LSP — competitors will have these eventually. The 10x gap comes from COMBINING intelligence primitives into insights no single feature can produce. Health scores, risk maps, change predictions, architecture drift — these are emergent properties of the compound system.

### Why briefing mode is the same structure as full mode

Consistency. The LLM doesn't need to learn two schemas. Briefing mode is just full mode with `content: ""` on packed files. The intelligence layer (health, risks, architecture) is identical in both.

### Why pre-computed layout for visualizations

Force-directed layout in the browser takes 5-30 seconds for large graphs. Users give up. cxpak computes positions in Rust (simplified Sugiyama for inter-module, grid/pack for intra-module), ships them as JSON inside the HTML. The browser just renders. Instant open.

### Why WASM for plugins

Sandboxed execution, cross-platform, no FFI headache. The `wasmtime` crate is mature. Plugins can be written in any language that compiles to WASM. Local files only — no remote loading.

### Why the visual dashboard is v2.0.0

It's the capstone. Every version from v1.2 to v1.6 builds intelligence data. v2.0.0 makes ALL of it visible in one place. The semver bump signals: this is a fundamentally new experience.

---

## Competitive Position After v2.0.0

| Capability | cxpak | Code Pathfinder | Serena | Claude Context | Sourcegraph Cody |
|---|---|---|---|---|---|
| Languages | 42 | 5-10 | 30+ | Any | Any |
| Convention extraction | ✅ | ❌ | ❌ | ❌ | ❌ |
| Blast radius | ✅ | ❌ | ❌ | ❌ | ❌ |
| Health score | ✅ | ❌ | ❌ | ❌ | ❌ |
| Risk ranking | ✅ | ❌ | ❌ | ❌ | ❌ |
| Call graph | ✅ | ✅ | ❌ | ❌ | ❌ |
| Dead code detection | ✅ | ❌ | ❌ | ❌ | ❌ |
| Architecture quality | ✅ | ❌ | ❌ | ❌ | ❌ |
| Change prediction | ✅ | ❌ | ❌ | ❌ | ❌ |
| Architecture drift | ✅ | ❌ | ❌ | ❌ | ❌ |
| Security surface | ✅ | ❌ | ❌ | ❌ | ❌ |
| Data flow | ✅ | ❌ | ❌ | ❌ | ❌ |
| Cross-language | ✅ | ❌ | ❌ | ❌ | ❌ |
| LSP server | ✅ | ❌ | ❌ | ❌ | ❌ |
| Visual dashboard | ✅ | ❌ | ❌ | ❌ | ❌ |
| Token budgeting | ✅ | ❌ | ❌ | ❌ | ❌ |
| Progressive degradation | ✅ | ❌ | ❌ | ❌ | ❌ |
| Schema awareness | ✅ | ❌ | ❌ | ❌ | ❌ |
| Embeddings | ✅ | ❌ | ❌ | ✅ | ✅ |
| MCP tools | 26 | ~5 | ~5 | ~3 | ~5 |
| Single binary | ✅ | ❌ | ❌ | ❌ | ❌ |

One person built this.
