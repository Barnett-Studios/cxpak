# cxpak v0.9.0 Design: MCP Integration + Task-Aware Context

## Executive Summary

Wire cxpak's existing MCP server (`serve --mcp`) into the Claude Code plugin via `.mcp.json`, making all tools available natively. Add a relevance scoring module and two new MCP tools (`cxpak_context_for_task`, `cxpak_pack_context`) that enable task-aware, token-budgeted context bundling with optional LLM-in-the-loop re-ranking.

**Goal:** Replace Claude Code's "discover-then-act" pattern (5-10 Glob/Grep/Read calls) with pre-computed, dependency-aware context delivered in 1-2 tool calls.

## Architecture

### New Modules

```
src/
├── relevance/              ← NEW
│   ├── mod.rs              # RelevanceScorer trait + MultiSignalScorer
│   ├── signals.rs          # Five signal implementations
│   └── seed.rs             # Seed selection + dependency fan-out
├── commands/
│   └── serve.rs            # +2 MCP tool handlers
├── budget/                 # existing — extended for pack_context
├── index/                  # existing — extended with term frequency data
└── ...

plugin/
├── .mcp.json               ← NEW — stdio MCP server config
├── lib/
│   ├── ensure-cxpak        # existing — unchanged
│   └── ensure-cxpak-serve  ← NEW — resolves binary, execs serve --mcp
├── commands/               # existing — kept as-is
└── skills/                 # existing — kept as-is
```

### Data Flow

```
CodebaseIndex (existing)
    │
    ├── RelevanceScorer scores files against task query
    │     ├── PathSimilarity signal
    │     ├── SymbolMatch signal
    │     ├── ImportProximity signal
    │     ├── TermFrequency signal
    │     └── RecencyBoost signal (optional)
    │
    ├── SeedSelector picks top-N above threshold
    │     └── DependencyGraph fan-out adds 1-hop neighbors
    │
    └── Budget packer fills token budget by relevance rank
```

## Relevance Module

### Trait Definition

```rust
pub trait RelevanceScorer: Send + Sync {
    fn score(&self, query: &str, file_path: &str, index: &CodebaseIndex) -> ScoredFile;
}

pub struct ScoredFile {
    pub path: String,
    pub score: f64,                  // 0.0–1.0 combined
    pub signals: Vec<SignalResult>,  // breakdown
}

pub struct SignalResult {
    pub name: &'static str,
    pub score: f64,
    pub detail: String,
}
```

### Five Signals

| Signal | Weight | Description |
|--------|--------|-------------|
| `PathSimilarity` | 0.20 | Tokenizes query + file path segments, Jaccard similarity |
| `SymbolMatch` | 0.35 | Fuzzy match query terms against function/struct/class names |
| `ImportProximity` | 0.15 | Boost if file imports/is imported by high-scoring files |
| `TermFrequency` | 0.20 | Lightweight TF of query terms in file content |
| `RecencyBoost` | 0.10 | Boost recently-changed files (when git history available) |

**Combined score:** weighted sum, normalized to 0.0–1.0.

### Seed Selection

- Files scoring above **0.3 threshold** become seeds
- Dependency fan-out adds 1-hop neighbors at **0.7× the seed's score**
- Everything sorted by final score descending

### Index Extension

`CodebaseIndex` gains:
```rust
pub term_frequencies: HashMap<String, HashMap<String, u32>>
```
Per-file term counts, built during parsing. Split on word boundaries, count occurrences. Lightweight — no external dependencies.

## MCP Tools

### Existing Tools (unchanged)

| # | Tool | Description |
|---|------|-------------|
| 1 | `cxpak_overview` | Structured codebase overview |
| 2 | `cxpak_trace` | Symbol tracing through dependency graph |
| 3 | `cxpak_diff` | Changes with dependency context |
| 4 | `cxpak_stats` | Index statistics |

### New Tools

#### Tool 5: `cxpak_context_for_task`

Score and rank codebase files by relevance to a natural language task description. Returns candidates for review.

**Input:**
```json
{
  "task": "add rate limiting to API endpoints",
  "limit": 15
}
```

**Output:**
```json
{
  "task": "add rate limiting to API endpoints",
  "candidates": [
    {
      "path": "src/api/mod.rs",
      "score": 0.87,
      "signals": [
        {"name": "symbol_match", "score": 0.95, "detail": "matched: ApiRouter, handle_request"},
        {"name": "path_similarity", "score": 0.80, "detail": "path contains: api"}
      ],
      "tokens": 1250,
      "dependencies": ["src/api/middleware.rs", "src/config.rs"]
    }
  ],
  "total_files_scored": 142,
  "hint": "Review candidates and call cxpak_pack_context with selected paths, or use these as-is."
}
```

#### Tool 6: `cxpak_pack_context`

Pack selected files into a token-budgeted context bundle with dependency context.

**Input:**
```json
{
  "files": ["src/api/mod.rs", "src/api/middleware.rs"],
  "tokens": "30k",
  "include_dependencies": true
}
```

