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

## Claude Code Plugin

cxpak ships as a Claude Code plugin — skills auto-trigger when you ask about codebase structure or changes, and slash commands give you direct control.

**Install the plugin:**

```
/plugin marketplace add Barnett-Studios/cxpak
/plugin install cxpak
```

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

The plugin auto-downloads the cxpak binary if it's not already installed.

## Usage

```bash
# Structured repo summary within a token budget
cxpak overview --tokens 50k .

# Write output to a file
cxpak overview --tokens 50k --out context.md .

# Focus on a specific directory (boosts ranking)
cxpak overview --tokens 50k --focus src/api .

# Trace from a function/error, pack relevant code paths
cxpak trace --tokens 50k "handle_request" .

# Trace with full dependency graph traversal
cxpak trace --tokens 50k --all "MyError" /path/to/repo

# Different output formats
cxpak overview --tokens 50k --format json .
cxpak overview --tokens 50k --format xml .

# Show changes with dependency context (vs working tree)
cxpak diff --tokens 50k .

# Diff against a specific ref
cxpak diff --tokens 50k --git-ref main .

# Diff by time range
cxpak diff --tokens 50k --since "1 week" .

# Full dependency graph context
cxpak diff --tokens 50k --all .

# Print pipeline timing info
cxpak overview --tokens 50k --timing .

# Clean cache and output files
cxpak clean .
```

## Daemon Mode

With the `daemon` feature flag, cxpak can run as a persistent server with a hot index that updates on file changes.

```bash
# Install with daemon support
cargo install cxpak --features daemon

# Watch for file changes and keep index hot
cxpak watch .

# Start HTTP server (default port 3000)
cxpak serve .
cxpak serve --port 8080 .

# Start as MCP server over stdio
cxpak serve --mcp .
```

### HTTP API

When running `cxpak serve`, these endpoints are available:

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Health check |
| `GET /stats` | Language stats and token counts |
| `GET /overview?tokens=50000` | Structured repo summary |
| `GET /trace?target=handle_request` | Trace a symbol through dependencies |
| `GET /diff?git_ref=HEAD~1` | Show changes with dependency context |

### MCP Server

When running `cxpak serve --mcp`, cxpak speaks [Model Context Protocol](https://modelcontextprotocol.io/) over stdin/stdout. It exposes seven tools (all support a `focus` path prefix parameter):

| Tool | Description |
|------|-------------|
| `cxpak_overview` | Structured repo summary |
| `cxpak_trace` | Trace a symbol through dependencies |
| `cxpak_stats` | Language stats and token counts |
| `cxpak_diff` | Show changes with dependency context |
| `cxpak_context_for_task` | Score and rank files by relevance to a task |
| `cxpak_pack_context` | Pack selected files into a token-budgeted bundle |
| `cxpak_search` | Regex search with context lines |

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
