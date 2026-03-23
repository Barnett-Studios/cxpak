# v0.10.0 Implementation Plan: 42 Languages + Search + Focus

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expand cxpak from 12 to 42 languages, add a regex search MCP tool, and add `focus` path filtering to all MCP tools.

**Architecture:** Three independent work streams — (1) structural changes to support new languages, (2) 30 new language parsers following the existing pattern, (3) search tool + focus param on serve.rs. Stream 1 must land first. Streams 2 and 3 are independent of each other.

**Tech Stack:** Rust, tree-sitter, regex crate, axum (HTTP), JSON-RPC (MCP)

**Spec:** `docs/superpowers/specs/2026-03-18-v0100-design.md`

---

## Stream 1: Structural Foundation

### Task 1: Extend `SymbolKind` with Tier 2 variants

**Files:**
- Modify: `src/parser/language.rs:16-26`
- Test: `src/parser/language.rs` (existing tests)

- [ ] **Step 1: Add new variants to `SymbolKind` enum**

In `src/parser/language.rs`, extend the enum:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SymbolKind {
    // Existing
    Function,
    Struct,
    Enum,
    Trait,
    Interface,
    Class,
    Method,
    Constant,
    TypeAlias,
    // New — Tier 2 structural units
    Selector,
    Mixin,
    Variable,
    Heading,
    Section,
    Key,
    Table,
    Block,
    Target,
    Rule,
    Element,
    Message,
    Service,
    Query,
    Mutation,
    Type,
    Instruction,
}
```

- [ ] **Step 2: Run existing tests to verify no regressions**

Run: `cargo test --verbose`
Expected: All existing tests pass — the new variants don't affect existing code.

- [ ] **Step 3: Commit**

```bash
git add src/parser/language.rs
git commit -m "feat: extend SymbolKind with 17 Tier 2 structural variants"
```

### Task 2: Extend `detect_language()` for extensionless files + new extensions

**Files:**
- Modify: `src/scanner/mod.rs:143-162`
- Test: `src/scanner/mod.rs` (add new tests)

- [ ] **Step 1: Write failing tests for filename-based detection**

Add to the test module in `src/scanner/mod.rs`:

```rust
#[test]
fn test_detect_dockerfile() {
    assert_eq!(detect_language(Path::new("Dockerfile")), Some("dockerfile".to_string()));
    assert_eq!(detect_language(Path::new("Dockerfile.prod")), Some("dockerfile".to_string()));
    assert_eq!(detect_language(Path::new("src/Dockerfile")), Some("dockerfile".to_string()));
}

