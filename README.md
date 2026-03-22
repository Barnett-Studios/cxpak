# cxpak

![Rust](https://img.shields.io/badge/Rust-1.80+-orange.svg)
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
cxpak trace --tokens 50k --all "MyError" .            # Full graph traversal
cxpak diff --tokens 50k --git-ref main .              # Diff against a branch
cxpak diff --tokens 50k --since "1 week" .            # Diff by time range
cxpak overview --tokens 50k --timing .                # Show pipeline timing
cxpak clean .                                         # Clear cache
```

### 2. MCP Server (for Claude Code, Cursor, and other AI tools)

Run cxpak as an [MCP](https://modelcontextprotocol.io/) server so your AI tool gets live access to 9 codebase tools — including relevance scoring, query expansion, and schema-aware context packing.

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
| `cxpak_overview` | Structured repo summary |
| `cxpak_trace` | Trace a symbol through dependencies |
| `cxpak_stats` | Language stats and token counts |
| `cxpak_diff` | Show changes with dependency context |
| `cxpak_context_for_task` | Score and rank files by relevance to a task |
| `cxpak_pack_context` | Pack selected files into a token-budgeted bundle |
| `cxpak_search` | Regex search with context lines |
| `cxpak_blast_radius` | Analyze change impact with risk scores |
| `cxpak_api_surface` | Extract public API surface |

All tools support a `focus` path prefix parameter to scope results.

> **Note:** The MCP server requires the `daemon` feature. Install with `cargo install cxpak --features daemon` or use Homebrew (includes daemon support by default).

### 3. Claude Code Plugin (auto-triggers + slash commands)

The plugin wraps cxpak as skills and slash commands. Skills auto-trigger when Claude detects relevant questions; slash commands give you direct control.

**Install:**

```
/plugin marketplace add Barnett-Studios/cxpak
/plugin install cxpak
```

The plugin auto-downloads the cxpak binary if it's not already installed.

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
# Install with daemon support
cargo install cxpak --features daemon

# Start HTTP server (default port 3000)
cxpak serve .
cxpak serve --port 8080 .

# Watch for file changes and keep index hot
cxpak watch .
```

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Health check |
| `GET /stats` | Language stats and token counts |
| `GET /overview?tokens=50000` | Structured repo summary |
| `GET /trace?target=handle_request` | Trace a symbol through dependencies |
| `GET /diff?git_ref=HEAD~1` | Show changes with dependency context |

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

## Data Layer Awareness

cxpak understands the data layer of your codebase and uses that knowledge to build richer dependency graphs.

**Schema Detection** — SQL (`CREATE TABLE`, `CREATE VIEW`, stored procedures), Prisma schema files, and other database DSLs are parsed to extract table definitions, column names, foreign key references, and view dependencies.

**ORM Detection** — Django models, SQLAlchemy mapped classes, TypeORM entities, and ActiveRecord models are recognized and linked to their underlying table definitions.

**Typed Dependency Graph** — Every edge in the dependency graph carries one of 9 semantic types:

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

Non-import edges are surfaced in the dependency graph output and in pack context annotations:

```
// score: 0.82 | role: dependency | parent: src/api/orders.py (via: embedded_sql)
```

**Migration Support** — Migration sequences are detected for Rails, Alembic, Flyway, Django, Knex, Prisma, and Drizzle. Each migration is linked to its predecessor so cxpak can trace the full migration chain.

**Embedded SQL Linking** — When application code (Python, TypeScript, Rust, etc.) contains inline SQL strings that reference known tables, cxpak creates `embedded_sql` edges connecting those files to the table definition files. This means `context_for_task` and `pack_context` will automatically pull in relevant schema files when you ask about database-related tasks.

**Schema-Aware Query Expansion** — When the Database domain is detected, table names and column names from the schema index are added as expansion terms. Queries for "orders" or "user_id" will match files that reference those identifiers even if the query term doesn't appear literally in the file path or symbol names.

## Intelligence

cxpak v0.13.0 adds graph-based intelligence features that go beyond static analysis.

**PageRank File Importance** — Every file in the dependency graph is scored 0.0–1.0 using PageRank over the import graph. Files that are transitively imported by many others rank higher. PageRank is used as signal #6 in relevance scoring (weight 0.17) and drives degradation priority via the formula `0.6 × pagerank + 0.2 × concept_priority + 0.2 × file_role`. Symbol-level importance is computed as `file_pagerank × symbol_weight`, where symbol_weight is 1.0 (public + referenced), 0.7 (public), or 0.3 (private).

**Blast Radius Analysis** — The `cxpak_blast_radius` MCP tool takes a set of changed files and returns categorized affected files: `direct_dependents`, `transitive_dependents`, `test_files`, and `schema_dependents`, each with a risk score. Risk is calculated as `hop_decay × edge_weight × pagerank × test_penalty`, clamped to [0, 1]. This tells you which parts of the codebase are most likely to break when you change a file.

**API Surface Extraction** — The `cxpak_api_surface` MCP tool extracts the public API of a codebase: public symbols sorted by PageRank, HTTP routes (12 frameworks including Express, Actix, Axum, Flask, Django, FastAPI, Spring, Gin, Echo, Fiber, Rails, and Phoenix), gRPC services, and GraphQL types. Output is token-budgeted.

**Test File Mapping** — cxpak automatically maps source files to their test files using naming conventions for 6 languages (Rust, TypeScript/JavaScript, Python, Java, Go, Ruby) plus a catch-all pattern, supplemented by import analysis. The `pack_context` tool auto-includes test files when the `include_tests` parameter is set. Blast radius uses the test map to populate the `test_files` category.

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
