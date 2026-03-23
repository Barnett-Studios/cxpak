# v0.11.0 Implementation Plan: Context Quality

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the context cxpak packs the best possible context for an LLM — progressive degradation, concept priority, chunk splitting, query expansion, and context annotations.

**Architecture:** New `src/context_quality/` module with three files (`degradation.rs`, `expansion.rs`, `annotation.rs`). The budget module calls into context_quality for rendering decisions. Existing modules get minimal changes: `relevance/signals.rs` makes `tokenize()` public and accepts optional expanded tokens, `index/mod.rs` caches detected domains, `commands/serve.rs` wires the new features into MCP tools.

**Tech Stack:** Rust, tree-sitter (re-parse for chunk splitting), tiktoken-rs (token counting)

**Spec:** `docs/superpowers/specs/2026-03-19-v0110-design.md`

---

## File Structure

### New Files
- `src/context_quality/mod.rs` — public API, re-exports
- `src/context_quality/degradation.rs` — DetailLevel, DegradedSymbol, FileRole, concept_priority(), render_symbol_at_level(), split_oversized_symbol(), allocate_with_degradation()
- `src/context_quality/expansion.rs` — Domain, CORE_SYNONYMS, DOMAIN_SYNONYMS, detect_domains(), expand_query()
- `src/context_quality/annotation.rs` — comment_syntax(), AnnotationContext, annotate_file()

### Modified Files
- `src/main.rs` — add `pub mod context_quality;`
- `src/parser/language.rs` — no changes (SymbolKind already has Tier 2 variants from v0.10.0)
- `src/relevance/signals.rs` — make `tokenize()` public, add `expanded_tokens` param to `term_frequency()` and `symbol_match()`
- `src/relevance/mod.rs` — add `expanded_tokens` field + `with_expansion()` builder to `MultiSignalScorer`
- `src/index/mod.rs` — add `domains: HashSet<Domain>` field to `CodebaseIndex`, populate at build time
- `src/budget/mod.rs` — no changes needed (`allocate_with_degradation()` lives in `context_quality/degradation.rs`)
- `src/commands/serve.rs` — wire degradation + annotations into `pack_context`, wire expansion into `context_for_task`

---

## Stream 1: Core Types + Degradation

### Task 1: Create `context_quality` module scaffold

**Files:**
- Create: `src/context_quality/mod.rs`
- Create: `src/context_quality/degradation.rs`
- Create: `src/context_quality/expansion.rs`
- Create: `src/context_quality/annotation.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Create module files with minimal content**

`src/context_quality/mod.rs`:
```rust
pub mod annotation;
pub mod degradation;
pub mod expansion;
```

`src/context_quality/degradation.rs`:
```rust
// Progressive degradation for context quality
```

`src/context_quality/expansion.rs`:
```rust
// Query expansion with hierarchical synonym maps
```

`src/context_quality/annotation.rs`:
```rust
// Language-aware context annotations
```

- [ ] **Step 2: Add module to `src/main.rs`**

Add `pub mod context_quality;` alongside the other module declarations.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check`
Expected: Compiles clean.

- [ ] **Step 4: Commit**

```bash
git add src/context_quality/ src/main.rs
git commit -m "feat: scaffold context_quality module for v0.11.0"
```

### Task 2: Implement DetailLevel, FileRole, DegradedSymbol types

**Files:**
- Modify: `src/context_quality/degradation.rs`