**Output:**
```json
{
  "packed_files": 8,
  "total_tokens": 28400,
  "budget": 30000,
  "files": [
    {
      "path": "src/api/mod.rs",
      "tokens": 1250,
      "content": "// full file content...",
      "included_as": "selected"
    },
    {
      "path": "src/api/middleware.rs",
      "tokens": 800,
      "content": "// full file content...",
      "included_as": "dependency"
    }
  ],
  "omitted": [
    {"path": "src/api/tests.rs", "tokens": 5200, "reason": "budget exceeded"}
  ]
}
```

### Usage Patterns

| Pattern | Flow | When |
|---------|------|------|
| **Cold start** | `context_for_task` → Claude reviews → `pack_context` | No prior context, Claude as re-ranker |
| **Warm** | `pack_context` directly | Claude already has overview, knows which files |
| **Standalone** | `context_for_task` alone | Candidates are enough without full content |

The two-phase handshake (Claude as re-ranker) is **optional**. Each tool is independently useful.

## Plugin Wiring

### `.mcp.json`

```json
{
  "cxpak": {
    "command": "${CLAUDE_PLUGIN_ROOT}/lib/ensure-cxpak-serve",
    "args": [],
    "env": {}
  }
}
```

### `ensure-cxpak-serve` wrapper

```bash
#!/usr/bin/env bash
set -euo pipefail
CXPAK="$(${CLAUDE_PLUGIN_ROOT}/lib/ensure-cxpak)"
exec "$CXPAK" serve --mcp
```

Reuses `ensure-cxpak` for binary resolution (PATH → cached install → auto-download). The `exec` replaces the shell process so stdio flows directly between Claude Code and cxpak.

### Coexistence

All existing commands (`/overview`, `/trace`, `/diff`, `/clean`) and skills (`codebase-context`, `diff-context`) remain. MCP tools are a parallel native path. Users choose whichever they prefer.

**Tool inventory after v0.9.0:**

| MCP Tool | Existing Equivalent | New? |
|----------|-------------------|------|
| `cxpak_overview` | `/overview`, `codebase-context` skill | No |
| `cxpak_trace` | `/trace` | No |
| `cxpak_diff` | `/diff`, `diff-context` skill | No |
| `cxpak_stats` | — | No |
| `cxpak_context_for_task` | — | **Yes** |
| `cxpak_pack_context` | — | **Yes** |

## Testing Strategy

**100% coverage required at every level.**

### Unit Tests — Relevance Module

| Test Area | Cases |
|-----------|-------|
| `PathSimilarity` | exact=1.0, partial=mid, no overlap=0.0, case insensitive, nested paths, special chars |
| `SymbolMatch` | exact hit, fuzzy match, multi-word query, no match=0.0, case insensitive |
| `ImportProximity` | boost for direct imports, no boost for unrelated, bidirectional |
| `TermFrequency` | high frequency scores higher, missing terms=0.0, stopword handling |
| `RecencyBoost` | recently changed=boosted, no git history=neutral (0.5), old files=low |
| `MultiSignalScorer` | weights sum correctly, normalization, signal combination |
| `SeedSelector` | threshold filtering, top-N limiting, fan-out at 0.7×, empty results |
| Score combination | all zero=0.0, all max=1.0, single dominant, weight changes |

### Integration Tests — MCP Tools

| Test Area | Cases |
|-----------|-------|
| `context_for_task` | happy path, empty query error, no matches, limit param |
| `pack_context` | happy path, with deps, without deps, budget overflow, missing files |
| Two-phase flow | `context_for_task` → select → `pack_context` → valid bundle |

### MCP Protocol Tests

| Test Area | Cases |
|-----------|-------|
| `tools/list` | 6 tools returned, schemas correct |
| `tools/call` | JSON-RPC round-trip for both new tools |
| Invalid args | proper error responses |

All tests use existing patterns: `tempfile::TempDir` + `git2` for temp repos, `mcp_stdio_loop_with_io` for protocol tests.

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Binary resolution | `ensure-cxpak-serve` wrapper | Reuses existing auto-download logic, portable |
| Keep slash commands | Yes | Users may prefer them; no maintenance burden |
| Relevance as core module | `src/relevance/` | Reusable across MCP, CLI, HTTP; testable in isolation |
| Two-tool pattern | `context_for_task` + `pack_context` | Claude as optional re-ranker; each tool independently useful |
| Term frequency in index | `HashMap<String, HashMap<String, u32>>` | Lightweight, no external deps, built at parse time |
| Seed threshold | 0.3 | Low enough to catch tangential files, high enough to filter noise |
| Fan-out discount | 0.7× | Dependencies are relevant but less than direct matches |
| Signal weights | sym=0.35, path=0.20, tf=0.20, import=0.15, recency=0.10 | Symbol matching is strongest signal for code tasks |

## Future Enhancements (not in v0.9.0)

- Embedding-based semantic search (swap RelevanceScorer implementation)
- LLM-assisted scoring via MCP sampling capability
- User-trained weights per project
- `cxpak context` CLI command using the same relevance module
- Plugin skill/command migration to prefer MCP when available
