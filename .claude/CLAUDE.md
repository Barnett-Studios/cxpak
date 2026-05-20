# cxpak — developer context for Claude Code sessions

cxpak is a Rust codebase intelligence daemon. It indexes a code corpus using
tree-sitter, builds a typed dependency graph, and exposes token-budgeted context
bundles, call graphs, blast radius, and dead code detection over HTTP and MCP.

## Repository layout

```
src/
  api/          HTTP endpoint handlers (axum)
  intelligence/ Architecture scoring, API surface, data-flow, security analysis
  lsp/          LSP server and 14 custom methods
  mcp/          MCP tool definitions (26 tools)
  parser/       Tree-sitter language parsers — one file per language
    languages/  Per-language parser modules (clojure.rs, typescript.rs, …)
  search/       Symbol index and PageRank ranking
plugin/         Claude Code plugin (skills, commands, .claude-plugin manifest)
.claude-plugin/ Marketplace manifest
```

## Supported Languages (43)

**Full extraction** (functions, classes, methods, imports, exports, visibility):
Rust, TypeScript, JavaScript, Python, Java, Go, C, C++, Ruby, C#, Swift,
Kotlin, Bash, PHP, Dart, Scala, Lua, Elixir, Zig, Haskell, Groovy,
Objective-C, R, Julia, OCaml, MATLAB, Clojure

**Structural extraction** (selectors, keys, blocks):
CSS, SCSS, Markdown, JSON, YAML, TOML, Dockerfile, HCL/Terraform, Protobuf,
Svelte, Makefile, HTML, GraphQL, XML

**Database DSLs:** SQL, Prisma

## Adding a new language

1. Add a `src/parser/languages/<lang>.rs` implementing `LanguageParser`.
2. Register the feature flag in `Cargo.toml` under `[features]`.
3. Wire it into `src/parser/mod.rs` `build_registry()`.
4. Map file extensions in `src/parser/languages/<lang>.rs` `extensions()`.
5. Add the language to `src/intelligence/api_surface.rs` `is_source_code_file()`.
6. Bump the language count in README.md (line 11 and `## Language support`),
   plugin.json, marketplace.json, and this file.
7. Add unit tests covering: symbol extraction, visibility, imports, line numbers.

## Key invariants

- Zero `unsafe` in production parser code.
- All `unwrap()` calls in parser code are guarded or replaced with `unwrap_or`.
- The registry floor in `src/parser/mod.rs` must equal the total language count.
- Identifier sanitization (bidi, control chars) is handled at the presentation
  layer (`search_index.rs`, `architecture.rs`, `lsp/methods.rs`), not in parsers.

## Running checks

```bash
cargo test --features lang-clojure   # Clojure-specific tests
cargo test                           # Full suite
cargo fmt -- --check                 # Formatter
cargo clippy                         # Linter
```