- [ ] **Step 1: Write failing tests for DetailLevel ordering**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detail_level_ordering() {
        assert!(DetailLevel::Full < DetailLevel::Trimmed);
        assert!(DetailLevel::Trimmed < DetailLevel::Documented);
        assert!(DetailLevel::Documented < DetailLevel::Signature);
        assert!(DetailLevel::Signature < DetailLevel::Stub);
    }

    #[test]
    fn test_detail_level_equality() {
        assert_eq!(DetailLevel::Full, DetailLevel::Full);
        assert_ne!(DetailLevel::Full, DetailLevel::Stub);
    }

    #[test]
    fn test_file_role_variants() {
        let selected = FileRole::Selected;
        let dep = FileRole::Dependency;
        assert_ne!(selected, dep);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test context_quality::degradation --verbose`
Expected: FAIL — types don't exist yet.

- [ ] **Step 3: Implement the types**

```rust
use crate::parser::language::{Symbol, SymbolKind};
use crate::relevance::SignalResult;

pub const MAX_SYMBOL_TOKENS: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DetailLevel {
    Full = 0,
    Trimmed = 1,
    Documented = 2,
    Signature = 3,
    Stub = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileRole {
    Selected,
    Dependency,
}

#[derive(Debug, Clone)]
pub struct DegradedSymbol {
    pub symbol: Symbol,
    pub level: DetailLevel,
    pub rendered: String,
    pub rendered_tokens: usize,
    pub chunk_index: Option<usize>,
    pub chunk_total: Option<usize>,
    pub parent_name: Option<String>,
}
```

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test context_quality::degradation --verbose`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/context_quality/degradation.rs
git commit -m "feat: add DetailLevel, FileRole, DegradedSymbol types"
```

### Task 3: Implement `concept_priority()`

**Files:**
- Modify: `src/context_quality/degradation.rs`

- [ ] **Step 1: Write failing tests — one per priority tier**

```rust
#[test]
fn test_concept_priority_definitions() {
    assert_eq!(concept_priority(&SymbolKind::Function), 1.00);
    assert_eq!(concept_priority(&SymbolKind::Method), 1.00);
}

#[test]
fn test_concept_priority_structures() {
    assert_eq!(concept_priority(&SymbolKind::Struct), 0.86);
    assert_eq!(concept_priority(&SymbolKind::Class), 0.86);
    assert_eq!(concept_priority(&SymbolKind::Enum), 0.86);
    assert_eq!(concept_priority(&SymbolKind::Interface), 0.86);
    assert_eq!(concept_priority(&SymbolKind::Trait), 0.86);
    assert_eq!(concept_priority(&SymbolKind::Type), 0.86);
    assert_eq!(concept_priority(&SymbolKind::TypeAlias), 0.86);
}

#[test]
fn test_concept_priority_api_surface() {
    assert_eq!(concept_priority(&SymbolKind::Message), 0.71);
    assert_eq!(concept_priority(&SymbolKind::Service), 0.71);
    assert_eq!(concept_priority(&SymbolKind::Query), 0.71);
    assert_eq!(concept_priority(&SymbolKind::Mutation), 0.71);
    assert_eq!(concept_priority(&SymbolKind::Table), 0.71);
}

#[test]
fn test_concept_priority_configuration() {
    assert_eq!(concept_priority(&SymbolKind::Key), 0.57);
    assert_eq!(concept_priority(&SymbolKind::Block), 0.57);
    assert_eq!(concept_priority(&SymbolKind::Variable), 0.57);
    assert_eq!(concept_priority(&SymbolKind::Target), 0.57);
    assert_eq!(concept_priority(&SymbolKind::Rule), 0.57);
    assert_eq!(concept_priority(&SymbolKind::Instruction), 0.57);
    assert_eq!(concept_priority(&SymbolKind::Selector), 0.57);
    assert_eq!(concept_priority(&SymbolKind::Mixin), 0.57);
}

#[test]
fn test_concept_priority_documentation() {
    assert_eq!(concept_priority(&SymbolKind::Heading), 0.43);
    assert_eq!(concept_priority(&SymbolKind::Section), 0.43);
    assert_eq!(concept_priority(&SymbolKind::Element), 0.43);
}

#[test]
fn test_concept_priority_constants() {
    assert_eq!(concept_priority(&SymbolKind::Constant), 0.29);
}

#[test]
fn test_concept_priority_ordering_is_monotonic() {
    // Definitions > Structures > API > Config > Docs > Constants
    assert!(concept_priority(&SymbolKind::Function) > concept_priority(&SymbolKind::Struct));
    assert!(concept_priority(&SymbolKind::Struct) > concept_priority(&SymbolKind::Message));
    assert!(concept_priority(&SymbolKind::Message) > concept_priority(&SymbolKind::Key));
    assert!(concept_priority(&SymbolKind::Key) > concept_priority(&SymbolKind::Heading));
    assert!(concept_priority(&SymbolKind::Heading) > concept_priority(&SymbolKind::Constant));
}

#[test]
fn test_file_concept_priority_max_wins() {
    let symbols = vec![
        make_fn_symbol("f", 10),  // Function → 1.00
        Symbol { kind: SymbolKind::Constant, ..make_fn_symbol("c", 5) },  // Constant → 0.29
    ];
    assert_eq!(file_concept_priority(&symbols), 1.00);
}

#[test]
fn test_file_concept_priority_empty() {
    assert_eq!(file_concept_priority(&[]), 0.0);
}

#[test]
fn test_file_concept_priority_single_symbol() {
    let symbols = vec![Symbol { kind: SymbolKind::Key, ..make_fn_symbol("k", 5) }];
    assert_eq!(file_concept_priority(&symbols), 0.57);
}
```

- [ ] **Step 2: Implement `concept_priority()`**

```rust
pub fn concept_priority(kind: &SymbolKind) -> f64 {
    match kind {
        SymbolKind::Function | SymbolKind::Method => 1.00,
        SymbolKind::Struct | SymbolKind::Class | SymbolKind::Enum
        | SymbolKind::Interface | SymbolKind::Trait
        | SymbolKind::Type | SymbolKind::TypeAlias => 0.86,
        SymbolKind::Message | SymbolKind::Service
        | SymbolKind::Query | SymbolKind::Mutation
        | SymbolKind::Table => 0.71,
        SymbolKind::Key | SymbolKind::Block | SymbolKind::Variable
        | SymbolKind::Target | SymbolKind::Rule
        | SymbolKind::Instruction | SymbolKind::Selector
        | SymbolKind::Mixin => 0.57,
        SymbolKind::Heading | SymbolKind::Section
        | SymbolKind::Element => 0.43,
        SymbolKind::Constant => 0.29,
        _ => 0.14, // Imports and any future variants
    }
}

/// Compute the concept priority for a file based on its highest-priority symbol.
pub fn file_concept_priority(symbols: &[Symbol]) -> f64 {
    symbols.iter()
        .map(|s| concept_priority(&s.kind))
        .fold(0.0_f64, f64::max)
}
```

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/context_quality/degradation.rs
git commit -m "feat: implement concept_priority with 7 priority tiers"
```

### Task 4: Implement `render_symbol_at_level()`

**Files:**
- Modify: `src/context_quality/degradation.rs`

- [ ] **Step 1: Write failing tests for each detail level**

```rust
use crate::parser::language::{Symbol, SymbolKind, Visibility};

fn make_test_symbol() -> Symbol {
    Symbol {
        name: "handle_request".to_string(),
        kind: SymbolKind::Function,
        visibility: Visibility::Public,
        signature: "pub fn handle_request(req: &Request) -> Response".to_string(),
        body: "pub fn handle_request(req: &Request) -> Response {\n    // line 1\n    // line 2\n    // line 3\n    // line 4\n    // line 5\n    // line 6\n    // line 7\n    // line 8\n    // line 9\n    // line 10\n    // line 11\n    // line 12\n    // line 13\n    // line 14\n    // line 15\n    // line 16\n    // line 17\n    // line 18\n    // line 19\n    // line 20\n    // line 21\n    // line 22\n    // line 23\n    // line 24\n    // line 25\n}".to_string(),
        start_line: 1,
        end_line: 27,
    }
}

#[test]
fn test_render_full_includes_entire_body() {
    let sym = make_test_symbol();
    let result = render_symbol_at_level(&sym, DetailLevel::Full);
    assert_eq!(result.level, DetailLevel::Full);
    assert_eq!(result.rendered, sym.body);
}

#[test]
fn test_render_trimmed_truncates_at_20_lines() {
    let sym = make_test_symbol();
    let result = render_symbol_at_level(&sym, DetailLevel::Trimmed);
    assert_eq!(result.level, DetailLevel::Trimmed);
    assert!(result.rendered.contains("// line 1"));
    assert!(result.rendered.contains("// line 20"));
    assert!(!result.rendered.contains("// line 21"));
    assert!(result.rendered.contains("... 5 more lines"));
}

#[test]
fn test_render_trimmed_short_body_no_truncation() {
    let mut sym = make_test_symbol();
    sym.body = "pub fn short() {\n    return 1;\n}".to_string();
    let result = render_symbol_at_level(&sym, DetailLevel::Trimmed);
    assert!(!result.rendered.contains("more lines"));
}

#[test]
fn test_render_documented_signature_only() {
    let sym = make_test_symbol();
    let result = render_symbol_at_level(&sym, DetailLevel::Documented);
    assert!(result.rendered.contains("pub fn handle_request"));
    assert!(!result.rendered.contains("// line 1"));
}

#[test]
fn test_render_signature_one_line() {
    let sym = make_test_symbol();
    let result = render_symbol_at_level(&sym, DetailLevel::Signature);
    assert_eq!(result.rendered.trim(), sym.signature);
}

#[test]
fn test_render_stub_compact() {
    let sym = make_test_symbol();
    let result = render_symbol_at_level(&sym, DetailLevel::Stub);
    assert!(result.rendered.contains("handle_request"));
    assert!(result.rendered.contains("+"));
    assert!(result.rendered.len() < 100);
}
```

- [ ] **Step 2: Implement `render_symbol_at_level()`**

```rust
use crate::budget::counter::TokenCounter;

pub fn render_symbol_at_level(symbol: &Symbol, level: DetailLevel) -> DegradedSymbol {
    let counter = TokenCounter::new();
    let rendered = match level {
        DetailLevel::Full => symbol.body.clone(),
        DetailLevel::Trimmed => render_trimmed(symbol),
        DetailLevel::Documented => render_documented(symbol),
        DetailLevel::Signature => symbol.signature.clone(),
        DetailLevel::Stub => render_stub(symbol),
    };
    let rendered_tokens = counter.count(&rendered);
    DegradedSymbol {
        symbol: symbol.clone(),
        level,
        rendered,
        rendered_tokens,
        chunk_index: None,
        chunk_total: None,
        parent_name: None,
    }
}

fn render_trimmed(symbol: &Symbol) -> String {
    let lines: Vec<&str> = symbol.body.lines().collect();
    if lines.len() <= 21 {
        // Signature line + 20 body lines — no truncation needed
        return symbol.body.clone();
    }
    // First 21 lines (signature + 20 body lines)
    let kept: Vec<&str> = lines[..21].to_vec();
    let omitted = lines.len() - 21;
    format!("{}\n    // ... {} more lines", kept.join("\n"), omitted)
}

fn render_documented(symbol: &Symbol) -> String {
    // Extract doc comment lines from body (lines starting with ///, #, --, etc.)
    let mut doc_lines = Vec::new();
    for line in symbol.body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("///")
            || trimmed.starts_with("//!")
            || trimmed.starts_with("/**")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("*/")
            || trimmed.starts_with("# ")  // hash-space only (avoids Rust #[derive] etc.)
            || trimmed.starts_with("--")
            || trimmed.starts_with("\"\"\"")
        {
            doc_lines.push(line);
        } else if !doc_lines.is_empty() {
            break; // doc comment ended
        }
    }
    if doc_lines.is_empty() {
        symbol.signature.clone()
    } else {
        format!("{}\n{}", doc_lines.join("\n"), symbol.signature)
    }
}

fn render_stub(symbol: &Symbol) -> String {
    let line_count = symbol.body.lines().count();
    format!("{} // +{} lines", symbol.signature, line_count)
}
```

- [ ] **Step 3: Add tests for doc comment extraction variants**

```rust
#[test]
fn test_render_documented_rust_doc_comment() {
    let mut sym = make_test_symbol();
    sym.body = "/// Handles incoming requests.\n/// Returns a Response.\npub fn handle_request(req: &Request) -> Response {\n    todo!()\n}".to_string();
    let result = render_symbol_at_level(&sym, DetailLevel::Documented);
    assert!(result.rendered.contains("Handles incoming requests"));
    assert!(result.rendered.contains(&sym.signature));
}

#[test]
fn test_render_documented_python_docstring() {
    let mut sym = make_test_symbol();
    sym.body = "\"\"\"Handle incoming requests.\"\"\"\ndef handle_request(req):".to_string();
    sym.signature = "def handle_request(req):".to_string();
    let result = render_symbol_at_level(&sym, DetailLevel::Documented);
    assert!(result.rendered.contains("Handle incoming requests"));
}

#[test]
fn test_render_documented_java_javadoc() {
    let mut sym = make_test_symbol();
    sym.body = "/**\n * Handles incoming requests.\n * @param req the request\n */\npublic Response handleRequest(Request req) {".to_string();
    sym.signature = "public Response handleRequest(Request req)".to_string();
    let result = render_symbol_at_level(&sym, DetailLevel::Documented);
    assert!(result.rendered.contains("Handles incoming requests"));
}

#[test]
fn test_render_documented_no_doc_comment() {
    let mut sym = make_test_symbol();
    sym.body = "pub fn no_docs() {\n    todo!()\n}".to_string();
    let result = render_symbol_at_level(&sym, DetailLevel::Documented);
    assert_eq!(result.rendered, sym.signature);
}

#[test]
fn test_render_documented_ruby_hash_comment() {
    let mut sym = make_test_symbol();
    sym.body = "# Handles incoming requests.\n# @param req [Request]\ndef handle_request(req)".to_string();
    sym.signature = "def handle_request(req)".to_string();
    let result = render_symbol_at_level(&sym, DetailLevel::Documented);
    assert!(result.rendered.contains("Handles incoming requests"));
}
```

- [ ] **Step 4: Run all tests, verify pass**

Run: `cargo test context_quality::degradation --verbose`

- [ ] **Step 5: Commit**

```bash
git add src/context_quality/degradation.rs
git commit -m "feat: implement render_symbol_at_level with 5 detail levels"
```

### Task 5: Implement chunk splitting for oversized symbols

**Files:**
- Modify: `src/context_quality/degradation.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_split_symbol_under_limit_no_split() {
    let sym = make_test_symbol(); // small symbol
    let chunks = split_oversized_symbol(&sym, &sym.body);
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].chunk_index.is_none());
}

#[test]
fn test_split_symbol_over_limit() {
    // Create a symbol with >4000 tokens (~16000 chars)
    let big_body = (0..500).map(|i| format!("    let var_{i} = compute_something_{i}(arg1, arg2, arg3);")).collect::<Vec<_>>().join("\n");
    let sig = "pub fn huge_function()".to_string();
    let body = format!("{sig} {{\n{big_body}\n}}");
    let sym = Symbol {
        name: "huge_function".to_string(),
        kind: SymbolKind::Function,
        visibility: Visibility::Public,
        signature: sig,
        body: body.clone(),
        start_line: 1,
        end_line: 502,
    };
    let chunks = split_oversized_symbol(&sym, &body);
    assert!(chunks.len() > 1, "should split into multiple chunks, got {}", chunks.len());
    // Each chunk should have metadata
    for (i, chunk) in chunks.iter().enumerate() {
        assert_eq!(chunk.chunk_index, Some(i));
        assert_eq!(chunk.chunk_total, Some(chunks.len()));
        assert_eq!(chunk.parent_name.as_deref(), Some("huge_function"));
    }
}

#[test]
fn test_split_chunk_naming() {
    let big_body = (0..500).map(|i| format!("    let v{i} = f{i}();")).collect::<Vec<_>>().join("\n");
    let body = format!("pub fn big() {{\n{big_body}\n}}");
    let sym = Symbol {
        name: "big".to_string(),
        kind: SymbolKind::Function,
        visibility: Visibility::Public,
        signature: "pub fn big()".to_string(),
        body: body.clone(),
        start_line: 1,
        end_line: 502,
    };
    let chunks = split_oversized_symbol(&sym, &body);
    assert!(chunks[0].symbol.name.contains("[1/"));
    assert!(chunks[1].symbol.name.contains("[2/"));
}

#[test]
fn test_split_preserves_signature_in_chunks() {
    let big_body = (0..500).map(|i| format!("    let v{i} = f{i}();")).collect::<Vec<_>>().join("\n");
    let body = format!("pub fn big() {{\n{big_body}\n}}");
    let sym = Symbol {
        name: "big".to_string(),
        kind: SymbolKind::Function,
        visibility: Visibility::Public,
        signature: "pub fn big()".to_string(),
        body: body.clone(),
        start_line: 1,
        end_line: 502,
    };
    let chunks = split_oversized_symbol(&sym, &body);
    for chunk in &chunks {
        assert!(chunk.symbol.body.contains("pub fn big()"),
            "each chunk should contain parent signature");
    }
}

#[test]
fn test_split_exactly_at_limit_no_split() {
    // Create a symbol with exactly MAX_SYMBOL_TOKENS tokens — should not split
    // Use a known token count: ~4 chars per token
    let line = "let x = 1;\n";
    let count = MAX_SYMBOL_TOKENS * 4 / line.len();
    let body = format!("fn f() {{\n{}\n}}", line.repeat(count));
    let sym = Symbol {
        name: "f".to_string(),
        kind: SymbolKind::Function,
        visibility: Visibility::Public,
        signature: "fn f()".to_string(),
        body: body.clone(),
        start_line: 1,
        end_line: count + 2,
    };
    // This may or may not split depending on exact token count — test the boundary
    let chunks = split_oversized_symbol(&sym, &body);
    // At least it should not panic
    assert!(!chunks.is_empty());
}

#[test]
fn test_split_line_numbers_adjusted() {
    let big_body = (0..500).map(|i| format!("    let v{i} = f{i}();")).collect::<Vec<_>>().join("\n");
    let body = format!("pub fn big() {{\n{big_body}\n}}");
    let sym = Symbol {
        name: "big".to_string(),
        kind: SymbolKind::Function,
        visibility: Visibility::Public,
        signature: "pub fn big()".to_string(),
        body: body.clone(),
        start_line: 10,
        end_line: 512,
    };
    let chunks = split_oversized_symbol(&sym, &body);
    // First chunk starts at original start_line
    assert_eq!(chunks[0].symbol.start_line, 10);
    // Last chunk ends at original end_line
    assert_eq!(chunks.last().unwrap().symbol.end_line, 512);
    // No overlapping line ranges
    for i in 1..chunks.len() {
        assert!(chunks[i].symbol.start_line > chunks[i-1].symbol.start_line);
    }
}
```

- [ ] **Step 2: Implement `split_oversized_symbol()`**

```rust
pub fn split_oversized_symbol(symbol: &Symbol, source: &str) -> Vec<DegradedSymbol> {
    let counter = TokenCounter::new();
    let total_tokens = counter.count(&symbol.body);

    if total_tokens <= MAX_SYMBOL_TOKENS {
        return vec![DegradedSymbol {
            symbol: symbol.clone(),
            level: DetailLevel::Full,
            rendered: symbol.body.clone(),
            rendered_tokens: total_tokens,
            chunk_index: None,
            chunk_total: None,
            parent_name: None,
        }];
    }

    // Strategy: try AST-aware split first, fall back to blank-line, then hard split.
    // AST-aware: re-parse source with tree-sitter, find top-level body children.
    // For now, use line-based splitting with blank-line preference.
    // TODO during implementation: add tree-sitter re-parse for AST-aware boundaries
    // when the LanguageRegistry is accessible (pass it as parameter or re-parse inline).
    let lines: Vec<&str> = symbol.body.lines().collect();
    let mut chunks: Vec<Vec<&str>> = Vec::new();
    let mut current_chunk: Vec<&str> = Vec::new();
    let mut current_tokens = 0usize;

    for line in &lines {
        let line_tokens = counter.count(line) + 1; // +1 for newline
        if current_tokens + line_tokens > MAX_SYMBOL_TOKENS && !current_chunk.is_empty() {
            chunks.push(current_chunk);
            current_chunk = Vec::new();
            current_tokens = 0;
        }
        current_chunk.push(line);
        current_tokens += line_tokens;
    }
    if !current_chunk.is_empty() {
        chunks.push(current_chunk);
    }

    let total_chunks = chunks.len();
    let total_lines = lines.len();
    let mut result = Vec::new();
    let mut line_offset = 0usize;

    for (i, chunk_lines) in chunks.iter().enumerate() {
        let chunk_content = if i == 0 {
            chunk_lines.join("\n")
        } else {
            format!("{} {{ // chunk {}/{}\n{}", symbol.signature, i + 1, total_chunks, chunk_lines.join("\n"))
        };

        let chunk_line_count = chunk_lines.len();
        let start_line = symbol.start_line + line_offset;
        let end_line = if i == total_chunks - 1 {
            symbol.end_line
        } else {
            start_line + chunk_line_count - 1
        };

        let rendered_tokens = counter.count(&chunk_content);

        result.push(DegradedSymbol {
            symbol: Symbol {
                name: format!("{} [{}/{}]", symbol.name, i + 1, total_chunks),
                kind: symbol.kind.clone(),
                visibility: symbol.visibility.clone(),
                signature: symbol.signature.clone(),
                body: chunk_content.clone(),
                start_line,
                end_line,
            },
            level: DetailLevel::Full,
            rendered: chunk_content,
            rendered_tokens,
            chunk_index: Some(i),
            chunk_total: Some(total_chunks),
            parent_name: Some(symbol.name.clone()),
        });

        line_offset += chunk_line_count;
    }

    result
}
```

- [ ] **Step 3: Run tests, verify pass**

Run: `cargo test context_quality::degradation --verbose`

- [ ] **Step 4: Commit**

```bash
git add src/context_quality/degradation.rs
git commit -m "feat: implement chunk splitting for oversized symbols (>4000 tokens)"
```

### Task 6: Implement budget allocation with degradation

**Files:**
- Modify: `src/context_quality/degradation.rs`

- [ ] **Step 1: Write failing tests**

```rust
use crate::index::IndexedFile;
use crate::parser::language::ParseResult;

fn make_indexed_file(path: &str, tokens: usize, symbols: Vec<Symbol>) -> IndexedFile {
    IndexedFile {
        relative_path: path.to_string(),
        language: Some("rust".to_string()),
        size_bytes: (tokens * 4) as u64,
        token_count: tokens,
        parse_result: Some(ParseResult { symbols, imports: vec![], exports: vec![] }),
        content: "x ".repeat(tokens),
    }
}

fn make_fn_symbol(name: &str, tokens: usize) -> Symbol {
    Symbol {
        name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: Visibility::Public,
        signature: format!("pub fn {}()", name),
        body: "x ".repeat(tokens),
        start_line: 1,
        end_line: tokens / 10,
    }
}

#[test]
fn test_allocate_fits_at_level0() {
    let files = vec![
        (make_indexed_file("a.rs", 100, vec![make_fn_symbol("a", 100)]), FileRole::Selected, 0.8),
    ];
    let result = allocate_with_degradation(&files, 1000);
    assert_eq!(result[0].1, DetailLevel::Full);
}

#[test]
fn test_allocate_degrades_lowest_score() {
    let files = vec![
        (make_indexed_file("high.rs", 600, vec![make_fn_symbol("high", 600)]), FileRole::Selected, 0.9),
        (make_indexed_file("low.rs", 600, vec![make_fn_symbol("low", 600)]), FileRole::Dependency, 0.3),
    ];
    let result = allocate_with_degradation(&files, 800);
    // high.rs should stay at higher detail than low.rs
    let high = result.iter().find(|r| r.0 == "high.rs").unwrap();
    let low = result.iter().find(|r| r.0 == "low.rs").unwrap();
    assert!(high.1 < low.1, "high-scored should be at better (lower) detail level");
}

#[test]
fn test_allocate_selected_never_below_documented() {
    let files = vec![
        (make_indexed_file("sel.rs", 5000, vec![make_fn_symbol("sel", 5000)]), FileRole::Selected, 0.5),
    ];
    let result = allocate_with_degradation(&files, 100);
    let sel = result.iter().find(|r| r.0 == "sel.rs").unwrap();
    assert!(sel.1 <= DetailLevel::Documented, "selected file should not degrade below Documented");
}

#[test]
fn test_allocate_dependency_can_be_dropped() {
    let files = vec![
        (make_indexed_file("sel.rs", 500, vec![make_fn_symbol("sel", 500)]), FileRole::Selected, 0.9),
        (make_indexed_file("dep.rs", 500, vec![make_fn_symbol("dep", 500)]), FileRole::Dependency, 0.1),
    ];
    let result = allocate_with_degradation(&files, 300);
    // dep.rs may be dropped entirely
    let dep = result.iter().find(|r| r.0 == "dep.rs");
    if let Some(d) = dep {
        assert_eq!(d.1, DetailLevel::Stub);
    }
    // sel.rs should still be present
    assert!(result.iter().any(|r| r.0 == "sel.rs"));
}

#[test]
fn test_allocate_empty_files() {
    let files: Vec<(&IndexedFile, FileRole, f64)> = vec![];
    let result = allocate_with_degradation(&files, 1000);
    assert!(result.is_empty());
}

#[test]
fn test_allocate_single_file_exact_budget() {
    let files = vec![
        (make_indexed_file("exact.rs", 1000, vec![make_fn_symbol("exact", 1000)]), FileRole::Selected, 0.8),
    ];
    let result = allocate_with_degradation(&files, 1000);
    assert_eq!(result[0].1, DetailLevel::Full);
}
```

- [ ] **Step 2: Implement `allocate_with_degradation()`**

The function takes `&[(&IndexedFile, FileRole, f64)]` (reference to file, role, relevance score) and a budget, returns `Vec<(String, DetailLevel, Vec<DegradedSymbol>)>` — path, chosen level, rendered symbols. Uses references because `IndexedFile` does not implement `Clone` and must not be moved out of the index.

Implements the algorithm from the spec: fast path check → try Level 0 → degrade dependencies first → degrade selected → drop dependencies as last resort.

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/context_quality/degradation.rs
git commit -m "feat: implement budget allocation with progressive degradation"
```

---

## Stream 2: Query Expansion

### Task 7: Make `tokenize()` public (prerequisite for expansion)

**Files:**
- Modify: `src/relevance/signals.rs`

- [ ] **Step 1: Change `fn tokenize(` to `pub fn tokenize(`**

In `src/relevance/signals.rs`, change the visibility of the `tokenize` function from private to public. This is needed by `context_quality/expansion.rs` to ensure consistent tokenization.

- [ ] **Step 2: Run existing tests to verify no regressions**

Run: `cargo test relevance --verbose`
Expected: All existing tests pass — visibility change only.

- [ ] **Step 3: Commit**

```bash
git add src/relevance/signals.rs
git commit -m "refactor: make tokenize() public for use by expansion module"
```

### Task 8: Implement core synonym map

**Files:**
- Modify: `src/context_quality/expansion.rs`

- [ ] **Step 1: Write failing tests — one per core synonym entry**

Write 30 tests, one per core synonym key, verifying that `expand_query("auth", &HashSet::new())` returns a set containing "authentication", "login", etc. The `expand_query()` function calls `crate::relevance::signals::tokenize()` (now public from Task 7).

- [ ] **Step 2: Implement CORE_SYNONYMS HashMap and `expand_query()`**

Use `std::sync::LazyLock` (Rust 1.80+) or `once_cell::sync::Lazy` for static initialization. The synonym map is a `HashMap<&'static str, &'static [&'static str]>`.

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/context_quality/expansion.rs
git commit -m "feat: implement core synonym map with ~30 entries"
```

### Task 9: Implement domain detection

**Files:**
- Modify: `src/context_quality/expansion.rs`

- [ ] **Step 1: Write failing tests — one per domain**

```rust
#[test]
fn test_detect_web_domain() { ... }  // files with .html extension
#[test]
fn test_detect_database_domain() { ... }  // files with .sql extension
#[test]
fn test_detect_auth_domain() { ... }  // path contains "auth"
#[test]
fn test_detect_infra_domain() { ... }  // .tf extension
#[test]
fn test_detect_testing_domain() { ... }  // path contains "test"
#[test]
fn test_detect_api_domain() { ... }  // path contains "handler"
#[test]
fn test_detect_mobile_domain() { ... }  // .swift extension
#[test]
fn test_detect_ml_domain() { ... }  // .ipynb present
#[test]
fn test_detect_no_domains() { ... }  // plain Rust project, no domains
#[test]
fn test_detect_multiple_domains() { ... }  // web + api + testing
```

- [ ] **Step 2: Implement `detect_domains()` and `Domain` enum**

Per the spec: uses file extensions from paths (not language field) and filenames for Dockerfile. ML requires `.ipynb`.

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/context_quality/expansion.rs
git commit -m "feat: implement domain detection with 8 domains"
```

### Task 10: Implement domain synonym maps

**Files:**
- Modify: `src/context_quality/expansion.rs`

- [ ] **Step 1: Add domain synonym entries**

Add `DOMAIN_SYNONYMS: HashMap<Domain, HashMap<&str, &[&str]>>` with all 8 domain maps from the spec.

- [ ] **Step 2: Update `expand_query()` to use domain synonyms**

When domains are provided, also look up each query token in the active domain maps.

- [ ] **Step 3: Write integration tests**

```rust
#[test]
fn test_expand_with_web_domain_active() {
    let mut domains = HashSet::new();
    domains.insert(Domain::Web);
    let expanded = expand_query("component state", &domains);
    assert!(expanded.contains("widget")); // web synonym for "component"
    assert!(expanded.contains("reducer")); // web synonym for "state"
}

#[test]
fn test_expand_no_domains_core_only() {
    let expanded = expand_query("auth", &HashSet::new());
    assert!(expanded.contains("authentication")); // core synonym
    assert!(!expanded.contains("saml")); // auth domain synonym — not active
}

#[test]
fn test_expand_empty_query() {
    let expanded = expand_query("", &HashSet::new());
    assert!(expanded.is_empty());
}

#[test]
fn test_expand_unknown_term_passthrough() {
    let expanded = expand_query("xyzzy", &HashSet::new());
    assert!(expanded.contains("xyzzy")); // original token preserved
    assert_eq!(expanded.len(), 1); // no synonyms added
}
```

- [ ] **Step 4: Run tests, verify pass**

- [ ] **Step 5: Commit**

```bash
git add src/context_quality/expansion.rs
git commit -m "feat: implement domain synonym maps with 8 domain categories"
```

### Task 11: Wire expansion into relevance module

**Files:**
- Modify: `src/relevance/signals.rs`
- Modify: `src/relevance/mod.rs`

- [ ] **Step 1: `tokenize()` is already public from Task 7 — skip**

- [ ] **Step 2: Add `expanded_tokens` parameter to `term_frequency()` and `symbol_match()`**

Add `expanded_tokens: Option<&HashSet<String>>` as the last parameter. When `Some`, use it instead of calling `tokenize(query)`. All existing call sites in `signals.rs` tests pass `None`.

- [ ] **Step 3: Update `MultiSignalScorer` with expansion support**

**CRITICAL: The `RelevanceScorer` trait signature does NOT change.** The trait method remains:
```rust
fn score(&self, query: &str, file_path: &str, index: &CodebaseIndex) -> ScoredFile;
```

Add `expanded_tokens: Option<HashSet<String>>` field to `MultiSignalScorer`. Add builder:
```rust
pub fn with_expansion(mut self, tokens: HashSet<String>) -> Self {
    self.expanded_tokens = Some(tokens);
    self
}
```

The trait impl at `impl RelevanceScorer for MultiSignalScorer` reads from `self.expanded_tokens` internally:
```rust
impl RelevanceScorer for MultiSignalScorer {
    fn score(&self, query: &str, file_path: &str, index: &CodebaseIndex) -> ScoredFile {
        let expanded = self.expanded_tokens.as_ref();
        // ...
        let symbol_sig = signals::symbol_match(query, file_path, index, expanded);
        let tf_sig = signals::term_frequency(query, file_path, index, expanded);
        // ... rest unchanged
    }
}
```

`score_all()` calls `self.score()` which resolves to this trait impl — so expansion is automatically used for all files.

- [ ] **Step 4: Write tests verifying expansion integration**

```rust
#[test]
fn test_term_frequency_with_expanded_tokens() {
    // ... create index with file containing "authentication"
    // ... call term_frequency with expanded_tokens containing "authentication"
    // ... verify score > 0 even though raw query was "auth"
}

#[test]
fn test_symbol_match_with_expanded_tokens() { ... }

#[test]
fn test_term_frequency_none_expansion_unchanged() { ... }

#[test]
fn test_symbol_match_none_expansion_unchanged() { ... }

#[test]
fn test_scorer_with_expansion_score_all() { ... }
```

- [ ] **Step 5: Verify all existing relevance tests still pass**

Run: `cargo test relevance --verbose`

- [ ] **Step 6: Commit**

```bash
git add src/relevance/signals.rs src/relevance/mod.rs
git commit -m "feat: wire query expansion into relevance scoring signals"
```

### Task 12: Add domains to CodebaseIndex

**Files:**
- Modify: `src/index/mod.rs`

- [ ] **Step 1: Add `domains` field to `CodebaseIndex`**

```rust
use std::collections::HashSet;
use crate::context_quality::expansion::Domain;

pub struct CodebaseIndex {
    // ... existing fields
    pub domains: HashSet<Domain>,
}
```

- [ ] **Step 2: Populate domains in `build()` and `build_with_content()`**

Note: `detect_domains()` accepts `&[IndexedFile]` (not `&CodebaseIndex`) so it can be called during index construction before `Self` is fully built. Call it on the `files` vec, then store the result in the struct:

```rust
let domains = crate::context_quality::expansion::detect_domains(&files);
Self {
    files,
    // ... other fields
    domains,
}
```

- [ ] **Step 3: Write tests**

```rust
#[test]
fn test_index_detects_domains() {
    // Build index with .sql file → Database domain detected
    // ...
}

#[test]
fn test_index_no_domains_plain_project() {
    // Build index with only .rs files → no domains
    // ...
}
```

- [ ] **Step 4: Run all tests**

Run: `cargo test --verbose`

- [ ] **Step 5: Commit**

```bash
git add src/index/mod.rs
git commit -m "feat: add domain detection to CodebaseIndex build"
```

---

## Stream 3: Annotations

### Task 13: Implement `comment_syntax()`

**Files:**
- Modify: `src/context_quality/annotation.rs`

- [ ] **Step 1: Write failing tests — one per language family**

```rust
#[test]
fn test_comment_syntax_c_style() {
    let (pre, suf) = comment_syntax("rust");
    assert_eq!(pre, "// ");
    assert_eq!(suf, "");
    // Also test: javascript, typescript, java, go, c, cpp, csharp, swift, kotlin,
    // scala, dart, zig, groovy, objc, proto, graphql
}

#[test]
fn test_comment_syntax_hash() {
    let (pre, suf) = comment_syntax("python");
    assert_eq!(pre, "# ");
    assert_eq!(suf, "");
    // Also test: ruby, bash, perl, r, julia, elixir, yaml, toml, makefile, dockerfile
}

#[test]
fn test_comment_syntax_double_dash() {
    let (pre, suf) = comment_syntax("haskell");
    assert_eq!(pre, "-- ");
    assert_eq!(suf, "");
    // Also test: lua, sql, ocaml, ocaml_interface
}

#[test]
fn test_comment_syntax_html_block() {
    let (pre, suf) = comment_syntax("html");
    assert_eq!(pre, "<!-- ");
    assert_eq!(suf, " -->");
    // Also test: xml, svelte, markdown
}

#[test]
fn test_comment_syntax_css_block() {
    let (pre, suf) = comment_syntax("css");
    assert_eq!(pre, "/* ");
    assert_eq!(suf, " */");
    // Also test: scss
}

#[test]
fn test_comment_syntax_matlab() {
    let (pre, suf) = comment_syntax("matlab");
    assert_eq!(pre, "% ");
    assert_eq!(suf, "");
}

#[test]
fn test_comment_syntax_unknown_default() {
    let (pre, suf) = comment_syntax("brainfuck");
    assert_eq!(pre, "// ");
    assert_eq!(suf, "");
}
```

- [ ] **Step 2: Implement `comment_syntax()`**

Per the spec's language-to-comment mapping.

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/context_quality/annotation.rs
git commit -m "feat: implement language-aware comment syntax mapping"
```

### Task 14: Implement `annotate_file()`

**Files:**
- Modify: `src/context_quality/annotation.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_annotate_selected_file_full_detail() {
    let ctx = AnnotationContext {
        path: "src/api/handler.rs".to_string(),
        language: "rust".to_string(),
        score: 0.85,
        role: FileRole::Selected,
        parent: None,
        signals: vec![
            SignalResult { name: "symbol_match", score: 0.9, detail: "matched: handle_request".to_string() },
            SignalResult { name: "path_similarity", score: 0.7, detail: "score=0.70".to_string() },
        ],
        detail_level: DetailLevel::Full,
        tokens: 4200,
    };
    let annotation = annotate_file(&ctx);
    assert!(annotation.contains("// [cxpak] src/api/handler.rs"));
    assert!(annotation.contains("score: 0.8500"));
    assert!(annotation.contains("role: selected"));
    assert!(annotation.contains("symbol_match=0.90"));
    assert!(annotation.contains("detail_level: full (4200 tokens)"));
    assert!(!annotation.contains("parent:")); // no parent for selected
}

#[test]
fn test_annotate_dependency_shows_parent() {
    let ctx = AnnotationContext {
        path: "src/auth/middleware.rs".to_string(),
        language: "rust".to_string(),
        score: 0.72,
        role: FileRole::Dependency,
        parent: Some("src/api/routes.rs".to_string()),
        signals: vec![],
        detail_level: DetailLevel::Full,
        tokens: 1500,
    };
    let annotation = annotate_file(&ctx);
    assert!(annotation.contains("role: dependency"));
    assert!(annotation.contains("parent: src/api/routes.rs"));
}

#[test]
fn test_annotate_signal_line_omitted_at_level2() {
    let ctx = AnnotationContext {
        path: "dep.rs".to_string(),
        language: "rust".to_string(),
        score: 0.3,
        role: FileRole::Dependency,
        parent: None,
        signals: vec![
            SignalResult { name: "tf", score: 0.3, detail: "".to_string() },
        ],
        detail_level: DetailLevel::Documented,
        tokens: 200,
    };
    let annotation = annotate_file(&ctx);
    assert!(!annotation.contains("signals:")); // omitted at Level 2+
    assert!(annotation.contains("detail_level: documented"));
}

#[test]
fn test_annotate_html_block_comments() {
    let ctx = AnnotationContext {
        path: "index.html".to_string(),
        language: "html".to_string(),
        score: 0.5,
        role: FileRole::Selected,
        parent: None,
        signals: vec![],
        detail_level: DetailLevel::Signature,
        tokens: 30,
    };
    let annotation = annotate_file(&ctx);
    assert!(annotation.contains("<!-- [cxpak]"));
    assert!(annotation.contains("-->"));
}

#[test]
fn test_annotate_empty_signals() {
    let ctx = AnnotationContext {
        path: "test.rs".to_string(),
        language: "rust".to_string(),
        score: 0.5,
        role: FileRole::Selected,
        parent: None,
        signals: vec![],
        detail_level: DetailLevel::Full,
        tokens: 100,
    };
    let annotation = annotate_file(&ctx);
    // Signal line should say "signals: (none)" or be omitted
    assert!(annotation.contains("[cxpak]"));
}
```

- [ ] **Step 2: Implement `annotate_file()`**

```rust
use crate::context_quality::degradation::{DetailLevel, FileRole};
use crate::relevance::SignalResult;

pub struct AnnotationContext {
    pub path: String,
    pub language: String,
    pub score: f64,
    pub role: FileRole,
    pub parent: Option<String>,
    pub signals: Vec<SignalResult>,
    pub detail_level: DetailLevel,
    pub tokens: usize,
}

pub fn annotate_file(ctx: &AnnotationContext) -> String {
    let (pre, suf) = comment_syntax(&ctx.language);
    let mut lines = Vec::new();

    // Line 1: path
    lines.push(format!("{pre}[cxpak] {}{suf}", ctx.path));

    // Line 2: score + role + parent
    let role_str = match ctx.role {
        FileRole::Selected => "selected",
        FileRole::Dependency => "dependency",
    };
    let mut line2 = format!("{pre}score: {:.4} | role: {role_str}", ctx.score);
    if let Some(ref parent) = ctx.parent {
        line2.push_str(&format!(" | parent: {parent}"));
    }
    line2.push_str(suf);
    lines.push(line2);

    // Line 3: signals (only at Level 0 and Level 1)
    if ctx.detail_level <= DetailLevel::Trimmed && !ctx.signals.is_empty() {
        let signal_parts: Vec<String> = ctx.signals.iter()
            .map(|s| format!("{}={:.2}", s.name, s.score))
            .collect();
        lines.push(format!("{pre}signals: {}{suf}", signal_parts.join(", ")));
    }

    // Line 4: detail level + tokens
    let level_name = match ctx.detail_level {
        DetailLevel::Full => "full",
        DetailLevel::Trimmed => "trimmed",
        DetailLevel::Documented => "documented",
        DetailLevel::Signature => "signature",
        DetailLevel::Stub => "stub",
    };
    lines.push(format!("{pre}detail_level: {level_name} ({} tokens){suf}", ctx.tokens));

    lines.join("\n")
}
```

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/context_quality/annotation.rs
git commit -m "feat: implement annotate_file with language-aware comment syntax"
```

---

## Stream 4: Integration + MCP Wiring

### Task 15: Wire degradation + annotations into `cxpak_pack_context`

**Files:**
- Modify: `src/commands/serve.rs`

- [ ] **Step 1: Update `cxpak_pack_context` handler**

Replace the simple token-counting loop with:
1. Build file list with roles (Selected vs Dependency)
2. Call `allocate_with_degradation()` to get degraded symbols per file
3. For each file, generate annotation via `annotate_file()`
4. Compute annotation token cost via `TokenCounter` and **subtract from the file's budget allocation** before rendering symbols (spec requirement: annotation tokens count toward budget)
5. Prepend annotation to rendered content
6. Include `detail_level` in the response JSON per file

- [ ] **Step 2: Write MCP round-trip tests**

```rust
#[test]
fn test_mcp_pack_context_includes_detail_level() { ... }

#[test]
fn test_mcp_pack_context_annotations_present() { ... }

#[test]
fn test_mcp_pack_context_degradation_under_tight_budget() { ... }
```

- [ ] **Step 3: Run all tests**

Run: `cargo test --verbose`

- [ ] **Step 4: Commit**

```bash
git add src/commands/serve.rs
git commit -m "feat: wire degradation and annotations into cxpak_pack_context"
```

### Task 16: Wire expansion into `cxpak_context_for_task`

**Files:**
- Modify: `src/commands/serve.rs`

- [ ] **Step 1: Update `cxpak_context_for_task` handler**

Before scoring:
1. Get `index.domains`
2. Call `expand_query(task, &index.domains)` to get expanded tokens
3. Create scorer with `MultiSignalScorer::new().with_expansion(expanded_tokens)`
4. Score as before

- [ ] **Step 2: Write MCP round-trip tests**

```rust
#[test]
fn test_mcp_context_for_task_uses_expansion() {
    // Create index with file containing "authentication" symbol
    // Query with "auth"
    // Verify the file is found (wouldn't be without expansion)
}
```

- [ ] **Step 3: Run all tests**

- [ ] **Step 4: Commit**

```bash
git add src/commands/serve.rs
git commit -m "feat: wire query expansion into cxpak_context_for_task"
```

### Task 17: Full pipeline integration tests

**Files:**
- Add tests to `src/commands/serve.rs` or `tests/`

- [ ] **Step 1: Write end-to-end tests**

```rust
#[test]
fn test_full_pipeline_expansion_scoring_packing_degradation_annotation() {
    // 1. Build index with multiple files (some with "authentication" in symbols)
    // 2. Call context_for_task with "auth" → verify expanded matches
    // 3. Call pack_context with selected files at tight budget
    // 4. Verify degradation levels vary
    // 5. Verify annotations present with correct syntax
    // 6. Verify budget respected
}

#[test]
fn test_pipeline_large_symbol_split_and_degraded() {
    // 1. Build index with one file containing a huge function (>4000 tokens)
    // 2. Call pack_context at tight budget
    // 3. Verify symbol was split into chunks
    // 4. Verify some chunks degraded more than others
}
```

- [ ] **Step 2: Run all tests**

Run: `cargo test --verbose`

- [ ] **Step 3: Commit**

```bash
git add src/commands/serve.rs tests/
git commit -m "test: add full pipeline integration tests for context quality"
```

---

## Stream 5: Documentation + Version

### Task 18: Update documentation

**Files:**
- Modify: `README.md`
- Modify: `.claude/CLAUDE.md`

- [ ] **Step 1: Update README.md**

- Document context quality features
- Describe degradation levels
- Document query expansion (core + domain synonyms)
- Document context annotations

- [ ] **Step 2: Update CLAUDE.md**

- Add `context_quality` module to architecture notes
- Document new budget behavior (progressive degradation)
- Note query expansion and domain detection

- [ ] **Step 3: Commit**

```bash
git add README.md .claude/CLAUDE.md
git commit -m "docs: document context quality features for v0.11.0"
```

### Task 19: Version bump

**Files:**
- Modify: `Cargo.toml`
- Modify: `plugin/.claude-plugin/plugin.json`
- Modify: `.claude-plugin/marketplace.json`
- Modify: `plugin/lib/ensure-cxpak`

- [ ] **Step 1: Bump version to 0.11.0 in all four files**

- [ ] **Step 2: Commit**

```bash
git add Cargo.toml plugin/.claude-plugin/plugin.json .claude-plugin/marketplace.json plugin/lib/ensure-cxpak
git commit -m "chore: bump version to 0.11.0"
```

### Task 20: Pre-Release QA + CI Validation

**This task MUST pass before tagging and pushing.**

- [ ] **Step 1: Run full test suite locally**

Run: `cargo test --verbose`
Expected: ALL tests pass. Zero failures.

- [ ] **Step 2: Run clippy (strict)**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: Zero warnings.

- [ ] **Step 3: Run formatter check**

Run: `cargo fmt -- --check`
Expected: No formatting changes needed.

- [ ] **Step 4: Run coverage check**

Run: `cargo tarpaulin --verbose --all-features --workspace --timeout 120 --out json`
Expected: ≥90% overall coverage. 100% on `src/context_quality/`. ≥95% on modified files in `relevance/`, `budget/`, `index/`, `commands/serve.rs`.

- [ ] **Step 5: Manual QA — degradation**

Run cxpak against a real repo with a tight budget and verify:
```bash
cargo run -- overview --tokens 5k .
```
- Verify output contains omission markers
- Verify symbols are at mixed detail levels (Full for important, Stub for low-priority)

- [ ] **Step 6: Manual QA — query expansion**

Start MCP server, call `context_for_task`:
```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_context_for_task","arguments":{"task":"auth"}}}' | cargo run --features daemon -- serve --mcp .
```
- Verify results include files matching expanded synonyms (e.g., files with "authentication", "login", "session")

- [ ] **Step 7: Manual QA — annotations**

Call `pack_context` via MCP, verify output:
- Each packed file has a `[cxpak]` annotation header
- Comment syntax matches the file's language
- `detail_level` and token count are present
- Dependencies show `parent:` field

- [ ] **Step 8: Manual QA — chunk splitting**

Find or create a file with a function >4000 tokens. Run `pack_context` and verify:
- Symbol is split into `[1/N]`, `[2/N]`, etc.
- Each chunk contains the parent signature
- Chunks degrade independently

- [ ] **Step 9: Simulate CI jobs locally**

Run the exact commands from `.github/workflows/`:
```bash
# Build
cargo build --verbose
# Test
cargo test --verbose
# Clippy
cargo clippy --all-targets -- -D warnings
# Format
cargo fmt -- --check
# Coverage (same as CI)
cargo tarpaulin --verbose --all-features --workspace --timeout 120 --fail-under 90
```
Expected: All pass. If any fail, fix before proceeding.

- [ ] **Step 10: Tag and push (only after all above pass)**

```bash
git tag v0.11.0
git push origin main --tags
```

---

## Task Summary

| Stream | Tasks | Dependencies |
|---|---|---|
| 1. Core Types + Degradation | Tasks 1-6 | Sequential |
| 2. Query Expansion | Tasks 7-12 | Task 7 (tokenize public) unblocks 8-10; Tasks 11-12 depend on 8-10 |
| 3. Annotations | Tasks 13-14 | Task 1 (module scaffold), Task 2 (FileRole type) |
| 4. Integration | Tasks 15-17 | All of Streams 1-3 |
| 5. Docs + Version | Tasks 18-19 | All prior |
| 6. Pre-Release QA | Task 20 | Task 19 (version bump) |

**Parallelizable:** Streams 2 and 3 can run in parallel after Task 2 completes (Task 7 just needs signals.rs, Tasks 13-14 just need the context_quality scaffold). Tasks 8-10 (expansion) and Tasks 13-14 (annotations) are independent.

**Critical path:** Tasks 1-6 → Task 7 → (Tasks 8-12 ∥ Tasks 13-14) → Tasks 15-17 → Tasks 18-19 → Task 20

**Total: 20 tasks, ~135 new tests, 100% branch coverage on `context_quality/`, 95%+ on modified existing modules. Task 20 is the release gate — no tag/push until all QA steps pass.**