#[test]
fn test_detect_makefile() {
    assert_eq!(detect_language(Path::new("Makefile")), Some("makefile".to_string()));
    assert_eq!(detect_language(Path::new("GNUmakefile")), Some("makefile".to_string()));
    assert_eq!(detect_language(Path::new("build/Makefile")), Some("makefile".to_string()));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test test_detect_dockerfile test_detect_makefile --verbose`
Expected: FAIL — current `detect_language` returns `None` for extensionless files.

- [ ] **Step 3: Write failing tests for new extension mappings**

Add tests for all 30 new languages' extensions (one test per language, verifying all variants):

```rust
#[test]
fn test_detect_new_tier1_extensions() {
    assert_eq!(detect_language(Path::new("script.sh")), Some("bash".to_string()));
    assert_eq!(detect_language(Path::new("script.bash")), Some("bash".to_string()));
    assert_eq!(detect_language(Path::new("index.php")), Some("php".to_string()));
    assert_eq!(detect_language(Path::new("main.dart")), Some("dart".to_string()));
    assert_eq!(detect_language(Path::new("App.scala")), Some("scala".to_string()));
    assert_eq!(detect_language(Path::new("build.sc")), Some("scala".to_string()));
    assert_eq!(detect_language(Path::new("init.lua")), Some("lua".to_string()));
    assert_eq!(detect_language(Path::new("mix.ex")), Some("elixir".to_string()));
    assert_eq!(detect_language(Path::new("test.exs")), Some("elixir".to_string()));
    assert_eq!(detect_language(Path::new("main.zig")), Some("zig".to_string()));
    assert_eq!(detect_language(Path::new("script.pl")), Some("perl".to_string()));
    assert_eq!(detect_language(Path::new("Module.pm")), Some("perl".to_string()));
    assert_eq!(detect_language(Path::new("Main.hs")), Some("haskell".to_string()));
    assert_eq!(detect_language(Path::new("build.groovy")), Some("groovy".to_string()));
    assert_eq!(detect_language(Path::new("build.gradle")), Some("groovy".to_string()));
    assert_eq!(detect_language(Path::new("ViewController.m")), Some("objc".to_string()));
    assert_eq!(detect_language(Path::new("ViewController.mm")), Some("objc".to_string()));
    assert_eq!(detect_language(Path::new("analysis.r")), Some("r".to_string()));
    assert_eq!(detect_language(Path::new("analysis.R")), Some("r".to_string()));
    assert_eq!(detect_language(Path::new("solver.jl")), Some("julia".to_string()));
    assert_eq!(detect_language(Path::new("parser.ml")), Some("ocaml".to_string()));
    assert_eq!(detect_language(Path::new("parser.mli")), Some("ocaml_interface".to_string()));
    assert_eq!(detect_language(Path::new("schema.sql")), Some("sql".to_string()));
}

#[test]
fn test_detect_new_tier2_extensions() {
    assert_eq!(detect_language(Path::new("style.css")), Some("css".to_string()));
    assert_eq!(detect_language(Path::new("style.scss")), Some("scss".to_string()));
    assert_eq!(detect_language(Path::new("README.md")), Some("markdown".to_string()));
    assert_eq!(detect_language(Path::new("page.mdx")), Some("markdown".to_string()));
    assert_eq!(detect_language(Path::new("config.json")), Some("json".to_string()));
    assert_eq!(detect_language(Path::new("config.yml")), Some("yaml".to_string()));
    assert_eq!(detect_language(Path::new("config.yaml")), Some("yaml".to_string()));
    assert_eq!(detect_language(Path::new("Cargo.toml")), Some("toml".to_string()));
    assert_eq!(detect_language(Path::new("main.tf")), Some("hcl".to_string()));
    assert_eq!(detect_language(Path::new("vars.tfvars")), Some("hcl".to_string()));
    assert_eq!(detect_language(Path::new("config.hcl")), Some("hcl".to_string()));
    assert_eq!(detect_language(Path::new("service.proto")), Some("proto".to_string()));
    assert_eq!(detect_language(Path::new("App.svelte")), Some("svelte".to_string()));
    assert_eq!(detect_language(Path::new("rules.mk")), Some("makefile".to_string()));
    assert_eq!(detect_language(Path::new("index.html")), Some("html".to_string()));
    assert_eq!(detect_language(Path::new("index.htm")), Some("html".to_string()));
    assert_eq!(detect_language(Path::new("schema.graphql")), Some("graphql".to_string()));
    assert_eq!(detect_language(Path::new("schema.gql")), Some("graphql".to_string()));
    assert_eq!(detect_language(Path::new("config.xml")), Some("xml".to_string()));
    assert_eq!(detect_language(Path::new("schema.xsd")), Some("xml".to_string()));
    assert_eq!(detect_language(Path::new("transform.xsl")), Some("xml".to_string()));
    assert_eq!(detect_language(Path::new("icon.svg")), Some("xml".to_string()));
}

#[test]
fn test_detect_matlab_extension() {
    // .m defaults to objc, not matlab (known ambiguity — objc wins unconditionally)
    assert_eq!(detect_language(Path::new("script.m")), Some("objc".to_string()));
}
```

- [ ] **Step 4: Implement the extended `detect_language()`**

Replace the function in `src/scanner/mod.rs`:

```rust
pub fn detect_language(path: &Path) -> Option<String> {
    // First: check by filename (for extensionless files)
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        let lang = match name {
            "Dockerfile" => Some("dockerfile"),
            "Makefile" | "GNUmakefile" => Some("makefile"),
            _ if name.starts_with("Dockerfile.") => Some("dockerfile"),
            _ => None,
        };
        if let Some(l) = lang {
            return Some(l.to_string());
        }
    }

    // Then: check by extension (case-sensitive first for .R, then lowercase for the rest)
    let raw_ext = path.extension()?.to_string_lossy();

    // Case-sensitive match first (only .R needs this)
    if raw_ext.as_ref() == "R" {
        return Some("r".to_string());
    }

    let ext = raw_ext.to_lowercase();
    let lang = match ext.as_str() {
        // Existing languages
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "java" => "java",
        "py" => "python",
        "go" => "go",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" | "hh" | "cxx" => "cpp",
        "rb" => "ruby",
        "cs" => "csharp",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        // New Tier 1
        "sh" | "bash" => "bash",
        "php" => "php",
        "dart" => "dart",
        "scala" | "sc" => "scala",
        "lua" => "lua",
        "ex" | "exs" => "elixir",
        "zig" => "zig",
        "pl" | "pm" => "perl",
        "hs" => "haskell",
        "groovy" | "gradle" => "groovy",
        "m" | "mm" => "objc",
        "r" => "r",
        "jl" => "julia",
        "ml" => "ocaml",
        "mli" => "ocaml_interface",
        "sql" => "sql",
        // New Tier 2
        "css" => "css",
        "scss" => "scss",
        "md" | "mdx" => "markdown",
        "json" => "json",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        "hcl" | "tf" | "tfvars" => "hcl",
        "proto" => "proto",
        "svelte" => "svelte",
        "mk" => "makefile",
        "html" | "htm" => "html",
        "graphql" | "gql" => "graphql",
        "xml" | "xsd" | "xsl" | "svg" => "xml",
        _ => return None,
    };
    Some(lang.to_string())
}
```

Note: `.R` (uppercase) is handled by the case-sensitive check before lowercasing. `.r` (lowercase) is handled by the lowercased match. `.pom` removed — `pom.xml` has extension `.xml`, not `.pom`.

- [ ] **Step 5: Run all tests**

Run: `cargo test --verbose`
Expected: All new tests pass, all existing tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/scanner/mod.rs
git commit -m "feat: extend detect_language for 30 new languages + extensionless files"
```

