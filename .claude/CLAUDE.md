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

Pipeline: **Scanner → Parser → Index → Budget → Output**

1. **Scanner** (`src/scanner/`) — walks git-tracked files, detects language from extension
2. **Parser** (`src/parser/`) — tree-sitter extraction of symbols, imports, exports per language
3. **Index** (`src/index/`) — builds `CodebaseIndex` with token counts, language stats, dependency graph
4. **Budget** (`src/budget/`) — allocates token budget across sections, truncates with omission markers
5. **Output** (`src/output/`) — renders to markdown, JSON, or XML

## Commands

- `overview` — structured repo summary within a token budget
- `trace` — finds a target symbol, walks dependency graph, packs relevant code paths

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

## Supported Languages (40)

**Tier 1 — Full extraction** (functions, classes, methods, imports, exports):
Rust, TypeScript, JavaScript, Python, Java, Go, C, C++, Ruby, C#, Swift, Kotlin,
Bash, PHP, Dart, Scala, Lua, Elixir, Zig, Haskell, Groovy, Objective-C, R, Julia, OCaml, MATLAB

**Tier 2 — Structural extraction** (selectors, headings, keys, blocks, etc.):
CSS, SCSS, Markdown, JSON, YAML, TOML, Dockerfile, HCL/Terraform, Protobuf, Svelte, Makefile, HTML, GraphQL, XML

## Claude Code Plugin

`plugin/` — Claude Code plugin that wraps cxpak as slash commands and MCP tools.

Key files with version references (all must stay in sync):
- `Cargo.toml` — crate version
- `plugin/.claude-plugin/plugin.json` — plugin metadata version
- `.claude-plugin/marketplace.json` — marketplace listing version
- `plugin/lib/ensure-cxpak` — `REQUIRED_VERSION` (pinned download version)

`plugin/lib/ensure-cxpak` checks PATH and cached install, verifies version matches `REQUIRED_VERSION`, and downloads the pinned release if outdated.

`plugin/lib/ensure-cxpak-serve` does the same for the `serve` command.

## Release

Tag with `vX.Y.Z` to trigger CI: cross-compile for Linux/macOS + publish to crates.io.

When bumping version, update all four files listed under Claude Code Plugin above.
