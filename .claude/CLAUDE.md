# CLAUDE.md

## Build & Test

```bash
cargo build              # Build
cargo test --verbose     # Run all tests
cargo fmt -- --check     # Check formatting
cargo clippy --all-targets -- -D warnings  # Lint
```

Pre-commit hooks enforce fmt + clippy + tests. CI enforces 90% coverage via tarpaulin. Install hooks with `bash scripts/install-hooks.sh`.

## Architecture

Pipeline: **Scanner → Parser → Schema → Index → Conventions → Budget → Context Quality → Intelligence → Auto Context → Output**

1. **Scanner** (`src/scanner/`) — walks git-tracked files, detects language from extension
2. **Parser** (`src/parser/`) — tree-sitter extraction of symbols, imports, exports per language
3. **Schema** (`src/schema/`) — detects and indexes the data layer; builds `SchemaIndex` with table/view/ORM/migration metadata; injects typed edges into the dependency graph
4. **Index** (`src/index/`) — builds `CodebaseIndex` with token counts, language stats, typed dependency graph, detected domains, and optional `SchemaIndex`
5. **Budget** (`src/budget/`) — allocates token budget across sections, truncates with omission markers
6. **Context Quality** (`src/context_quality/`) — progressive degradation, query expansion, context annotations
7. **Intelligence** (`src/intelligence/`) — PageRank file importance, blast radius analysis, API surface extraction, test file mapping
8. **Conventions** (`src/conventions/`) — extracts the codebase's actual patterns (naming, imports, errors, dependencies, testing, visibility, functions, git health) as a quantified `ConventionProfile`. Built after index construction via `build_convention_profile()`. Includes `verify.rs` for checking code changes against conventions, `render.rs` for the ~1000 token DNA section included in every `auto_context` call, and `ConventionExport`/`ConventionDiff` for persisting profiles to `.cxpak/conventions.json` (with SHA256 checksum) and diffing against a baseline.
9. **Auto Context** (`src/auto_context/`) — orchestrates the 10-step auto_context pipeline: query expansion, scoring, seed selection, noise filtering, test/schema/blast-radius/API-surface enrichment, budget allocation, and annotation. Step 0 renders a DNA section (~1000 tokens) from the convention profile, deducted from the budget before fill-then-overflow allocation.
10. **Embeddings** (`src/embeddings/`) — local candle inference with all-MiniLM-L6-v2 and remote API providers (OpenAI, Voyage AI, Cohere); builds and queries the vector index for semantic similarity scoring
11. **Output** (`src/output/`) — renders to markdown, JSON, or XML
12. **LSP** (`src/lsp/`) — LSP server over stdio (`cxpak lsp`). `backend.rs` implements `tower_lsp::LanguageServer` with 4 standard methods (codeLens, hover, diagnostic, workspace/symbol) and 14 custom `cxpak/*` JSON-RPC methods. `methods.rs` holds the dispatch logic for custom methods. Reuses `build_index` from the daemon module. Feature flag: `lsp = ["dep:tower-lsp", "daemon"]`
13. **Visual** (`src/visual/`) — interactive dashboards and static diagrams. `layout.rs` implements the Sugiyama layout engine (layer assignment with SCC condensation, barycenter crossing minimization, Brandes-Kopf coordinate assignment, 3-level builders for module/file/symbol graphs, 7±2 cognitive limit clustering). `render.rs` has 6 view renderers (Dashboard, Architecture Explorer, Risk Heatmap, Flow Diagram, Time Machine, Diff View) producing self-contained HTML with inlined D3.js. `export.rs` provides multi-format output (Mermaid, SVG, PNG via resvg, C4 DSL, JSON). `timeline.rs` computes git history snapshots. `onboard.rs` renders onboarding maps as markdown/JSON. Feature flag: `visual = ["dep:resvg", "dep:petgraph", "dep:thiserror"]`
14. **Plugin** (`src/plugin/`) — WASM plugin SDK. `mod.rs` defines the always-compiled types (`PluginCapability`, `IndexSnapshot`, `FileSnapshot`, `Finding`, `Detection`, `CxpakPlugin` trait). `loader.rs` (feature-gated `plugins`) wraps wasmtime with 10MB size limit. `manifest.rs` handles `.cxpak/plugins.json` with SHA-256 checksum verification and file pattern scoping. `security.rs` enforces 1MB return payload limits and content access warnings. Feature flag: `plugins = ["dep:wasmtime"]`
15. **Onboarding** (`src/intelligence/onboarding.rs`) — canonical onboarding logic (feature-gated `visual`). `topological_sort_files` (Kahn's algorithm with lexicographic cycle-break), `group_into_phases` (module grouping, PageRank ordering, 7±2 splitting), `format_reading_time` (200 tokens/min). The visual/onboard.rs module delegates to these functions.

## Commands

- `overview` — structured repo summary within a token budget
- `trace` — finds a target symbol, walks dependency graph, packs relevant code paths
- `conventions export <path>` — writes `.cxpak/conventions.json` (`ConventionExport` with SHA256 checksum)
- `conventions diff <path>` — compares current conventions against `.cxpak/conventions.json` baseline, reports changes
- `lsp [path]` — runs LSP server over stdio (requires `lsp` feature flag)
- `visual [path]` — generates visual dashboards/diagrams. `--visual-type` (dashboard|architecture|risk|flow|timeline|diff), `--format` (html|mermaid|svg|png|c4|json), `--out`, `--symbol` (flow), `--files` (diff). Requires `visual` feature flag.
- `onboard [path]` — generates onboarding reading order. `--format` (markdown|json|xml), `--out`. Requires `visual` feature flag.

### MCP Tools (v2.0.0 — 26 tools)

- `cxpak_predict` — POST, params: `files` (required list), `depth`, `focus`. Predicts change impact with structural/historical/call-based signals.
- `cxpak_drift` — POST, params: `save_baseline` (bool), `focus`. Compares architecture snapshot against baseline and historical snapshots.
- `cxpak_security_surface` — POST, params: `focus`. Analyzes unprotected endpoints, secrets, SQL injection, validation gaps, exposure scores.

`RouteEndpoint.handler` now extracts real handler function names per framework (12 frameworks); fallback `"<anonymous>"` for inline closures.

`AutoContextResult.predictions` is populated when the task string mentions specific file paths matching the index.

- `cxpak_visual` — POST, params: `type` (dashboard|architecture|risk|flow|timeline|diff), `format` (html|mermaid|svg|png|c4|json), `focus`, `symbol` (required for flow), `files` (required for diff). Generates visual output inline or writes to `.cxpak/visual/` if >1MB.
- `cxpak_onboard` — POST, params: `focus`. Returns onboarding map with phases, file ordering, and reading time.

### Intelligence API (v2.0.0)

HTTP API at `/v1/` with 12 endpoints, exposed via `cxpak serve`. `/v1/health` is GET; all others are POST. Bearer token auth (timing-safe) required on all `/v1/` routes when `--token` is set. Live endpoints: `health`, `conventions`, `briefing`. Stub endpoints (return `status: not_implemented`): `risks`, `architecture`, `call_graph`, `dead_code`, `predict`, `drift`, `security_surface`, `data_flow`, `cross_lang`. Route names use underscores (e.g., `/v1/security_surface`). Request body limit: 2 MB. Graceful shutdown on SIGTERM/Ctrl-C.

## Key Patterns

### Adding a Language

1. Add `tree-sitter-{lang}` to `Cargo.toml` as optional dep
2. Add feature flag `lang-{name} = ["dep:tree-sitter-{lang}"]` and add to `default`
3. Add extension mapping in `src/scanner/mod.rs` `detect_language()`
4. Create `src/parser/languages/{name}.rs` implementing `LanguageSupport` trait
5. Register in `src/parser/languages/mod.rs` and `src/parser/mod.rs`
6. Add unit tests in the language file

### Pack Mode

When `index.total_tokens > token_budget`, overview writes `.cxpak/` with full detail files.
`SectionContent { budgeted, full, was_truncated }` tracks both versions.
Detail file extensions match `--format` (`.md`, `.json`, `.xml`).

### Trace Command

Finds target via `index.find_symbol()` (case-insensitive), falls back to `find_content_matches()`.
Walks `DependencyGraph` — 1-hop default, full BFS with `--all`.
Non-import edges are rendered with `(via: edge_type)` in the dependency subgraph output.

### Schema Module

`src/schema/` detects and indexes the data layer:

- **`mod.rs`** — `SchemaIndex` (tables, views, functions, orm_models, migrations), `EdgeType` (9 variants: Import, ForeignKey, ViewReference, TriggerTarget, IndexTarget, FunctionReference, EmbeddedSql, OrmModel, MigrationSequence), `TypedEdge`
- **`detect.rs`** — file-pattern heuristics for recognizing SQL schema files, migration directories, ORM model files
- **`extract.rs`** — parses SQL DDL, Prisma schemas, and ORM class definitions to extract table/column/FK metadata
- **`link.rs`** — `build_schema_edges()` converts `SchemaIndex` into typed graph edges; `detect_embedded_sql()` finds inline SQL in application code

`build_dependency_graph()` in `src/index/graph.rs` accepts `Option<&SchemaIndex>` and calls `build_schema_edges()` to inject schema-aware edges alongside the standard Import edges derived from parse results.

### Context Quality Module

`src/context_quality/` contains three submodules:

- **`degradation.rs`** — `DetailLevel` (Full→Trimmed→Documented→Signature→Stub), `FileRole` (Selected/Dependency), `concept_priority()` (7-tier SymbolKind ranking), `render_symbol_at_level()`, `split_oversized_symbol()` (chunks >4000 tokens), `allocate_with_degradation()` (budget-aware progressive detail reduction)
- **`expansion.rs`** — `Domain` enum (8 domains), `detect_domains()` (file-pattern heuristics), `expand_query()` (~30 core synonyms + 8 domain-specific synonym maps)
- **`annotation.rs`** — `comment_syntax()` (per-language comment prefix/suffix), `annotate_file()` (generates `[cxpak]` header with score, role, signals, detail level)

`allocate_with_degradation()` takes `&[(&IndexedFile, FileRole, f64)]` — references, not owned. Selected files never degrade below Documented; dependencies can be dropped.

### Intelligence Module

`src/intelligence/` provides graph-based intelligence features:

- **`pagerank.rs`** — `compute_pagerank()` (iterative PageRank over the dependency graph), `build_symbol_cross_refs()` (cross-file symbol reference map), `symbol_importance()` (file_pagerank × symbol_weight: 1.0 public+referenced, 0.7 public, 0.3 private)
- **`blast_radius.rs`** — `compute_blast_radius()` (BFS from changed files, categorizes into direct_dependents, transitive_dependents, test_files, schema_dependents), `compute_risk()` (hop_decay × edge_weight × pagerank × test_penalty, clamped to [0,1])
- **`api_surface.rs`** — `extract_api_surface()` (public symbols sorted by PageRank, token-budgeted), `detect_routes()` (HTTP route extraction for 12 frameworks: Express, Actix, Axum, Flask, Django, FastAPI, Spring, Gin, Echo, Fiber, Rails, Phoenix), gRPC service and GraphQL type extraction
- **`test_map.rs`** — `build_test_map()` (source→test file mapping via naming conventions for 6 languages: Rust, TypeScript/JavaScript, Python, Java, Go, Ruby, plus catch-all; supplemented by import analysis)

- **`predict.rs`** — `predict()` combines structural (blast radius), historical (co-change), and call-based signals into `PredictionResult` with `TestPrediction` entries ranked by confidence (0.3–0.9 across all 7 signal combinations)
- **`drift.rs`** — `build_drift_report()` compares the current architecture snapshot against a stored baseline (`.cxpak/baseline.json`) and historical snapshots (`.cxpak/snapshots/`); `snapshot_from_index()` auto-saves on each call
- **`security.rs`** — `build_security_surface()` runs 5 deterministic detections: unprotected endpoints (real handler names from api_surface), input validation gaps (high-PageRank files), secret patterns (per-type regex, 5 types), SQL injection (interpolation detection per language), and exposure scores
- **`co_change.rs`** — `mine_co_changes_from_git()` walks git log 180 days back; `build_co_change_edges_with_dates()` applies configurable threshold and recency decay; edges stored on `CodebaseIndex.co_changes`

PageRank scores feed into relevance scoring (signal #6, weight 0.17) and degradation priority (0.6 × pagerank + 0.2 × concept_priority + 0.2 × file_role).

### Auto Context Module

`src/auto_context/` orchestrates the one-call auto_context pipeline:

- **`mod.rs`** — orchestration pipeline: wires the 10 steps (expansion → scoring → seeds → noise filtering → tests → schema → blast radius → API surface → budget allocation → annotations) into a single entry point
- **`noise.rs`** — 3-layer noise filtering: blocklist (generated/vendored files), similarity dedup (near-duplicate content), relevance floor (below minimum score); reports every filtered file and the layer that caught it in `filtered_out`
- **`briefing.rs`** — fill-then-overflow budget allocation: packs seeds first, then tests, schema, blast radius, and API surface by priority until the token budget is exhausted; applies progressive degradation to fit remaining content
- **`diff.rs`** — context snapshots and deltas: captures a snapshot after each `auto_context` call and computes diffs (modified/new/deleted files, symbol changes, graph edge changes) for the `cxpak_context_diff` tool

### Embeddings Module

`src/embeddings/` provides semantic embedding support:

- Local inference via candle with the `all-MiniLM-L6-v2` model (~30 MB, downloaded on first use)
- Remote API providers: OpenAI, Voyage AI, Cohere — configured via `.cxpak.json` with provider, model, api_key_env, base_url, dimensions, batch_size
- Vector index for fast similarity queries
- Embedding similarity is the 7th scoring signal (weight 0.15); graceful fallback to 6 deterministic signals on any failure

## Supported Languages (42)

**Tier 1 — Full extraction** (functions, classes, methods, imports, exports):
Rust, TypeScript, JavaScript, Python, Java, Go, C, C++, Ruby, C#, Swift, Kotlin,
Bash, PHP, Dart, Scala, Lua, Elixir, Zig, Haskell, Groovy, Objective-C, R, Julia, OCaml, MATLAB

**Tier 2 — Structural extraction** (selectors, headings, keys, blocks, etc.):
CSS, SCSS, Markdown, JSON, YAML, TOML, Dockerfile, HCL/Terraform, Protobuf, Svelte, Makefile, HTML, GraphQL, XML

**Database DSLs:**
SQL (via tree-sitter-sequel), Prisma

## Claude Code Plugin

`plugin/` — Claude Code plugin that wraps cxpak as slash commands and MCP tools.

Key files with version references (all must stay in sync):
- `Cargo.toml` — crate version
- `plugin/.claude-plugin/plugin.json` — plugin metadata version
- `.claude-plugin/marketplace.json` — marketplace listing version
- `plugin/lib/ensure-cxpak` — `REQUIRED_VERSION` for auto-install

`plugin/lib/ensure-cxpak` finds cxpak on PATH, or installs via Homebrew/cargo if not found.

`plugin/lib/ensure-cxpak-serve` uses `ensure-cxpak` to resolve the binary, then exec's `cxpak serve --mcp`.

## Release

Tag with `vX.Y.Z` to trigger CI: cross-compile for Linux/macOS + publish to crates.io.

When bumping version, update all four files listed under Claude Code Plugin above, then run `cargo check` to regenerate `Cargo.lock` and commit it BEFORE tagging. The `cargo publish` step fails if `Cargo.lock` has a stale version.