### Task 3: Add all 30 tree-sitter crates to Cargo.toml

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add optional dependencies and feature flags**

Add to `[dependencies]`:

```toml
tree-sitter-bash = { version = "0.25", optional = true }
tree-sitter-css = { version = "0.25", optional = true }
tree-sitter-scss = { version = "1.0", optional = true }
tree-sitter-sql = { version = "0.0.2", optional = true }
tree-sitter-php = { version = "0.24", optional = true }
tree-sitter-markdown = { version = "0.7", optional = true }
tree-sitter-json = { version = "0.24", optional = true }
tree-sitter-yaml = { version = "0.7", optional = true }
tree-sitter-toml = { version = "0.20", optional = true }
tree-sitter-dockerfile = { version = "0.2", optional = true }
tree-sitter-hcl = { version = "1.1", optional = true }
tree-sitter-dart = { version = "0.1", optional = true }
tree-sitter-scala = { version = "0.25", optional = true }
tree-sitter-lua = { version = "0.5", optional = true }
tree-sitter-elixir = { version = "0.3", optional = true }
tree-sitter-zig = { version = "1.1", optional = true }
tree-sitter-perl = { version = "1.1", optional = true }
tree-sitter-haskell = { version = "0.23", optional = true }
tree-sitter-groovy = { version = "0.1", optional = true }
tree-sitter-objc = { version = "3.0", optional = true }
tree-sitter-r = { version = "1.2", optional = true }
tree-sitter-julia = { version = "0.23", optional = true }
tree-sitter-ocaml = { version = "0.24", optional = true }
tree-sitter-matlab = { version = "1.3", optional = true }
tree-sitter-proto = { version = "0.4", optional = true }
tree-sitter-svelte = { version = "0.10", optional = true }
tree-sitter-make = { version = "1.1", optional = true }
tree-sitter-html = { version = "0.23", optional = true }
tree-sitter-graphql = { version = "0.1", optional = true }
tree-sitter-xml = { version = "0.7", optional = true }
regex = "1"
```

