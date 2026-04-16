# cxpak

![Rust](https://img.shields.io/badge/Rust-1.91+-orange.svg)
![CI](https://github.com/Barnett-Studios/cxpak/actions/workflows/ci.yml/badge.svg)
![Crates.io](https://img.shields.io/crates/v/cxpak)
![Downloads](https://img.shields.io/crates/d/cxpak)
![Homebrew](https://img.shields.io/badge/Homebrew-tap-blue.svg)
![License](https://img.shields.io/badge/License-MIT-green.svg)

> Spends CPU cycles so you don't spend tokens. The LLM gets a briefing packet instead of a flashlight in a dark room.

A Rust CLI that indexes codebases using tree-sitter and produces token-budgeted context bundles for LLMs.

## Installation

```bash
# Via Homebrew (macOS/Linux)
brew tap Barnett-Studios/tap
brew install cxpak

# Via cargo
cargo install cxpak
```

## How to Use cxpak

There are four ways to use cxpak, from simplest to most powerful:

### 1. CLI (no setup required)

Run cxpak directly on any git repo:

```bash
# Structured repo summary within a token budget
cxpak overview --tokens 50k .

# Trace a symbol through the dependency graph
cxpak trace --tokens 50k "handle_request" .

# Show changes with dependency context
cxpak diff --tokens 50k .

# More options
cxpak overview --tokens 50k --out context.md .       # Write to file
cxpak overview --tokens 50k --focus src/api .         # Focus on a directory
cxpak overview --tokens 50k --format json .           # JSON or XML output
cxpak overview --tokens 50k --health .                # Append codebase health score
cxpak overview --tokens 50k --workspace packages/api .  # Monorepo workspace scope
cxpak trace --tokens 50k --all "MyError" .            # Full graph traversal
cxpak trace --tokens 50k --workspace packages/api "handle" .  # Trace within workspace
cxpak diff --tokens 50k --git-ref main .              # Diff against a branch
cxpak diff --tokens 50k --since "1 week" .            # Diff by time range
cxpak overview --tokens 50k --timing .                # Show pipeline timing
cxpak clean .                                         # Clear cache

# Convention management
cxpak conventions export .                            # Write .cxpak/conventions.json
cxpak conventions diff .                              # Compare against baseline

# LSP server (for IDE integration)
cxpak lsp .                                           # Run LSP server over stdio
```

### 2. MCP Server (for Claude Code, Cursor, and other AI tools)

Run cxpak as an [MCP](https://modelcontextprotocol.io/) server so your AI tool gets live access to 26 codebase tools — including relevance scoring, query expansion, convention verification, health scoring, call graph analysis, change impact prediction, architecture drift detection, security surface analysis, structural data flow tracing, cross-language symbol resolution, visual intelligence dashboards, onboarding maps, and schema-aware context packing.

**Claude Code** — add to `.mcp.json` in your project root (or `~/.claude/.mcp.json` globally):

```json
{
  "mcpServers": {
    "cxpak": {
      "command": "cxpak",
      "args": ["serve", "--mcp", "."]
    }
  }
}
```

Restart Claude Code after adding the config. The cxpak tools will appear automatically.

**Cursor** — add to `.cursor/mcp.json` in your project:

```json
{
  "mcpServers": {
    "cxpak": {
      "command": "cxpak",
      "args": ["serve", "--mcp", "."]
    }
  }
}
```

**Any MCP client** — run `cxpak serve --mcp .` over stdio. It speaks JSON-RPC 2.0.

Once configured, your AI tool can call these tools:

| Tool | Description |
|------|-------------|
| `cxpak_auto_context` | One-call optimal context for any task |
| `cxpak_overview` | Structured repo summary |
| `cxpak_trace` | Trace a symbol through dependencies |
| `cxpak_stats` | Language stats and token counts |
| `cxpak_diff` | Show changes with dependency context |
| `cxpak_context_for_task` | Score and rank files by relevance to a task |
| `cxpak_pack_context` | Pack selected files into a token-budgeted bundle |
| `cxpak_search` | Regex search with context lines |
| `cxpak_blast_radius` | Analyze change impact with risk scores |
| `cxpak_api_surface` | Extract public API surface |
| `cxpak_context_diff` | Show what changed since last auto_context call |
| `cxpak_verify` | Check code changes against observed conventions |
| `cxpak_conventions` | Full convention profile with evidence and patterns |
| `cxpak_health` | Composite health score across 6 dimensions |
| `cxpak_risks` | Top risky files ranked by churn, blast radius, and test gap |
| `cxpak_briefing` | Compact orientation: manifest, health, risks, architecture — no code |
| `cxpak_call_graph` | Cross-file call graph with confidence levels |
| `cxpak_dead_code` | Dead symbols ranked by importance |
| `cxpak_architecture` | Architecture quality: coupling, cohesion, boundary violations |
| `cxpak_predict` | Predict change impact with structural, historical, and call-based signals |
| `cxpak_drift` | Detect architecture drift against a stored baseline |
| `cxpak_security_surface` | Unprotected endpoints, secrets, SQL injection, validation gaps, exposure |
| `cxpak_data_flow` | Trace how a value flows from source to sink through the call graph |
| `cxpak_cross_lang` | List cross-language boundaries: HTTP, FFI, gRPC, GraphQL, shared schemas, exec |
| `cxpak_visual` | Generate visual intelligence dashboard (HTML, Mermaid, SVG, PNG, C4, JSON) |
| `cxpak_onboard` | Generate guided onboarding reading order for new engineers |

All tools support a `focus` path prefix parameter to scope results.

> **Note:** The MCP server, embeddings, and all features are included by default. No extra feature flags needed.

### 3. Claude Code Plugin (auto-triggers + slash commands)

The plugin wraps cxpak as skills and slash commands. Skills auto-trigger when Claude detects relevant questions; slash commands give you direct control.

**Install:**

```
/plugin marketplace add Barnett-Studios/cxpak
/plugin install cxpak
```

The plugin installs cxpak automatically via Homebrew (or cargo) if not already on PATH.

**Skills (auto-invoked):**

| Skill | Triggers when you... |
|-------|---------------------|
| `codebase-context` | Ask about project structure, architecture, how components relate |
| `diff-context` | Ask to review changes, prepare a PR description, understand what changed |

**Commands (user-invoked):**

| Command | Description |
|---------|-------------|
| `/cxpak:overview` | Generate a structured repo summary |
| `/cxpak:trace <symbol>` | Trace a symbol through the dependency graph |
| `/cxpak:diff` | Show changes with dependency context |
| `/cxpak:clean` | Remove `.cxpak/` cache and output files |

### 4. HTTP Server (for custom integrations)

Run cxpak as a persistent HTTP server with a hot index:

```bash
# Start HTTP server (default port 3000)
cxpak serve .
cxpak serve --port 8080 .
cxpak serve --bind 0.0.0.0 --port 8080 .             # Bind to all interfaces

# With authentication on /v1/ endpoints
cxpak serve --token my-secret .

# Watch for file changes and keep index hot
cxpak watch .
```

**Legacy endpoints** (no auth required):

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Health check |
| `GET /stats` | Language stats and token counts |
| `GET /overview?tokens=50000` | Structured repo summary |
| `GET /trace?target=handle_request` | Trace a symbol through dependencies |
| `GET /diff?git_ref=HEAD~1` | Show changes with dependency context |
| `POST /search` | Regex search with context |
| `POST /blast_radius` | Change impact analysis |
| `GET /api_surface` | Public API extraction |
| `POST /auto_context` | One-call optimal context |
| `GET /context_diff` | Session delta |
| `GET /health_score` | Codebase health score |
| `GET /risks` | Top risky files |
| `POST /call_graph` | Cross-file call graph |
| `POST /dead_code` | Dead symbol detection |
| `POST /architecture` | Architecture quality report |
| `POST /predict` | Change impact prediction with test predictions |
| `POST /drift` | Architecture drift against a stored baseline |
| `POST /security_surface` | Security surface analysis |
| `POST /data_flow` | Structural data flow tracing |
| `GET /cross_lang` | Cross-language boundary list |

**Intelligence API v1** (requires `--token` when set):

All `/v1/` endpoints are POST and accept JSON bodies. Pass the token as `Authorization: Bearer <token>`.

| Endpoint | Description |
|----------|-------------|
| `GET /v1/health` | Index stats (total_files, total_tokens) |
| `POST /v1/conventions` | Full convention profile |
| `POST /v1/briefing` | Compact orientation for a task (requires `task` field) |
| `POST /v1/risks` | Top risky files (stub) |
| `POST /v1/architecture` | Architecture quality (stub) |
| `POST /v1/call_graph` | Cross-file call graph (stub) |
| `POST /v1/dead_code` | Dead symbol detection (stub) |
| `POST /v1/predict` | Change impact prediction (stub) |
| `POST /v1/drift` | Architecture drift (stub) |
| `POST /v1/security_surface` | Security surface analysis (stub) |
| `POST /v1/data_flow` | Structural data flow tracing (stub) |
| `POST /v1/cross_lang` | Cross-language boundaries (stub) |

Stub endpoints return placeholder JSON; full analysis is available via the corresponding legacy endpoints.

### 5. LSP Server (for IDE integration)

Run cxpak as an LSP server over stdio for editor/IDE integration:

```bash
cxpak lsp .
cxpak lsp /path/to/repo
```

**Standard LSP methods:**

| Method | Description |
|--------|-------------|
| `textDocument/codeLens` | Code lenses showing symbol token counts |
| `textDocument/hover` | Hover info with symbol details from the index |
| `textDocument/diagnostic` | Diagnostics for convention violations |
| `workspace/symbol` | Workspace symbol search across the index |

**Custom JSON-RPC methods (14):**

| Method | Description |
|--------|-------------|
| `cxpak/health` | Index health stats |
| `cxpak/conventions` | Convention profile |
| `cxpak/blastRadius` | Change impact analysis |
| `cxpak/overview` | Codebase overview (stub) |
| `cxpak/trace` | Symbol tracing (stub) |
| `cxpak/diff` | Change context (stub) |
| `cxpak/search` | Code search (stub) |
| `cxpak/apiSurface` | API surface (stub) |
| `cxpak/deadCode` | Dead code detection (stub) |
| `cxpak/callGraph` | Call graph (stub) |
| `cxpak/predict` | Change prediction (stub) |
| `cxpak/drift` | Architecture drift (stub) |
| `cxpak/securitySurface` | Security surface (stub) |
| `cxpak/dataFlow` | Data flow analysis (stub) |

The LSP server builds the codebase index at startup and supports `textDocument/didOpen`, `didChange`, and `didClose` for in-editor reactivity. A background file watcher keeps the index hot for on-disk changes.

## What You Get

The `overview` command produces a structured briefing with these sections:

- **Project Metadata** — file counts, languages, estimated tokens
- **Directory Tree** — full file listing
- **Module / Component Map** — files with their public symbols
- **Dependency Graph** — import relationships between files
- **Key Files** — full content of README, config files, manifests
- **Function / Type Signatures** — every public symbol's signature
- **Git Context** — recent commits, file churn, contributors

Each section has a budget allocation. When content exceeds its budget, it's truncated with the most important items preserved first.

## Context Quality

cxpak applies intelligent context management to maximize the usefulness of every token:

**Progressive Degradation** — When content exceeds the budget, symbols are progressively reduced through 5 detail levels (Full → Trimmed → Documented → Signature → Stub). High-relevance files keep full detail while low-relevance dependencies are summarized. Selected files never degrade below Documented; dependencies can be dropped entirely as a last resort.

**Concept Priority** — Symbols are ranked by type: functions/methods (1.0) > structs/classes (0.86) > API surface (0.71) > configuration (0.57) > documentation (0.43) > constants (0.29). This determines degradation order — functions survive longest.

**Query Expansion** — When using `context_for_task`, queries are expanded with ~30 core synonym mappings (e.g., "auth" → authentication, login, jwt, oauth) plus 8 domain-specific maps (Web, Database, Auth, Infra, Testing, API, Mobile, ML) activated automatically by detecting file patterns in the repo.

**Context Annotations** — Each packed file gets a language-aware comment header showing its relevance score, role (selected/dependency), signal breakdown, and detail level. The LLM knows exactly why each file was included and how much detail it's seeing.

**Chunk Splitting** — Symbols exceeding 4000 tokens are split into labeled chunks (e.g., `handler [1/3]`) that degrade independently. Each chunk carries the parent signature for context.

## Conventions

cxpak extracts a quantified convention profile from the codebase — the patterns your team actually follows, not what a linter wishes you followed.

**8 convention categories:** naming, imports, errors, dependencies, testing, visibility, functions, git health. Each pattern includes counts, percentages, strength labels (Convention ≥90%, Trend ≥70%, Mixed), and exceptions.

**Convention verification** — `cxpak_verify` checks code changes against observed conventions. It only flags violations in changed lines, not pre-existing debt. Reports include severity, evidence, and suggested fixes.

**Convention export/diff** — `cxpak conventions export .` writes the full convention profile to `.cxpak/conventions.json` with a SHA256 checksum. `cxpak conventions diff .` compares the current profile against the stored baseline and reports which categories changed. Use this in CI to catch convention drift across PRs.

**Repository DNA** — Every `auto_context` call includes a ~1000 token DNA section summarizing naming conventions, error handling patterns, import style, architecture layering, visibility defaults, function length stats, testing patterns, and git health. This gives the LLM implicit knowledge of "how we do things here" before it sees any code.

## Intelligence

cxpak includes graph-based intelligence features that go beyond static analysis.

**PageRank File Importance** — Every file in the dependency graph is scored 0.0–1.0 using PageRank over the import graph. Files that are transitively imported by many others rank higher. PageRank is used as signal #6 in relevance scoring (weight 0.17) and drives degradation priority via the formula `0.6 × pagerank + 0.2 × concept_priority + 0.2 × file_role`. Symbol-level importance is computed as `file_pagerank × symbol_weight`, where symbol_weight is 1.0 (public + referenced), 0.7 (public), or 0.3 (private).

**Blast Radius Analysis** — The `cxpak_blast_radius` tool takes a set of changed files and returns categorized affected files: `direct_dependents`, `transitive_dependents`, `test_files`, and `schema_dependents`, each with a risk score. Risk is calculated as `hop_decay × edge_weight × pagerank × test_penalty`, clamped to [0, 1]. This tells you which parts of the codebase are most likely to break when you change a file.

**API Surface Extraction** — The `cxpak_api_surface` tool extracts the public API of a codebase: public symbols sorted by PageRank, HTTP routes (12 frameworks including Express, Actix, Axum, Flask, Django, FastAPI, Spring, Gin, Echo, Fiber, Rails, and Phoenix), gRPC services, and GraphQL types. Output is token-budgeted.

**Test File Mapping** — cxpak automatically maps source files to their test files using naming conventions for 6 languages (Rust, TypeScript/JavaScript, Python, Java, Go, Ruby) plus a catch-all pattern, supplemented by import analysis. The `pack_context` tool auto-includes test files when the `include_tests` parameter is set. Blast radius uses the test map to populate the `test_files` category.

**Call Graph** — `cxpak_call_graph` builds a cross-file call graph from parse results. Each edge carries a confidence level: Exact (import-resolved) or Approximate (name-matched). Use it to understand how functions flow across module boundaries.

**Dead Code Detection** — `cxpak_dead_code` identifies symbols with zero callers that are not entry points and not referenced from tests. Results are sorted by a liveness score so the most important dead symbols surface first.

**Health Score** — `cxpak_health` returns a composite metric across 6 dimensions: convention adherence, test coverage, churn stability, module coupling, circular dependencies, and dead code. Use it to understand overall codebase quality before making structural changes.

**Risk Ranking** — `cxpak_risks` ranks files by a composite of churn rate, blast radius, and test coverage gap. These are the files most likely to cause problems and most in need of refactoring or additional tests.

**Architecture Quality** — `cxpak_architecture` reports per-module metrics: coupling (inter-module dependencies), cohesion (intra-module relatedness), circular dependency count, boundary violations (cross-layer imports), and god files (files with excessive responsibility).

**Change Prediction** — `cxpak_predict` takes a list of changed files and returns a ranked impact prediction using three signals: structural (blast radius), historical (git co-change within the last 180 days), and call-based (call graph proximity). Each prediction carries a confidence score between 0.3 and 0.9 computed from which signals fire. Test predictions tell you exactly which tests to run to validate a change, ranked by confidence.

**Architecture Drift** — `cxpak_drift` compares the current architecture snapshot against a stored baseline in `.cxpak/baseline.json` and against historical snapshots in `.cxpak/snapshots/`. It reports new module boundaries, changed coupling and cohesion metrics, new circular dependencies, and new boundary violations. Set `save_baseline=true` to establish the current state as the new baseline. Use it in CI or release reviews to track architectural health over time. Snapshots are auto-saved on every call so you always have a record of where things were.

**Security Surface** — `cxpak_security_surface` runs five deterministic detections: unprotected endpoints (HTTP handlers without auth guards across 12 frameworks, with real handler names extracted per framework), input validation gaps (public entry points that skip validation), secret patterns (AWS access keys, GitHub PATs, connection strings, Slack tokens, and hardcoded passwords), SQL injection risks (string-interpolated queries across 6 languages), and exposure scores (files ranked by public surface area and inbound dependency count). Authentication pattern matching is configurable.

**Data Flow Analysis** — `cxpak_data_flow` traces how a named value flows through the system from a source symbol toward sinks by walking the call graph. Each node is classified as Source, Transform, Sink, or Passthrough by name heuristic, and each path is tagged with a confidence level: `Exact` (direct static calls), `Approximate` (name-resolved), or `Speculative` (closures, higher-order functions, trait objects, virtual methods, or dynamic dispatch). Paths report whether they cross a module boundary, a language boundary, or a security-sensitive file (via integration with the security surface). Max depth is 10 hops with cycle detection. Every response includes a `limitations` array so the LLM always sees what the trace cannot prove — no hidden assumptions.

**Cross-Language Symbol Resolution** — `cxpak_cross_lang` detects six types of cross-language boundaries: HTTP calls (`fetch` / `axios` / `reqwest` matched to detected routes), FFI bindings (Rust `extern "C"` or Python `ctypes` matched to C/C++ symbol definitions), gRPC calls (client method invocations matched to `.proto` service methods), GraphQL queries (named operations matched to typed schema files), shared database schemas (two files in different languages touching the same table), and command exec bridges (`subprocess.run` / `exec.Command` / `std::process::Command::new` matched to indexed binaries or scripts). Detected edges are injected into the dependency graph as `EdgeType::CrossLanguage(BridgeType)` so blast radius, PageRank, and auto_context all pick them up. A language whitelist and test-path exclusion prevent documentation code examples and test fixtures from producing false positives.

## Auto Context

`cxpak_auto_context` is the main entry point — one call that delivers optimal context for any task. Give it a task description and token budget; it returns everything the LLM needs.

**Pipeline:**

0. **DNA section** — renders a ~1000 token convention summary (naming, errors, imports, architecture, visibility, functions, testing, git health), deducted from the budget before allocation
1. **Query expansion** — expands the task description with synonyms and domain-specific terms
2. **Relevance scoring** — scores every file against the expanded query using 7 weighted signals
3. **Seed selection** — picks the top-scoring files as seeds for graph traversal
4. **Noise filtering** — 3 layers remove low-value files: blocklist (generated/vendored), similarity dedup (near-duplicate content), and relevance floor (below minimum score). Files removed by each layer are reported in `filtered_out` for transparency
5. **Test inclusion** — maps seed files to their test files via naming conventions and import analysis
6. **Schema linking** — pulls in schema files connected to seeds via typed dependency edges
7. **Blast radius** — identifies files at risk from the seed set, sorted by risk score
8. **API surface** — extracts public symbols and HTTP routes from seed files
9. **Cross-language edges** — filters `cross_lang_edges` by focus and emits them as a dedicated section (capped at 500 tokens, structured JSON kept intact)
10. **Budget allocation** — fill-then-overflow priority packing: seeds first, then tests, schema, blast radius, API surface, and cross-language edges until the budget is exhausted
11. **Annotations** — each packed file gets a language-aware comment header with score, role, signals, and detail level

**Noise filtering** applies three independent layers. The `filtered_out` field in the response lists every file removed and which layer caught it, so you can audit what was excluded and why.

**Token-budgeted output** uses fill-then-overflow priority packing: high-priority categories (seeds, tests) fill first; lower-priority categories (blast radius, API surface) overflow into remaining budget. Content that doesn't fit is progressively degraded through 5 detail levels before being dropped.

## Workspace Support

For monorepos, the `--workspace` flag scopes scanning to a subdirectory while keeping the full repo as the git root:

```bash
cxpak overview --tokens 50k --workspace packages/api .
cxpak trace --tokens 50k --workspace packages/api "handle_request" .
```

Only files under the workspace prefix are indexed. MCP tools that accept a `workspace` parameter (`cxpak_call_graph`, `cxpak_dead_code`, `cxpak_architecture`) also support workspace scoping.

## Data Layer Awareness

cxpak understands the data layer of your codebase and uses that knowledge to build richer dependency graphs.

**Schema Detection** — SQL (`CREATE TABLE`, `CREATE VIEW`, stored procedures), Prisma schema files, and other database DSLs are parsed to extract table definitions, column names, foreign key references, and view dependencies.

**ORM Detection** — Django models, SQLAlchemy mapped classes, TypeORM entities, and ActiveRecord models are recognized and linked to their underlying table definitions.

**Typed Dependency Graph** — Every edge in the dependency graph carries one of 10 semantic types:

| Edge Type | Meaning |
|-----------|---------|
| `import` | Standard language import / require |
| `foreign_key` | Table FK reference to another table file |
| `view_reference` | SQL view references a source table |
| `trigger_target` | Trigger defined on a table |
| `index_target` | Index defined on a table |
| `function_reference` | Stored function references a table |
| `embedded_sql` | Application code contains inline SQL referencing a table |
| `orm_model` | ORM model class maps to a table file |
| `migration_sequence` | Migration file depends on its predecessor |
| `cross_language` | Cross-language boundary (HTTP, FFI, gRPC, GraphQL, shared schema, or command exec) — carries a `BridgeType` payload |

Non-import edges are surfaced in the dependency graph output and in pack context annotations:

```
// score: 0.82 | role: dependency | parent: src/api/orders.py (via: embedded_sql)
```

**Migration Support** — Migration sequences are detected for Rails, Alembic, Flyway, Django, Knex, Prisma, and Drizzle. Each migration is linked to its predecessor so cxpak can trace the full migration chain.

**Embedded SQL Linking** — When application code (Python, TypeScript, Rust, etc.) contains inline SQL strings that reference known tables, cxpak creates `embedded_sql` edges connecting those files to the table definition files. This means `context_for_task` and `pack_context` will automatically pull in relevant schema files when you ask about database-related tasks.

**Schema-Aware Query Expansion** — When the Database domain is detected, table names and column names from the schema index are added as expansion terms. Queries for "orders" or "user_id" will match files that reference those identifiers even if the query term doesn't appear literally in the file path or symbol names.

## Embeddings

cxpak supports semantic embeddings as the 7th scoring signal (`embedding_similarity`, weight 0.15), improving relevance scoring for queries that don't share exact keywords with file content.

**Local (zero config)** — On first use, cxpak downloads the `all-MiniLM-L6-v2` model (~30 MB) and runs inference locally via candle. No API keys needed.

**BYOK (Bring Your Own Key)** — For higher-quality embeddings, configure a remote provider in `.cxpak.json`:

```json
{
  "embeddings": {
    "provider": "openai",
    "model": "text-embedding-3-small",
    "api_key_env": "OPENAI_API_KEY",
    "base_url": "https://api.openai.com/v1",
    "dimensions": 1536,
    "batch_size": 100
  }
}
```

Supported providers: `openai`, `voyageai`, `cohere`. Set `api_key_env` to the environment variable holding your API key.

**Graceful fallback** — If embedding computation fails for any reason (model download error, API timeout, missing key), cxpak falls back to the 6 deterministic scoring signals with zero impact on the rest of the pipeline.

## Context Diff

`cxpak_context_diff` shows what changed in the codebase since the last `cxpak_auto_context` call, enabling efficient session-length workflows.

**Tracked changes:**

- **Modified files** — files with content changes since the snapshot
- **New files** — files added since the snapshot
- **Deleted files** — files removed since the snapshot
- **Symbol changes** — functions, types, and other symbols added, removed, or modified
- **Graph edge changes** — new or removed dependency relationships

The output includes a human-readable recommendation summarizing what changed and whether a fresh `auto_context` call is warranted.

## Visual Intelligence

cxpak generates interactive visual dashboards and static diagrams from the codebase index — no browser-side build step, no CDN dependency. Every output is self-contained.

```bash
# Interactive HTML dashboard (default)
cxpak visual --visual-type dashboard .

# Architecture explorer with 3-level semantic zoom (module → file → symbol)
cxpak visual --visual-type architecture .

# Risk heatmap — treemap sized by blast radius, colored by risk score
cxpak visual --visual-type risk .

# Data flow diagram for a symbol
cxpak visual --visual-type flow --symbol handle_request .

# Git history time machine
cxpak visual --visual-type timeline .

# Change impact diff view
cxpak visual --visual-type diff --files "src/api.rs,src/db.rs" .
```

**6 view types:** Dashboard (4-quadrant overview with health, risks, architecture, alerts), Architecture Explorer (3-level zoom: modules → files → symbols), Risk Heatmap (D3 treemap), Flow Diagram (left-to-right with cross-language dividers), Time Machine (git history snapshots with key event detection), Diff View (before/after with blast radius overlay).

**6 export formats:** HTML (self-contained with inlined D3.js), Mermaid, SVG, PNG (via resvg rasterization), C4 DSL (Structurizr), JSON.

**Layout engine:** Sugiyama method with SCC condensation, barycenter crossing minimization, and Brandes-Kopf coordinate assignment. Cognitive load capped at 7±2 nodes per layer via automatic clustering.

## Onboarding Map

Generate a dependency-ordered reading guide for new engineers:

```bash
cxpak onboard .
cxpak onboard --format json .
```

Files are topologically sorted so dependencies appear before dependents, then grouped into phases by module (one module per phase, max 9 files per phase). Phases are ordered by aggregate PageRank — most important module first. Each file lists up to 5 key symbols to focus on and an estimated reading time at 200 tokens/minute.

## WASM Plugin SDK

Extend cxpak with custom analyzers and detectors via WASM plugins:

- **Plugin manifest** — `.cxpak/plugins.json` declares plugins with file pattern scoping, content access control, and SHA-256 checksum verification
- **Analyzer plugins** — receive an `IndexSnapshot` (filtered by declared patterns), return `Vec<Finding>` with severity levels and metadata
- **Detector plugins** — receive individual `FileSnapshot` per matching file, return `Vec<Detection>` with line-level precision
- **Security** — SHA-256 checksum verification before WASM compilation, 10 MB plugin size limit, 1 MB return payload cap, wasmtime epoch interruption (CPU time limit), memory growth capped at 64 MB via `ResourceLimiter`, capability enforcement (Analyzer-only plugins cannot call Detector methods), content access warnings displayed on first load

The plugin loader uses wasmtime for sandboxed execution. Plugin types (`PluginCapability`, `Finding`, `Detection`, `IndexSnapshot`, `FileSnapshot`) are always compiled; the wasmtime runtime is behind the `plugins` feature flag.

## Stable API

v2.0.0 establishes semver for the MCP API. Tool names, required parameters, and response structures are stable across 2.x.

## Pack Mode

When a repo exceeds the token budget, cxpak automatically switches to **pack mode**:

- The overview stays within budget (one file, fits in one LLM prompt)
- A `.cxpak/` directory is created with **full untruncated** detail files
- Truncated sections in the overview get pointers to their detail files

```
repo/
  .cxpak/
    tree.md          # complete directory tree
    modules.md       # every file, every symbol
    dependencies.md  # full import graph
    signatures.md    # every public signature
    key-files.md     # full key file contents
    git.md           # full git history
```

Detail file extensions match `--format`: `.md` for markdown, `.json` for json, `.xml` for xml.

The overview tells the LLM what exists. The detail files let it drill in on demand. `.cxpak/` is automatically added to `.gitignore`.

If the repo fits within budget, you get a single file with everything — no `.cxpak/` directory needed.

## Caching

cxpak caches parse results in `.cxpak/cache/` to speed up re-runs. The cache is keyed on file modification time and size — when a file changes, it's automatically re-parsed.

To clear the cache and all output files:

```bash
cxpak clean .
```

## Supported Languages (42)

**Tier 1 — Full extraction** (functions, classes, methods, imports, exports):
Rust, TypeScript, JavaScript, Python, Java, Go, C, C++, Ruby, C#, Swift, Kotlin,
Bash, PHP, Dart, Scala, Lua, Elixir, Zig, Haskell, Groovy, Objective-C, R, Julia, OCaml, MATLAB

**Tier 2 — Structural extraction** (selectors, headings, keys, blocks, targets, etc.):
CSS, SCSS, Markdown, JSON, YAML, TOML, Dockerfile, HCL/Terraform, Protobuf, Svelte, Makefile, HTML, GraphQL, XML

**Database DSLs:**
SQL, Prisma

Tree-sitter grammars are compiled in. All 42 languages are enabled by default. Language features can be toggled:

```bash
# Only Rust and Python support
cargo install cxpak --no-default-features --features lang-rust,lang-python
```

## License

MIT

---

## About

Built and maintained by Barnett Studios — building products, teams, and systems that last. Part-time technical leadership for startups and scale-ups.