Note: Rust crate imports use underscores (`tree_sitter_json`), which does not conflict with `serde_json`. No `package` renames needed. Verify all compile with `cargo check`.

Add to `[features]`:

```toml
lang-bash = ["dep:tree-sitter-bash"]
lang-css = ["dep:tree-sitter-css"]
lang-scss = ["dep:tree-sitter-scss"]
lang-sql = ["dep:tree-sitter-sql"]
lang-php = ["dep:tree-sitter-php"]
lang-markdown = ["dep:tree-sitter-markdown"]
lang-json = ["dep:tree-sitter-json"]
lang-yaml = ["dep:tree-sitter-yaml"]
lang-toml = ["dep:tree-sitter-toml"]
lang-dockerfile = ["dep:tree-sitter-dockerfile"]
lang-hcl = ["dep:tree-sitter-hcl"]
lang-dart = ["dep:tree-sitter-dart"]
lang-scala = ["dep:tree-sitter-scala"]
lang-lua = ["dep:tree-sitter-lua"]
lang-elixir = ["dep:tree-sitter-elixir"]
lang-zig = ["dep:tree-sitter-zig"]
lang-perl = ["dep:tree-sitter-perl"]
lang-haskell = ["dep:tree-sitter-haskell"]
lang-groovy = ["dep:tree-sitter-groovy"]
lang-objc = ["dep:tree-sitter-objc"]
lang-r = ["dep:tree-sitter-r"]
lang-julia = ["dep:tree-sitter-julia"]
lang-ocaml = ["dep:tree-sitter-ocaml"]
lang-matlab = ["dep:tree-sitter-matlab"]
lang-proto = ["dep:tree-sitter-proto"]
lang-svelte = ["dep:tree-sitter-svelte"]
lang-makefile = ["dep:tree-sitter-make"]
lang-html = ["dep:tree-sitter-html"]
lang-graphql = ["dep:tree-sitter-graphql"]
lang-xml = ["dep:tree-sitter-xml"]
```

Add all 30 new `lang-*` features to the `default` list alongside the existing 12.

- [ ] **Step 2: Verify it compiles**

Run: `cargo check`
Expected: Compiles without errors. If any crate has naming conflicts, adjust the `package = "..."` rename.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "feat: add 30 tree-sitter grammar dependencies for new languages"
```

---

## Stream 2: Language Parsers (30 languages)

Each language follows the identical pattern. Tasks 4-33 below can be **parallelized** — they are fully independent of each other. Each task creates one language file, registers it, and adds tests.

### Pattern for Every Language

For each language `{name}` with struct `{Name}Language`:

**Files:**
- Create: `src/parser/languages/{name}.rs`
- Modify: `src/parser/languages/mod.rs` (add `#[cfg(feature = "lang-{name}")] pub mod {name};`)
- Modify: `src/parser/mod.rs` (add `#[cfg(feature = "lang-{name}")] self.register(Box::new(languages::{name}::{Name}Language));`)

**Steps:**
1. Write failing test in `src/parser/languages/{name}.rs` — parse a snippet, verify symbols
2. Run test, verify it fails (module doesn't exist yet)
3. Implement `{Name}Language` struct with `LanguageSupport` trait
4. Run test, verify it passes
5. Add more tests (imports, empty input, complex snippet; Tier 2 adds structural + nesting tests)
6. Run all tests
7. Commit

### Tier 1 Language Reference

Each Tier 1 language must extract from the tree-sitter AST:
- **Functions/methods**: name, signature (everything before body), body, start/end lines, visibility
- **Structs/classes/types**: name, body, visibility
- **Imports**: source path + imported names
- **Exports**: exported symbol names + kinds

Use `SymbolKind::Function`, `Method`, `Class`, `Struct`, `Constant`, `TypeAlias`, etc. from the existing enum.

### Tier 2 Language Reference

Each Tier 2 language must extract structural units. Use the new `SymbolKind` variants. Imports and exports will typically be empty (config files don't import). Focus on extracting named structural units:

| Language | Extract As |
|---|---|
| CSS | Selectors → `Selector`, @-rules → `Rule`, custom properties → `Variable` |
| SCSS | Same as CSS + mixins → `Mixin`, variables → `Variable` |
| Markdown | Headings → `Heading`, code blocks → `Block` |
| JSON | Top-level keys → `Key`, nested objects → `Block` |
| YAML | Top-level keys → `Key`, nested maps → `Block` |
| TOML | Tables → `Table`, key-value pairs → `Key` |
| Dockerfile | FROM stages → `Section`, instructions → `Instruction` |
| HCL | Blocks → `Block`, variables → `Variable` |
| Proto | Messages → `Message`, services → `Service`, enums → `Enum`, RPCs → `Method` |
| Svelte | Script → `Block`, style → `Block`, each component → `Section` |
| Makefile | Targets → `Target`, variables → `Variable`, rules → `Rule` |
| HTML | Significant elements → `Element`, head/body → `Section` |
| GraphQL | Types → `Type`, queries → `Query`, mutations → `Mutation`, subscriptions → `Query` |
| XML | Top-level elements → `Element`, named elements → `Element` |

### Task 4–19: Tier 1 Languages (16 languages)

Each follows the pattern above. Listed here for tracking:

- [ ] **Task 4: Bash** — `src/parser/languages/bash.rs`, `tree-sitter-bash`, functions + variables
- [ ] **Task 5: PHP** — `src/parser/languages/php.rs`, `tree-sitter-php` (use `LANGUAGE_PHP`), functions + classes + methods + imports
- [ ] **Task 6: Dart** — `src/parser/languages/dart.rs`, functions + classes + imports
- [ ] **Task 7: Scala** — `src/parser/languages/scala.rs`, objects + classes + defs + traits + imports
- [ ] **Task 8: Lua** — `src/parser/languages/lua.rs`, functions + local functions + requires
- [ ] **Task 9: Elixir** — `src/parser/languages/elixir.rs`, defmodule + def/defp + aliases/imports
- [ ] **Task 10: Zig** — `src/parser/languages/zig.rs`, functions + structs + consts
- [ ] **Task 11: Perl** — `src/parser/languages/perl.rs`, subs + packages + use statements
- [ ] **Task 12: Haskell** — `src/parser/languages/haskell.rs`, function bindings + type signatures + data types + imports
- [ ] **Task 13: Groovy** — `src/parser/languages/groovy.rs`, methods + classes + closures + imports
- [ ] **Task 14: Objective-C** — `src/parser/languages/objc.rs`, @interface/@implementation + methods + imports
- [ ] **Task 15: R** — `src/parser/languages/r.rs`, function assignments + library() calls
- [ ] **Task 16: Julia** — `src/parser/languages/julia.rs`, function/macro + struct + module + using/import
- [ ] **Task 17: OCaml** — `src/parser/languages/ocaml.rs`, registered as two languages: `"ocaml"` (`.ml`, uses `LANGUAGE_OCAML`) and `"ocaml_interface"` (`.mli`, uses `LANGUAGE_OCAML_INTERFACE`). Both share the same extraction logic (let bindings, type definitions, module, open). `detect_language` maps `.ml` → `"ocaml"`, `.mli` → `"ocaml_interface"`. Both use the same `lang-ocaml` feature flag.
- [ ] **Task 18: MATLAB** — `src/parser/languages/matlab.rs`, function definitions + classdef
- [ ] **Task 19: SQL** — `src/parser/languages/sql.rs`, CREATE TABLE/FUNCTION/VIEW/INDEX/TRIGGER + ALTER TABLE

### Task 20–33: Tier 2 Languages (14 languages)

- [ ] **Task 20: CSS** — `src/parser/languages/css.rs`
- [ ] **Task 21: SCSS** — `src/parser/languages/scss.rs`
- [ ] **Task 22: Markdown** — `src/parser/languages/markdown.rs`
- [ ] **Task 23: JSON** — `src/parser/languages/json_lang.rs` (avoid module name conflict with serde_json)
- [ ] **Task 24: YAML** — `src/parser/languages/yaml.rs`
- [ ] **Task 25: TOML** — `src/parser/languages/toml_lang.rs` (avoid conflict with toml crate)
- [ ] **Task 26: Dockerfile** — `src/parser/languages/dockerfile.rs`
- [ ] **Task 27: HCL** — `src/parser/languages/hcl.rs`
- [ ] **Task 28: Proto** — `src/parser/languages/proto.rs`
- [ ] **Task 29: Svelte** — `src/parser/languages/svelte.rs`
- [ ] **Task 30: Makefile** — `src/parser/languages/makefile.rs`
- [ ] **Task 31: HTML** — `src/parser/languages/html.rs`
- [ ] **Task 32: GraphQL** — `src/parser/languages/graphql.rs`
- [ ] **Task 33: XML** — `src/parser/languages/xml.rs`

### Task 34: Register all 30 languages

**Files:**
- Modify: `src/parser/languages/mod.rs`
- Modify: `src/parser/mod.rs`

- [ ] **Step 1: Add module declarations to `src/parser/languages/mod.rs`**

Add 30 new `#[cfg(feature = "lang-{name}")] pub mod {name};` entries.

- [ ] **Step 2: Add registrations to `src/parser/mod.rs` `register_defaults()`**

Add 30 new `#[cfg(feature = "lang-{name}")] self.register(Box::new(languages::{name}::{Name}Language));` entries.

- [ ] **Step 3: Update the `test_supported_languages_returns_all` test**

Change `assert!(langs.len() >= 12, ...)` to `assert!(langs.len() >= 42, ...)`.

- [ ] **Step 4: Run all tests**

Run: `cargo test --verbose`
Expected: All 42 languages registered, all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/parser/languages/mod.rs src/parser/mod.rs
git commit -m "feat: register all 30 new languages in LanguageRegistry"
```

### Task 35: Integration test — overview with all 42 languages

**Files:**
- Modify: `tests/` (add integration test)

- [ ] **Step 1: Write integration test**

Create a temp repo with one file per language, run `cxpak overview`, verify it completes and reports all languages.

- [ ] **Step 2: Run test**

Run: `cargo test integration_42_languages --verbose`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add tests/
git commit -m "test: integration test for overview with all 42 languages"
```

---

## Stream 3: Search Tool + Focus

### Task 36: Add `matches_focus` utility

**Files:**
- Modify: `src/commands/serve.rs`

- [ ] **Step 1: Write test for `matches_focus`**

```rust
#[test]
fn test_matches_focus_none() {
    assert!(matches_focus("src/api/mod.rs", None));
}

#[test]
fn test_matches_focus_with_prefix() {
    assert!(matches_focus("src/api/mod.rs", Some("src/")));
    assert!(matches_focus("src/api/mod.rs", Some("src")));
    assert!(!matches_focus("tests/api_test.rs", Some("src/")));
}

#[test]
fn test_matches_focus_empty_string() {
    assert!(matches_focus("src/api/mod.rs", Some("")));
}

#[test]
fn test_matches_focus_nested() {
    assert!(matches_focus("src/api/v2/handler.rs", Some("src/api/v2/")));
    assert!(!matches_focus("src/api/v1/handler.rs", Some("src/api/v2/")));
}
```

- [ ] **Step 2: Implement `matches_focus`**

```rust
fn matches_focus(path: &str, focus: Option<&str>) -> bool {
    focus.map_or(true, |f| path.starts_with(f))
}
```

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/commands/serve.rs
git commit -m "feat: add matches_focus utility for path filtering"
```

### Task 37: Add `focus` param to all existing MCP tools

**Files:**
- Modify: `src/commands/serve.rs` (schemas + handlers)

- [ ] **Step 1: Write failing tests for focus on each existing tool**

For each of the 6 existing tools, add two tests: one with `focus` filtering results, one without `focus` (regression).

- [ ] **Step 2: Add `focus` to each tool's JSON schema**

Add to each tool's `inputSchema.properties`:
```json
"focus": { "type": "string", "description": "Path prefix to scope results (e.g. 'src/')" }
```

- [ ] **Step 3: Extract `focus` in each handler and apply `matches_focus` filter**

Each handler gets:
```rust
let focus = args.get("focus").and_then(|f| f.as_str());
```
Then filter `index.files` with `matches_focus(&f.relative_path, focus)`.

For `cxpak_trace`: list out-of-scope deps in `out_of_scope_deps` array.
For `cxpak_pack_context`: include out-of-scope deps but flag as `"included_as": "out_of_scope_dependency"`.

- [ ] **Step 4: Run all tests**

Run: `cargo test --verbose`
Expected: All pass.

- [ ] **Step 5: Commit**

```bash
git add src/commands/serve.rs
git commit -m "feat: add focus param to all 6 existing MCP tools"
```

### Task 38: Implement `cxpak_search` MCP tool

**Files:**
- Modify: `src/commands/serve.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_mcp_search_happy_path() {
    let index = make_test_index();
    let repo_path = Path::new("/tmp");
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_search","arguments":{"pattern":"fn main"}}}"#;
    let input = format!("{request}\n");
    let mut output = Vec::new();
    mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
    let response = parse_mcp_response(&output);
    let result = &response["result"]["content"][0]["text"];
    let parsed: Value = serde_json::from_str(result.as_str().unwrap()).unwrap();
    assert!(parsed["matches"].as_array().unwrap().len() > 0);
    assert!(parsed["total_matches"].as_u64().unwrap() > 0);
}

#[test]
fn test_mcp_search_no_matches() { ... }

#[test]
fn test_mcp_search_invalid_regex() { ... }

#[test]
fn test_mcp_search_with_focus() { ... }

#[test]
fn test_mcp_search_with_limit() { ... }

#[test]
fn test_mcp_search_context_lines_zero() { ... }

#[test]
fn test_mcp_search_context_lines_large() { ... }

#[test]
fn test_mcp_search_truncation() { ... }

#[test]
fn test_mcp_search_unicode() { ... }

#[test]
fn test_mcp_search_empty_content_skipped() { ... }

#[test]
fn test_mcp_search_empty_pattern_error() { ... }
```

- [ ] **Step 2: Add `cxpak_search` to tools/list schema**

Add the tool definition to the `tools/list` response in `mcp_stdio_loop_with_io`.

- [ ] **Step 3: Implement the handler**

In `handle_tool_call`, add the `"cxpak_search"` arm:

```rust
"cxpak_search" => {
    let pattern = args.get("pattern").and_then(|p| p.as_str()).unwrap_or("");
    if pattern.is_empty() {
        return mcp_tool_result(id, "Error: 'pattern' argument is required and must not be empty");
    }
    let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(20) as usize;
    let focus = args.get("focus").and_then(|f| f.as_str());
    let context_lines = args.get("context_lines").and_then(|c| c.as_u64()).unwrap_or(2) as usize;

    let re = match regex::Regex::new(pattern) {
        Ok(r) => r,
        Err(e) => return mcp_tool_result(id, &format!("Error: invalid regex: {e}")),
    };

    let mut matches = vec![];
    let mut total_matches = 0usize;
    let mut files_searched = 0usize;

    for file in &index.files {
        if !matches_focus(&file.relative_path, focus) { continue; }
        if file.content.is_empty() { continue; }
        files_searched += 1;

        let lines: Vec<&str> = file.content.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if re.is_match(line) {
                total_matches += 1;
                if matches.len() < limit {
                    let start = i.saturating_sub(context_lines);
                    let end = (i + context_lines + 1).min(lines.len());
                    let ctx_before: Vec<&str> = lines[start..i].to_vec();
                    let ctx_after: Vec<&str> = lines[(i+1)..end].to_vec();
                    matches.push(json!({
                        "path": &file.relative_path,
                        "line": i + 1,
                        "content": line,
                        "context_before": ctx_before,
                        "context_after": ctx_after,
                    }));
                }
            }
        }
    }

    mcp_tool_result(id, &serde_json::to_string_pretty(&json!({
        "pattern": pattern,
        "matches": matches,
        "total_matches": total_matches,
        "files_searched": files_searched,
        "truncated": total_matches > limit,
    })).unwrap_or_default())
}
```

- [ ] **Step 4: Add `POST /search` HTTP endpoint**

Add to `build_router`:
```rust
.route("/search", axum::routing::post(search_handler))
```

Implement `search_handler` that deserializes JSON body and calls the same logic.

- [ ] **Step 5: Update tools/list to return 7 tools (was 6)**

- [ ] **Step 6: Run all tests**

Run: `cargo test --verbose`
Expected: All pass, including the `tools/list` test updated to expect 7 tools.

- [ ] **Step 7: Commit**

```bash
git add src/commands/serve.rs
git commit -m "feat: add cxpak_search MCP tool with regex, focus, and context lines"
```

---

## Stream 4: Documentation + Version

### Task 39: Update documentation

**Files:**
- Modify: `README.md`
- Modify: `.claude/CLAUDE.md`
- Modify: `plugin/README.md`

- [ ] **Step 1: Update README.md**

- Language count: 12 → 42
- Add full language table with tiers
- Document `cxpak_search` tool
- Document `focus` param on all tools

- [ ] **Step 2: Update CLAUDE.md**

- "Supported Languages (12)" → "Supported Languages (42)"
- List all 42
- Add new `SymbolKind` variants to architecture notes
- Document `cxpak_search` and `focus`

- [ ] **Step 3: Update plugin/README.md**

- MCP tool count: 6 → 7
- Document `cxpak_search`
- Document `focus` on all tools

- [ ] **Step 4: Commit**

```bash
git add README.md .claude/CLAUDE.md plugin/README.md
git commit -m "docs: update documentation for v0.10.0 — 42 languages, search, focus"
```

### Task 40: Version bump

**Files:**
- Modify: `Cargo.toml` (version)
- Modify: `plugin/.claude-plugin/plugin.json`
- Modify: `.claude-plugin/marketplace.json`
- Modify: `plugin/lib/ensure-cxpak`

- [ ] **Step 1: Bump version to 0.10.0 in all four files**

- [ ] **Step 2: Run full test suite**

Run: `cargo test --verbose`
Expected: All pass.

- [ ] **Step 3: Run clippy and fmt**

Run: `cargo fmt -- --check && cargo clippy --all-targets -- -D warnings`
Expected: Clean.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml plugin/.claude-plugin/plugin.json .claude-plugin/marketplace.json plugin/lib/ensure-cxpak
git commit -m "chore: bump version to 0.10.0"
```

---

## Task Summary

| Stream | Tasks | Parallelizable? |
|---|---|---|
| 1. Foundation | Tasks 1-3 (SymbolKind, detect_language, Cargo.toml) | Sequential |
| 2. Languages | Tasks 4-35 (30 parsers + registration + integration) | Tasks 4-33 fully parallel |
| 3. Search + Focus | Tasks 36-38 (utility, focus, search) | Sequential within stream |
| 4. Docs + Version | Tasks 39-40 | Sequential, after all else |

**Critical path:** Stream 1 → (Stream 2 ∥ Stream 3) → Stream 4

**Total: 40 tasks, ~212 new tests**
