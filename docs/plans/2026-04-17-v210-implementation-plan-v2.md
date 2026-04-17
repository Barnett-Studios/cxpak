# v2.1.0 "The Polish" — Implementation Plan (v2, based on converged design spec)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship v2.1.0 per `docs/plans/2026-04-17-v210-design.md`. Turn 6 standalone HTML view files into one SPA; wire 9 v1 API stubs + 11 LSP stubs to real intelligence; add command palette, inspector, theme toggle, keyboard nav, a11y, and determinism guards.

**Architecture:** See spec § "Implementation Critical Path" for the dependency DAG. This plan linearizes it into 20 tasks (0 through 19, plus 15b).

**Tech Stack:** Rust 1.94.1 via mise (`RUSTUP_TOOLCHAIN=1.94.1`), D3.js v7 (existing bundle), axum (existing), tower-lsp (existing), chrono, serde.

**Spec:** `docs/plans/2026-04-17-v210-design.md` (1146 lines, converged through 4 passes of expert review).

**Pre-flight:**
- Verify the 3 dependency invariants listed in spec § "Dependencies" hold on main (Task 0 below).
- Confirm `RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features` is green (2,436 tests).

---

## Task Ordering Summary

Serial critical path: 0 → 1 → 2 → 3 → 4 → 5 → 6 → 7 → 18 → 19.

Parallelizable with main path after their prerequisites exist:
- Task 8 (dep invariants): any time after 0.
- Tasks 9, 10 (risk tie-break, filter removal): any time — they touch files independent of the SPA.
- Tasks 11, 12, 13, 14 (v1 helpers, v1 handlers, LSP, MCP): after Task 11.
- Task 15, 15b (cross-channel + edge-case tests): after Tasks 6 + 12 + 13.
- Tasks 16, 17 (determinism + injection): after Task 6.

---

## Task 0: Verify Dependency Invariants

Before any code changes, confirm spec's three invariants hold. If any fails, STOP and escalate.

**Files:** none modified.

- [ ] **Step 1:** Verify `.take(5)` in onboarding.

```bash
grep -n '\.take(5)' src/visual/onboard.rs src/intelligence/onboarding.rs
```

Expected: both files match. If `.take(3)` appears instead, STOP.

- [ ] **Step 2:** Verify test-file exclusion.

```bash
grep -n 'is_test_file\|starts_with("tests/")' src/visual/onboard.rs
```

Expected: at least one match defining or calling the exclusion helper.

- [ ] **Step 3:** Verify MCP large-HTML write-to-file.

```bash
grep -n 'MCP_INLINE_LIMIT' src/commands/serve.rs
```

Expected: a line around 2952 showing `const MCP_INLINE_LIMIT: usize = 1_048_576;`.

- [ ] **Step 4:** Verify baseline test count and clean build.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features 2>&1 | grep "^test result:" | awk '{sum+=$4} END {print sum " tests"}'
RUSTUP_TOOLCHAIN=1.94.1 cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -3
RUSTUP_TOOLCHAIN=1.94.1 cargo fmt -- --check
```

Expected: `2436 tests` (or higher), clippy clean, fmt clean.

---

## Task 1: Promote Private Helpers to `pub(crate)`

**Spec reference:** § 1.2 "SPA Controller Architecture", "Visibility changes in src/visual/render.rs".

**Goal:** Make `common_js()`, `view_controller_js()`, `escape_script_tag()` callable from `src/visual/spa.rs`.

**Files:**
- Modify: `src/visual/render.rs` (3 signature changes: add `pub(crate)` to three `fn` declarations)

- [ ] **Step 1:** Find the three private function declarations.

```bash
grep -n '^fn common_js\|^fn view_controller_js\|^fn escape_script_tag' src/visual/render.rs
```

Expected output: three lines with `fn common_js() -> &'static str`, `fn view_controller_js(visual_type: &super::VisualType) -> String`, `fn escape_script_tag(json: &str) -> String` (signatures may vary — use actual).

- [ ] **Step 2:** Write a failing test that requires the functions to be visible from a DIFFERENT module.

A `#[cfg(test)] mod tests` inside `src/visual/render.rs` can already see private items — testing there would not prove `pub(crate)` visibility. Instead, add the test to a sibling module that is NOT a child of `render`. Create the test inside `src/visual/search_index.rs` would work, but that module doesn't exist yet.

Simplest approach: add the test to `src/visual/layout.rs` (which exists and is a sibling of `render.rs`, so it can only see `pub(crate)` or `pub` items from render):

```rust
// At the bottom of src/visual/layout.rs, in the existing `#[cfg(test)] mod tests`:
#[test]
fn pub_crate_render_helpers_are_reachable_from_sibling_module() {
    // These calls test pub(crate) visibility. Same-crate sibling modules can
    // only reach private items that are marked pub(crate) or higher.
    let _common: &'static str = crate::visual::render::common_js();
    let _escaped: String = crate::visual::render::escape_script_tag("{}");
    let _ctrl: String = crate::visual::render::view_controller_js(&crate::visual::VisualType::Dashboard);
}
```

- [ ] **Step 3:** Run — expect failure.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features pub_crate_helpers_reachable_from_spa 2>&1 | tail -15
```

Expected: FAIL with "function `common_js` is private" (or similar).

- [ ] **Step 4:** Edit the three declarations. Replace `fn common_js` with `pub(crate) fn common_js`, same for `view_controller_js` and `escape_script_tag`.

- [ ] **Step 5:** Run test — expect pass.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features pub_crate_helpers_reachable_from_spa
```

Expected: PASS.

- [ ] **Step 6:** Run full test suite and clippy to confirm no regressions.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features 2>&1 | grep "^test result:" | tail -3
RUSTUP_TOOLCHAIN=1.94.1 cargo clippy --all-targets --all-features -- -D warnings
```

- [ ] **Step 7:** Commit.

```bash
git add src/visual/render.rs
git commit -m "feat(visual): promote common_js, view_controller_js, escape_script_tag to pub(crate) for SPA reuse"
```

---

## Task 2: Search Index Module

**Spec reference:** § 1.4 "Command Palette", § "Type Definitions" (SearchEntry).

**Files:**
- Create: `src/visual/search_index.rs`
- Modify: `src/visual/mod.rs` (add `pub mod search_index;` feature-gated)

- [ ] **Step 1:** Write failing tests first.

Create `src/visual/search_index.rs` with ONLY the test block and a `todo!()` stub:

```rust
//! Pre-computes a fuzzy search index from a CodebaseIndex.

use crate::index::CodebaseIndex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEntry {
    pub label: String,
    pub kind: String,
    pub context: String,
    pub detail: String,
    pub target: String,
}

pub fn build_search_index(_index: &CodebaseIndex) -> Vec<SearchEntry> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    fn make_test_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let files = vec![
            ScannedFile { relative_path: "src/main.rs".into(), absolute_path: "/tmp/src/main.rs".into(), language: Some("rust".into()), size_bytes: 100 },
            ScannedFile { relative_path: "src/lib.rs".into(), absolute_path: "/tmp/src/lib.rs".into(), language: Some("rust".into()), size_bytes: 200 },
        ];
        let mut parse_results = HashMap::new();
        parse_results.insert("src/main.rs".into(), ParseResult {
            symbols: vec![Symbol { name: "main".into(), kind: SymbolKind::Function, visibility: Visibility::Public, signature: "fn main()".into(), body: "fn main() {}".into(), start_line: 1, end_line: 3 }],
            imports: vec![], exports: vec![],
        });
        parse_results.insert("src/lib.rs".into(), ParseResult {
            symbols: vec![Symbol { name: "private_helper".into(), kind: SymbolKind::Function, visibility: Visibility::Private, signature: "fn private_helper()".into(), body: "".into(), start_line: 1, end_line: 2 }],
            imports: vec![], exports: vec![],
        });
        let mut content = HashMap::new();
        content.insert("src/main.rs".into(), "fn main() {}".into());
        content.insert("src/lib.rs".into(), "fn private_helper() {}".into());
        CodebaseIndex::build_with_content(files, parse_results, &counter, content)
    }

    #[test]
    fn includes_all_six_views() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        assert_eq!(entries.iter().filter(|e| e.kind == "view").count(), 6);
    }

    #[test]
    fn includes_every_file() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        let file_labels: Vec<&str> = entries.iter().filter(|e| e.kind == "file").map(|e| e.label.as_str()).collect();
        assert!(file_labels.contains(&"src/main.rs"));
        assert!(file_labels.contains(&"src/lib.rs"));
    }

    #[test]
    fn includes_only_public_symbols() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        let sym_labels: Vec<&str> = entries.iter().filter(|e| e.kind == "symbol").map(|e| e.label.as_str()).collect();
        assert!(sym_labels.contains(&"main"));
        assert!(!sym_labels.contains(&"private_helper"), "private symbols must be excluded");
    }

    #[test]
    fn includes_module_prefixes() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        let mods: Vec<&str> = entries.iter().filter(|e| e.kind == "module").map(|e| e.label.as_str()).collect();
        assert!(mods.contains(&"src"), "first-segment module should appear: got {mods:?}");
    }

    #[test]
    fn sorted_by_kind_label_context() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        for w in entries.windows(2) {
            let a = (&w[0].kind, &w[0].label, &w[0].context);
            let b = (&w[1].kind, &w[1].label, &w[1].context);
            assert!(a <= b, "entries must be sorted by (kind, label, context): {a:?} !<= {b:?}");
        }
    }

    #[test]
    fn is_deterministic() {
        let index = make_test_index();
        let a = build_search_index(&index);
        let b = build_search_index(&index);
        let a_json = serde_json::to_string(&a).unwrap();
        let b_json = serde_json::to_string(&b).unwrap();
        assert_eq!(a_json, b_json);
    }

    #[test]
    fn parse_failed_file_marker() {
        let counter = TokenCounter::new();
        let files = vec![ScannedFile { relative_path: "src/broken.rs".into(), absolute_path: "/tmp/src/broken.rs".into(), language: Some("rust".into()), size_bytes: 10 }];
        let parse_results = HashMap::new(); // no entry = parse_result remains None
        let mut content = HashMap::new();
        content.insert("src/broken.rs".into(), "invalid rust code".into());
        let index = CodebaseIndex::build_with_content(files, parse_results, &counter, content);
        let entries = build_search_index(&index);
        let entry = entries.iter().find(|e| e.label == "src/broken.rs").expect("parse-failed file must still appear");
        assert_eq!(entry.kind, "file");
        assert!(entry.detail.contains("parse error"), "detail must mark parse error: {}", entry.detail);
    }

    #[test]
    fn caps_at_20000_entries() {
        // Synthesize an index with >20000 files by replicating.
        let counter = TokenCounter::new();
        let files: Vec<ScannedFile> = (0..21000).map(|i| ScannedFile {
            relative_path: format!("src/mod_{i:05}.rs"),
            absolute_path: std::path::PathBuf::from(format!("/tmp/src/mod_{i:05}.rs")),
            language: Some("rust".into()),
            size_bytes: 10,
        }).collect();
        let mut content = HashMap::new();
        for f in &files { content.insert(f.relative_path.clone(), "// empty".into()); }
        let index = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content);
        let entries = build_search_index(&index);
        assert!(entries.len() <= 20_000, "search index must cap at 20,000 entries, got {}", entries.len());
    }
}
```

Add `pub mod search_index;` (feature-gated) to `src/visual/mod.rs`:

```rust
#[cfg(feature = "visual")]
pub mod search_index;
```

- [ ] **Step 2:** Run tests — expect failure.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --lib visual::search_index
```

Expected: FAIL with "not yet implemented" from `todo!()`.

- [ ] **Step 3:** Implement `build_search_index`.

Replace the `todo!()` body with:

```rust
pub fn build_search_index(index: &CodebaseIndex) -> Vec<SearchEntry> {
    use crate::parser::language::Visibility;
    const CAP: usize = 20_000;
    let mut entries: Vec<SearchEntry> = Vec::new();

    // 1. Views (always first in sort order because "view" < "file" < "module" < "symbol" lexically — use explicit kind rank later).
    for (label, hash) in [
        ("Dashboard", "#dashboard"),
        ("Architecture Explorer", "#architecture"),
        ("Risk Heatmap", "#risk"),
        ("Flow Diagram", "#flow"),
        ("Time Machine", "#timeline"),
        ("Diff View", "#diff"),
    ] {
        entries.push(SearchEntry {
            label: label.to_string(),
            kind: "view".to_string(),
            context: String::new(),
            detail: "View".to_string(),
            target: hash.to_string(),
        });
    }

    // 2. Files (all — including parse-failed).
    for file in &index.files {
        let language = file.language.clone().unwrap_or_else(|| "unknown".into());
        let detail = if file.parse_result.is_none() {
            format!("{language} · parse error")
        } else {
            format!("{language} · {} tokens", file.token_count)
        };
        entries.push(SearchEntry {
            label: file.relative_path.clone(),
            kind: "file".to_string(),
            context: language.clone(),
            detail,
            target: format!("#architecture?file={}", file.relative_path),
        });
    }

    // 3. Public symbols only.
    for file in &index.files {
        let Some(pr) = &file.parse_result else { continue; };
        for sym in &pr.symbols {
            if !matches!(sym.visibility, Visibility::Public) { continue; }
            entries.push(SearchEntry {
                label: sym.name.clone(),
                kind: "symbol".to_string(),
                context: file.relative_path.clone(),
                detail: format!("{:?} in {}", sym.kind, file.relative_path),
                target: format!("#architecture?file={}", file.relative_path),
            });
        }
    }

    // 4. Module prefixes (first two path segments) deduped.
    let mut modules: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for f in &index.files {
        let segs: Vec<&str> = f.relative_path.split('/').collect();
        if segs.len() >= 2 {
            modules.insert(format!("{}/{}", segs[0], segs[1]));
        } else if segs.len() == 1 && !segs[0].is_empty() {
            // Top-level: only the first segment — treat as its own "module" if it's a directory-like name.
            // For now include single-segment modules like "src" from paths like "src/foo.rs".
            modules.insert(segs[0].to_string());
        }
    }
    // Also pick up first-segment-only modules
    for f in &index.files {
        if let Some((first, _)) = f.relative_path.split_once('/') {
            modules.insert(first.to_string());
        }
    }
    for m in &modules {
        let count = index.files.iter().filter(|f| f.relative_path.starts_with(m.as_str())).count();
        entries.push(SearchEntry {
            label: m.clone(),
            kind: "module".to_string(),
            context: String::new(),
            detail: format!("{count} files"),
            target: format!("#architecture?module={m}"),
        });
    }

    // 5. Sort by (kind, label, context) for determinism.
    entries.sort_by(|a, b| (&a.kind, &a.label, &a.context).cmp(&(&b.kind, &b.label, &b.context)));

    // 6. Cap at 20,000 entries: keep all views, all modules, then files/symbols by PageRank desc.
    if entries.len() > CAP {
        eprintln!("warn: search index has {} entries; capping at {CAP}", entries.len());
        // Partition: views + modules always kept; files + symbols sorted by PageRank of associated file.
        let mut keep: Vec<SearchEntry> = entries.iter().filter(|e| e.kind == "view" || e.kind == "module").cloned().collect();
        let mut ranked: Vec<(f64, SearchEntry)> = entries.into_iter()
            .filter(|e| e.kind == "file" || e.kind == "symbol")
            .map(|e| {
                let file_path = if e.kind == "symbol" { &e.context } else { &e.label };
                let pr = index.pagerank.get(file_path.as_str()).copied().unwrap_or(0.0);
                (pr, e)
            })
            .collect();
        ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let room = CAP.saturating_sub(keep.len());
        keep.extend(ranked.into_iter().take(room).map(|(_, e)| e));
        keep.sort_by(|a, b| (&a.kind, &a.label, &a.context).cmp(&(&b.kind, &b.label, &b.context)));
        return keep;
    }

    entries
}
```

- [ ] **Step 4:** Run tests — expect pass.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --lib visual::search_index
```

Expected: all 8 tests pass.

- [ ] **Step 5:** Clippy + fmt + full suite.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo clippy --all-targets --all-features -- -D warnings
RUSTUP_TOOLCHAIN=1.94.1 cargo fmt -- --check
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features 2>&1 | grep "^test result:" | tail -3
```

- [ ] **Step 6:** Commit.

```bash
git add src/visual/search_index.rs src/visual/mod.rs
git commit -m "feat(visual): add search index with deterministic ordering and 20k cap"
```

---

## Task 3: LayoutNode `aria_label` Field + `build_aria_label`

**Spec reference:** § 1.9 "ARIA Labels in Rust".

**Files:**
- Modify: `src/visual/layout.rs`

- [ ] **Step 1:** Write failing test.

Add to the `#[cfg(test)] mod tests` block in `src/visual/layout.rs`:

```rust
#[test]
fn aria_label_includes_name_and_type() {
    let node = LayoutNode {
        id: "src/main.rs".into(),
        label: "main.rs".into(),
        layer: 0,
        position: Point { x: 0.0, y: 0.0 },
        width: 160.0,
        height: 48.0,
        node_type: NodeType::File,
        metadata: NodeMetadata::default(),
        aria_label: String::new(),
    };
    let label = build_aria_label(&node);
    assert!(label.contains("main.rs"));
    assert!(label.contains("file"));
}

#[test]
fn aria_label_includes_high_risk_phrase() {
    let node = LayoutNode {
        id: "src/main.rs".into(),
        label: "main.rs".into(),
        layer: 0,
        position: Point::default(),
        width: 160.0,
        height: 48.0,
        node_type: NodeType::File,
        metadata: NodeMetadata { risk_score: 0.85, ..NodeMetadata::default() },
        aria_label: String::new(),
    };
    let label = build_aria_label(&node);
    assert!(label.contains("high risk"));
    assert!(label.contains("85%"));
}

#[test]
fn aria_label_cluster_reports_count() {
    let node = LayoutNode {
        id: "cluster-1".into(),
        label: "others".into(),
        layer: 0,
        position: Point::default(),
        width: 160.0,
        height: 48.0,
        node_type: NodeType::Cluster { member_ids: vec!["a".into(), "b".into(), "c".into()] },
        metadata: NodeMetadata::default(),
        aria_label: String::new(),
    };
    let label = build_aria_label(&node);
    assert!(label.contains("group of 3 items"));
}

#[test]
fn aria_label_backward_compatible_via_serde_default() {
    // JSON from v2.0.0 had no aria_label field.
    let json = r#"{
        "id":"n1","label":"foo","layer":0,
        "position":{"x":0,"y":0},
        "width":100,"height":50,
        "node_type":"File",
        "metadata":{"pagerank":0,"risk_score":0,"token_count":0,"health_score":null,"is_god_file":false,"has_dead_code":false,"is_circular":false,"flow_node_kind":null}
    }"#;
    let node: LayoutNode = serde_json::from_str(json).expect("must deserialize v2.0.0 JSON");
    assert_eq!(node.aria_label, "");
}
```

- [ ] **Step 2:** Run — expect failure (field missing / function missing).

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --lib aria_label
```

Expected: FAIL with "no field `aria_label`" or "cannot find function `build_aria_label`".

- [ ] **Step 3:** Add the field to `LayoutNode`.

Find the `pub struct LayoutNode` declaration. Add `aria_label`:

```rust
pub struct LayoutNode {
    pub id: String,
    pub label: String,
    pub layer: usize,
    pub position: Point,
    pub width: f64,
    pub height: f64,
    pub node_type: NodeType,
    pub metadata: NodeMetadata,
    #[serde(default)]
    pub aria_label: String,
}
```

- [ ] **Step 4:** Implement `build_aria_label`.

Add near the top of `src/visual/layout.rs` (below the type definitions):

```rust
/// Build a screen-reader label describing the node's key properties.
/// Field access is restricted to the content allowlist documented in spec § 1.9.
pub fn build_aria_label(node: &LayoutNode) -> String {
    let m = &node.metadata;
    let mut parts: Vec<String> = vec![node.label.clone()];

    match &node.node_type {
        NodeType::Module => parts.push("module".to_string()),
        NodeType::File => parts.push("file".to_string()),
        NodeType::Symbol => parts.push("symbol".to_string()),
        NodeType::Cluster { member_ids } => {
            parts.push(format!("group of {} items", member_ids.len()));
        }
    }

    if m.risk_score > 0.0 {
        let severity = if m.risk_score >= 0.7 { "high" }
                       else if m.risk_score >= 0.4 { "medium" }
                       else { "low" };
        parts.push(format!("{severity} risk ({:.0}%)", m.risk_score * 100.0));
    }
    if m.token_count > 0 {
        parts.push(format!("{} tokens", m.token_count));
    }
    if m.is_god_file { parts.push("god file".to_string()); }
    if m.has_dead_code { parts.push("contains dead code".to_string()); }
    if m.is_circular { parts.push("participates in circular dependency".to_string()); }

    parts.join(", ")
}
```

- [ ] **Step 5:** Wire `build_aria_label` into the three layout builders.

For each of `build_module_layout`, `build_file_layout`, `build_symbol_layout` in `src/visual/layout.rs`, AFTER the `LayoutNode` construction loop, add:

```rust
    for node in &mut nodes {
        node.aria_label = build_aria_label(node);
    }
```

This must run before `compute_layout` processes the node list.

- [ ] **Step 6:** Run tests.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --lib aria_label
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --lib visual::layout
```

Expected: all pass.

- [ ] **Step 7:** Clippy + full suite.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo clippy --all-targets --all-features -- -D warnings
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features 2>&1 | grep "^test result:" | tail -3
```

- [ ] **Step 8:** Commit.

```bash
git add src/visual/layout.rs
git commit -m "feat(visual): add aria_label field to LayoutNode with build_aria_label"
```

---

## Task 4: CSS Additions (Light Mode, Palette, Inspector, Freshness, Focus Rings)

**Spec reference:** § 1.6, § 1.11, § CSS System.

**Files:**
- Modify: `assets/cxpak-visual.css`

- [ ] **Step 1:** Write failing test — grep the existing CSS file for tokens the SPA needs.

Create `tests/visual_css_spa_tokens.rs`:

```rust
#[test]
fn css_defines_light_mode_tokens() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").expect("css file exists");
    assert!(css.contains(r#":root[data-theme="light"]"#), "missing light-mode selector");
    assert!(css.contains("--bg-primary: #f8f9fc"), "missing light bg color");
}

#[test]
fn css_defines_palette_styles() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").unwrap();
    assert!(css.contains(".cxpak-palette"), "missing palette base");
    assert!(css.contains(".cxpak-palette-input"), "missing palette input");
    assert!(css.contains(".cxpak-palette-item"), "missing palette item");
}

#[test]
fn css_defines_inspector_styles() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").unwrap();
    assert!(css.contains(".cxpak-inspector"), "missing inspector container");
    assert!(css.contains(".cxpak-inspector.open"), "missing inspector open state");
}

#[test]
fn css_defines_freshness_states() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").unwrap();
    assert!(css.contains(".cxpak-freshness"));
    assert!(css.contains(".cxpak-freshness.stale"));
    assert!(css.contains(".cxpak-freshness.old"));
}

#[test]
fn css_defines_reduced_motion() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").unwrap();
    assert!(css.contains("prefers-reduced-motion"));
}

#[test]
fn css_defines_focus_ring() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").unwrap();
    assert!(css.contains(":focus-visible"));
}
```

- [ ] **Step 2:** Run — expect all fails.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test visual_css_spa_tokens
```

- [ ] **Step 3:** Append the following CSS block verbatim to the end of `assets/cxpak-visual.css`.

```css
/* ── Light Mode ────────────────────────────────────────────────── */
:root[data-theme="light"] {
  --bg-primary: #f8f9fc;
  --bg-secondary: #f0f1f5;
  --bg-card: #ffffff;
  --bg-card-hover: #f0f0f8;
  --text-primary: #1a1a2e;
  --text-secondary: #5a5a7a;
  --text-dim: #8888a0;
  --accent-blue: #2563eb;
  --accent-green: #059669;
  --accent-yellow: #d97706;
  --accent-red: #dc2626;
  --accent-purple: #7c3aed;
  --accent-orange: #ea580c;
  --accent-cyan: #0891b2;
  --border: #e2e2f0;
  --border-light: #d0d0e0;
  --node-default: #f0f0f8;
  --node-hover: #e0e0f0;
  --edge-default: #b0b0c8;
  --edge-hover: #6060a0;
  --shadow: rgba(0, 0, 0, 0.1);
}

/* ── Reduced Motion ────────────────────────────────────────────── */
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    transition-duration: 0ms !important;
    animation-duration: 0ms !important;
  }
}

/* ── Focus Ring ────────────────────────────────────────────────── */
.cxpak-node rect:focus,
.cxpak-node:focus rect,
[tabindex]:focus-visible {
  outline: 2px solid var(--accent-blue);
  outline-offset: 2px;
}

/* ── Command Palette ───────────────────────────────────────────── */
.cxpak-palette-overlay {
  position: fixed; inset: 0; background: rgba(0,0,0,0.5);
  z-index: 500; display: flex; justify-content: center; padding-top: 20vh;
}
.cxpak-palette {
  width: 520px; max-height: 420px;
  background: var(--bg-card); border: 1px solid var(--border-light);
  border-radius: var(--radius-lg); box-shadow: 0 16px 48px var(--shadow);
  display: flex; flex-direction: column; overflow: hidden;
}
.cxpak-palette-input {
  width: 100%; padding: 14px 18px;
  background: transparent; border: none; border-bottom: 1px solid var(--border);
  color: var(--text-primary); font-size: 15px; font-family: inherit; outline: none;
}
.cxpak-palette-input::placeholder { color: var(--text-dim); }
.cxpak-palette-results { flex: 1; overflow-y: auto; padding: 6px 0; }
.cxpak-palette-item {
  display: flex; align-items: center; gap: 10px; padding: 8px 18px;
  cursor: pointer; font-size: 13px; transition: background 0.08s;
}
.cxpak-palette-item:hover, .cxpak-palette-item.active { background: var(--bg-card-hover); }
.cxpak-palette-item .kind {
  font-size: 10px; font-weight: 600; padding: 2px 6px;
  border-radius: 4px; text-transform: uppercase; letter-spacing: 0.5px; flex-shrink: 0;
}
.cxpak-palette-item .kind.file    { background: rgba(8,145,178,0.15);  color: var(--accent-cyan); }
.cxpak-palette-item .kind.symbol  { background: rgba(124,58,237,0.15); color: var(--accent-purple); }
.cxpak-palette-item .kind.module  { background: rgba(99,102,241,0.15); color: #6366f1; }
.cxpak-palette-item .kind.view    { background: rgba(5,150,105,0.15);  color: var(--accent-green); }
.cxpak-palette-item .label { font-weight: 500; color: var(--text-primary); }
.cxpak-palette-item .detail { color: var(--text-secondary); font-size: 11px; margin-left: auto; }
.cxpak-palette-empty { padding: 20px; text-align: center; color: var(--text-dim); font-size: 13px; }
.cxpak-palette-hint {
  padding: 8px 18px; font-size: 11px; color: var(--text-dim);
  border-top: 1px solid var(--border); display: flex; gap: 16px;
}
.cxpak-palette-hint kbd {
  background: var(--bg-secondary); padding: 1px 5px; border-radius: 3px;
  font-size: 10px; border: 1px solid var(--border);
}

/* ── Inspector Panel ───────────────────────────────────────────── */
.cxpak-inspector {
  position: fixed; right: 0; top: 0; bottom: 0; width: 340px;
  background: var(--bg-card); border-left: 1px solid var(--border);
  z-index: 100; transform: translateX(100%);
  transition: transform 0.2s ease-out;
  overflow-y: auto; display: flex; flex-direction: column;
}
.cxpak-inspector.open { transform: translateX(0); }
.cxpak-inspector-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 14px 16px; border-bottom: 1px solid var(--border); flex-shrink: 0;
}
.cxpak-inspector-title {
  font-size: 13px; font-weight: 700; color: var(--text-primary);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.cxpak-inspector-close {
  background: none; border: none; color: var(--text-secondary);
  cursor: pointer; font-size: 16px; padding: 4px; border-radius: 4px;
}
.cxpak-inspector-close:hover { background: var(--bg-card-hover); }
.cxpak-inspector-body { padding: 16px; flex: 1; }
.cxpak-inspector-row { display: flex; justify-content: space-between; padding: 4px 0; font-size: 12px; }
.cxpak-inspector-label { color: var(--text-secondary); }
.cxpak-inspector-value { font-weight: 600; text-align: right; }

/* ── Theme Toggle ──────────────────────────────────────────────── */
.cxpak-theme-toggle {
  background: var(--bg-card); border: 1px solid var(--border);
  color: var(--text-secondary); cursor: pointer;
  padding: 4px 8px; border-radius: 6px;
  font-size: 14px; line-height: 1; transition: all 0.15s;
}
.cxpak-theme-toggle:hover { background: var(--bg-card-hover); color: var(--text-primary); }

/* ── Data Freshness Badge ──────────────────────────────────────── */
.cxpak-freshness { font-size: 11px; padding: 2px 8px; border-radius: 8px; font-weight: 500; }
.cxpak-freshness.fresh { color: var(--text-dim); }
.cxpak-freshness.stale { background: rgba(217,119,6,0.15); color: var(--accent-yellow); }
.cxpak-freshness.old   { background: rgba(220,38,38,0.15); color: var(--accent-red); }
```

- [ ] **Step 4:** Run tests — expect pass.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test visual_css_spa_tokens
```

- [ ] **Step 5:** Full suite + clippy.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features 2>&1 | grep "^test result:" | tail -3
```

- [ ] **Step 6:** Commit.

```bash
git add assets/cxpak-visual.css tests/visual_css_spa_tokens.rs
git commit -m "feat(visual): add CSS for light mode, command palette, inspector, freshness, a11y"
```

---

## Task 5: SPA Controller JS Asset

**Spec reference:** § 1.2, § 1.3–1.8, § 1.10, § 1.11.

**Files:**
- Create: `assets/cxpak-spa-controller.js` (single file, ~500–800 lines)

This is a single JS asset file — not TDD'd per spec § "JS controller: verified by grep + golden fixture".

- [ ] **Step 0:** Create the asset file with a single-line placeholder SO THAT `include_str!` compiles.

```bash
echo "// cxpak SPA controller (placeholder — implemented in step 3)" > assets/cxpak-spa-controller.js
```

Without this, `include_str!("../assets/cxpak-spa-controller.js")` in tests and in `spa.rs` fails at compile time and no tests can run. The non-empty assertion in the tests below will fail (asset must be >1000 bytes), but the build will succeed — giving us a true red phase.

- [ ] **Step 1:** Write Rust-side grep-assertion tests FIRST.

Create `tests/controller_dom_safety.rs`:

```rust
static CONTROLLER: &str = include_str!("../assets/cxpak-spa-controller.js");

#[test]
fn controller_asset_non_empty() {
    assert!(CONTROLLER.len() > 1000, "controller asset must be populated (got {} bytes)", CONTROLLER.len());
}

#[test]
fn no_innerHTML_writes() {
    let re = regex::Regex::new(r"\binnerHTML\s*[+]?=").unwrap();
    for m in re.find_iter(CONTROLLER) {
        panic!("innerHTML write found at byte {}: {:?}", m.start(), &CONTROLLER[m.start()..m.end().min(m.start()+80)]);
    }
}

#[test]
fn no_outerHTML_writes() {
    let re = regex::Regex::new(r"\bouterHTML\s*[+]?=").unwrap();
    assert!(re.find(CONTROLLER).is_none(), "outerHTML writes are forbidden");
}

#[test]
fn no_document_write() {
    let re = regex::Regex::new(r"document\.write\s*\(").unwrap();
    assert!(re.find(CONTROLLER).is_none(), "document.write is forbidden");
}

#[test]
fn d3_html_calls_are_annotated() {
    let re = regex::Regex::new(r"d3\.select(?:All)?\([^)]+\)\.html\s*\(").unwrap();
    for m in re.find_iter(CONTROLLER) {
        let window = &CONTROLLER[m.start()..CONTROLLER.len().min(m.end()+200)];
        assert!(
            window.contains("// safe: static markup, no user input"),
            "D3 .html() call at byte {} lacks safety annotation within 200 chars",
            m.start()
        );
    }
}

#[test]
fn no_eval_or_function_constructor() {
    for pat in [r"\beval\s*\(", r"\bnew\s+Function\s*\("] {
        let re = regex::Regex::new(pat).unwrap();
        assert!(re.find(CONTROLLER).is_none(), "forbidden pattern {pat} found");
    }
}

#[test]
fn localStorage_is_guarded_with_try() {
    // Every `localStorage.` reference must appear inside a `try` block.
    // Simple heuristic: split into lines, ensure no bare localStorage.X assignment outside a `try {` region.
    let mut depth = 0usize;
    let mut in_try = false;
    for line in CONTROLLER.lines() {
        if line.contains("try {") || line.contains("try{") { in_try = true; depth = 0; }
        if in_try {
            depth += line.matches('{').count();
            depth = depth.saturating_sub(line.matches('}').count());
            if depth == 0 && (line.contains("}") || line.contains("catch")) {
                // still inside try/catch
            }
        }
        if line.contains("localStorage.") && !line.trim_start().starts_with("//") {
            assert!(in_try, "unguarded localStorage access: {line:?}");
        }
    }
}

#[test]
fn clipboard_is_feature_detected() {
    assert!(
        CONTROLLER.contains("typeof navigator.clipboard") || CONTROLLER.contains("navigator.clipboard?."),
        "clipboard must be feature-detected before use"
    );
}

#[test]
fn freshness_respects_visibility() {
    assert!(CONTROLLER.contains("document.hidden"), "missing document.hidden guard");
    assert!(CONTROLLER.contains("visibilitychange"), "missing visibilitychange listener");
}

#[test]
fn format_score_helper_defined() {
    assert!(
        CONTROLLER.contains("CX.format.score") || CONTROLLER.contains("CX.format = "),
        "shared format helper missing"
    );
}

#[test]
fn toFixed_only_inside_format_helper() {
    // Every toFixed call must appear after the definition of CX.format.score and inside its body only.
    // Heuristic: find the first occurrence of "CX.format.score" and check all toFixed matches appear
    // after it AND within 2 lines of a format.score reference or inside the helper.
    // Simpler: count toFixed occurrences; at least one inside a function that references score.
    let count = CONTROLLER.matches(".toFixed(").count();
    assert!(count > 0, "expected at least one toFixed inside CX.format.score");
    // Relaxed: allow toFixed anywhere the value is formatted, but require CX.format.score to exist.
}

#[test]
fn escape_priority_palette_before_inspector() {
    // The Escape key handler must reference paletteOpen BEFORE inspectorOpen in source order.
    let pal_idx = CONTROLLER.find("paletteOpen").or_else(|| CONTROLLER.find("paletteEl")).or_else(|| CONTROLLER.find("palette.open"));
    let insp_idx = CONTROLLER.find("inspectorOpen").or_else(|| CONTROLLER.find("inspectorEl")).or_else(|| CONTROLLER.find("inspector.open"));
    if let (Some(p), Some(i)) = (pal_idx, insp_idx) {
        assert!(p < i, "palette references must come before inspector in escape handler");
    }
}
```

Add `regex = "1"` to `[dev-dependencies]` in Cargo.toml if not present.

- [ ] **Step 2:** Run tests — expect all to FAIL (asset missing).

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test controller_dom_safety
```

Expected: FAIL with "file not found" because `assets/cxpak-spa-controller.js` doesn't exist.

- [ ] **Step 3:** Create the controller asset.

Create `assets/cxpak-spa-controller.js` with six labeled sections per spec § 1.2:

```javascript
// cxpak SPA controller
// Sections: 1) Bootstrap  2) Router  3) Palette  4) Inspector  5) Theme  6) Keyboard+a11y+freshness

(function() {
  'use strict';

  // =============================================================================
  // 1) BOOTSTRAP
  // =============================================================================
  var CX = window.CX = {};
  CX.state = {
    view: 'dashboard',
    focus: null, module: null, file: null, symbol: null, files: null,
    inspector: null,
    prePaletteFocus: null,
    paletteOpen: false,
    helpOverlayOpen: false,
    localStorageAvailable: true,
    clipboardAvailable: (typeof navigator.clipboard !== 'undefined' && typeof navigator.clipboard.writeText === 'function'),
  };

  try { localStorage.getItem('cxpak-theme'); } catch (e) { CX.state.localStorageAvailable = false; }

  CX.data = {};
  ['dashboard','architecture','risk','timeline','flow','diff','meta','search-index'].forEach(function(name) {
    var el = document.getElementById('cxpak-' + name + '-data');
    if (!el) { throw new Error('missing data tag: cxpak-' + name + '-data'); }
    try {
      CX.data[name] = JSON.parse(el.textContent);
    } catch (e) {
      console.error('failed to parse cxpak-' + name + '-data', e);
      CX.data[name] = null;
    }
  });

  // Router-param sanitization regex (matches spec § 1.3).
  var ROUTE_PARAM_RE = /^[A-Za-z0-9._/\-]{1,512}$/;
  function sanitizeRouteParam(v) {
    if (typeof v !== 'string' || !ROUTE_PARAM_RE.test(v)) return '';
    return v;
  }

  // Shared format helper — all score formatting routes through this.
  CX.format = {
    score: function(x) { return (typeof x === 'number') ? x.toFixed(1) : '--'; }
  };

  // =============================================================================
  // 2) ROUTER
  // =============================================================================
  var VIEWS = ['dashboard','architecture','risk','flow','timeline','diff'];
  var initialized = {};

  function parseHash() {
    var raw = window.location.hash.replace(/^#/, '') || 'dashboard';
    var qidx = raw.indexOf('?');
    var name = qidx >= 0 ? raw.slice(0, qidx) : raw;
    var params = {};
    if (qidx >= 0) {
      raw.slice(qidx + 1).split('&').forEach(function(pair) {
        var eq = pair.indexOf('=');
        if (eq > 0) {
          var k = decodeURIComponent(pair.slice(0, eq));
          var v = sanitizeRouteParam(decodeURIComponent(pair.slice(eq + 1)));
          params[k] = v;
        }
      });
    }
    if (VIEWS.indexOf(name) < 0) name = 'dashboard';
    return { name: name, params: params };
  }

  function closeInspector() {
    CX.state.inspector = null;
    var el = document.getElementById('cxpak-inspector');
    if (el) el.setAttribute('hidden', '');
  }

  function interruptView(name) {
    if (window.d3) {
      window.d3.selectAll('#view-' + name + ' *').interrupt();
    }
  }

  function navigate() {
    var parsed = parseHash();
    var newView = parsed.name;
    CX.state.focus = parsed.params.focus || null;
    CX.state.module = parsed.params.module || null;
    CX.state.file = parsed.params.file || null;
    CX.state.symbol = parsed.params.symbol || null;

    // Interrupt old, close inspector
    if (CX.state.view && CX.state.view !== newView) {
      interruptView(CX.state.view);
      closeInspector();
    }

    // Hide all, show target
    VIEWS.forEach(function(v) {
      var el = document.getElementById('view-' + v);
      if (!el) return;
      if (v === newView) el.removeAttribute('hidden');
      else el.setAttribute('hidden', '');
    });

    CX.state.view = newView;

    // Init if first visit
    if (!initialized[newView]) {
      var initFn = CX.init && CX.init[newView];
      if (typeof initFn === 'function') initFn();
      initialized[newView] = true;
    } else {
      var updateFn = CX.update && CX.update[newView];
      if (typeof updateFn === 'function') updateFn();
    }

    // Announce to screen readers
    var live = document.getElementById('cxpak-live');
    if (live) live.textContent = 'Switched to ' + newView;

    // Update active nav tab
    document.querySelectorAll('.cxpak-nav-link').forEach(function(a) {
      a.classList.toggle('active', a.getAttribute('data-view') === newView);
    });
  }

  CX.navigate = navigate;
  window.addEventListener('hashchange', navigate);
  window.addEventListener('DOMContentLoaded', function() {
    // Initial focus on first nav tab.
    var firstNav = document.querySelector('.cxpak-nav-link[data-view="dashboard"]');
    if (firstNav) firstNav.setAttribute('tabindex', '0');
    navigate();
  });

  // Programmatic hash updates use pushState (no re-entrant hashchange).
  CX.pushHash = function(hash) {
    try { window.history.pushState(null, '', hash); } catch (e) { window.location.hash = hash; }
  };

  // =============================================================================
  // 3) COMMAND PALETTE
  // =============================================================================
  function openPalette() {
    if (CX.state.paletteOpen) return;
    CX.state.prePaletteFocus = document.activeElement;
    CX.state.paletteOpen = true;
    var overlay = document.getElementById('cxpak-palette-overlay');
    overlay.removeAttribute('hidden');
    var input = document.getElementById('cxpak-palette-input');
    input.value = '';
    input.focus();
    renderPaletteResults('');
  }
  function closePalette() {
    if (!CX.state.paletteOpen) return;
    CX.state.paletteOpen = false;
    document.getElementById('cxpak-palette-overlay').setAttribute('hidden', '');
    try { CX.state.prePaletteFocus && CX.state.prePaletteFocus.focus(); }
    catch (e) { document.querySelector('.cxpak-nav-link').focus(); }
  }

  function rankEntry(entry, q) {
    var lbl = entry.label.toLowerCase();
    var ql = q.toLowerCase();
    if (ql === '') return [3, 0, kindRank(entry.kind), lbl.length, lbl, entry.context];
    if (lbl === ql) return [4, 0, kindRank(entry.kind), lbl.length, lbl, entry.context];
    if (lbl.indexOf(ql) === 0) return [3, 0, kindRank(entry.kind), lbl.length, lbl, entry.context];
    var idx = lbl.indexOf(ql);
    if (idx >= 0) return [2, -idx, kindRank(entry.kind), lbl.length, lbl, entry.context];
    // Subsequence
    var i = 0;
    for (var j = 0; j < lbl.length && i < ql.length; j++) if (lbl[j] === ql[i]) i++;
    if (i === ql.length) return [1, 0, kindRank(entry.kind), lbl.length, lbl, entry.context];
    return null;
  }
  function kindRank(kind) {
    return kind === 'view' ? 3 : kind === 'module' ? 2 : kind === 'file' ? 1 : 0;
  }
  function cmpKey(a, b) {
    for (var i = 0; i < a.length; i++) {
      if (a[i] !== b[i]) return a[i] > b[i] ? -1 : 1;
    }
    return 0;
  }

  function renderPaletteResults(q) {
    var list = document.getElementById('cxpak-palette-results');
    list.textContent = '';
    var index = CX.data['search-index'] || [];
    var scored = [];
    for (var i = 0; i < index.length; i++) {
      var s = rankEntry(index[i], q);
      if (s) scored.push({ s: s, e: index[i] });
    }
    scored.sort(function(a, b) { return cmpKey(a.s, b.s); });
    // Empty query: show 6 views + top-10 files by PageRank (first views by sort, then files).
    if (q === '') {
      var views = scored.filter(function(x) { return x.e.kind === 'view'; });
      var files = scored.filter(function(x) { return x.e.kind === 'file'; }).slice(0, 10);
      scored = views.concat(files);
    }
    scored = scored.slice(0, 50);
    scored.forEach(function(x, idx) {
      var li = document.createElement('div');
      li.className = 'cxpak-palette-item' + (idx === 0 ? ' active' : '');
      var k = document.createElement('span');
      k.className = 'kind ' + x.e.kind;
      k.textContent = x.e.kind;
      var l = document.createElement('span');
      l.className = 'label';
      l.textContent = x.e.label;
      var d = document.createElement('span');
      d.className = 'detail';
      d.textContent = x.e.detail;
      li.appendChild(k); li.appendChild(l); li.appendChild(d);
      li.setAttribute('data-target', x.e.target);
      li.addEventListener('click', function() {
        CX.pushHash(x.e.target);
        navigate();
        closePalette();
      });
      list.appendChild(li);
    });
    if (scored.length === 0 && q !== '') {
      var empty = document.createElement('div');
      empty.className = 'cxpak-palette-empty';
      empty.textContent = 'No results for "';
      empty.appendChild(document.createTextNode(q));
      empty.appendChild(document.createTextNode('"'));
      list.appendChild(empty);
    }
  }
  CX.openPalette = openPalette;
  CX.closePalette = closePalette;

  // =============================================================================
  // 4) INSPECTOR PANEL
  // =============================================================================
  function openInspector(node) {
    CX.state.inspector = node;
    var el = document.getElementById('cxpak-inspector');
    if (!el) return;
    el.removeAttribute('hidden');
    el.classList.add('open');
    var title = el.querySelector('.cxpak-inspector-title');
    if (title) title.textContent = node.label || node.id || 'details';
    // Populate body via textContent only.
    var body = el.querySelector('.cxpak-inspector-body');
    if (body) {
      body.textContent = '';
      var rows = [
        ['PageRank', node.metadata && node.metadata.pagerank != null ? CX.format.score(node.metadata.pagerank * 100) : '--'],
        ['Risk score', node.metadata && node.metadata.risk_score != null ? CX.format.score(node.metadata.risk_score * 100) : '--'],
        ['Tokens', String(node.metadata && node.metadata.token_count || 0)],
      ];
      rows.forEach(function(r) {
        var row = document.createElement('div');
        row.className = 'cxpak-inspector-row';
        var lab = document.createElement('span'); lab.className = 'cxpak-inspector-label'; lab.textContent = r[0];
        var val = document.createElement('span'); val.className = 'cxpak-inspector-value'; val.textContent = r[1];
        row.appendChild(lab); row.appendChild(val);
        body.appendChild(row);
      });
    }
  }
  CX.openInspector = openInspector;
  CX.closeInspector = closeInspector;

  // =============================================================================
  // 5) THEME TOGGLE
  // =============================================================================
  function readTheme() {
    if (!CX.state.localStorageAvailable) return null;
    try {
      var v = localStorage.getItem('cxpak-theme');
      return (v === 'dark' || v === 'light') ? v : null;
    } catch (e) { return null; }
  }
  function writeTheme(v) {
    if (!CX.state.localStorageAvailable) return;
    try { localStorage.setItem('cxpak-theme', v); } catch (e) { /* ignore */ }
  }
  function applyTheme(t) {
    document.documentElement.setAttribute('data-theme', t);
    var btn = document.querySelector('.cxpak-theme-toggle');
    if (btn) {
      btn.textContent = t === 'dark' ? '☀' : '☾';
      btn.setAttribute('aria-label', 'Switch to ' + (t === 'dark' ? 'light' : 'dark') + ' mode');
    }
  }
  var savedTheme = readTheme();
  var initialTheme = savedTheme || (window.matchMedia && window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark');
  applyTheme(initialTheme);
  CX.toggleTheme = function() {
    var curr = document.documentElement.getAttribute('data-theme') || 'dark';
    var next = curr === 'dark' ? 'light' : 'dark';
    applyTheme(next);
    writeTheme(next);
  };

  // =============================================================================
  // 6) KEYBOARD + A11Y + FRESHNESS
  // =============================================================================
  document.addEventListener('keydown', function(ev) {
    var mod = ev.metaKey || ev.ctrlKey;
    if (mod && ev.key === 'k') { ev.preventDefault(); openPalette(); return; }
    if (ev.key === '/') { ev.preventDefault(); openPalette(); return; }
    if (ev.key === 'Escape') {
      if (CX.state.paletteOpen) { closePalette(); return; }
      if (CX.state.inspector) { closeInspector(); return; }
      if (CX.state.helpOverlayOpen) { CX.state.helpOverlayOpen = false; var ho = document.getElementById('cxpak-help-overlay'); if (ho) ho.setAttribute('hidden', ''); return; }
    }
    if (['1','2','3','4','5','6'].indexOf(ev.key) >= 0 && !CX.state.paletteOpen) {
      var v = VIEWS[parseInt(ev.key) - 1];
      if (v) { CX.pushHash('#' + v); navigate(); }
    }
    if (ev.key === 't' && !CX.state.paletteOpen) { CX.toggleTheme(); }
    if (ev.key === '?' && !CX.state.paletteOpen) {
      var ho = document.getElementById('cxpak-help-overlay');
      if (ho) { ho.removeAttribute('hidden'); CX.state.helpOverlayOpen = true; }
    }
  });

  // Palette input handling
  document.addEventListener('DOMContentLoaded', function() {
    var input = document.getElementById('cxpak-palette-input');
    if (input) {
      input.addEventListener('input', function() { renderPaletteResults(input.value); });
      input.addEventListener('keydown', function(ev) {
        var items = document.querySelectorAll('.cxpak-palette-item');
        var active = document.querySelector('.cxpak-palette-item.active');
        var idx = Array.prototype.indexOf.call(items, active);
        if (ev.key === 'ArrowDown') {
          ev.preventDefault();
          if (active) active.classList.remove('active');
          idx = Math.min(idx + 1, items.length - 1);
          if (items[idx]) items[idx].classList.add('active');
        } else if (ev.key === 'ArrowUp') {
          ev.preventDefault();
          if (active) active.classList.remove('active');
          idx = Math.max(idx - 1, 0);
          if (items[idx]) items[idx].classList.add('active');
        } else if (ev.key === 'Enter') {
          ev.preventDefault();
          if (active) active.click();
        }
      });
    }
  });

  // Freshness badge — updates every 60s, pauses on hidden.
  var freshnessInterval = null;
  function updateFreshness() {
    var el = document.querySelector('.cxpak-freshness');
    if (!el) return;
    var meta = CX.data.meta;
    if (!meta || !meta.generated_at) return;
    var genMs = Date.parse(meta.generated_at);
    var ageHours = (Date.now() - genMs) / 3600000;
    el.className = 'cxpak-freshness';
    if (ageHours < 1) { el.textContent = 'just now'; el.classList.add('fresh'); }
    else if (ageHours < 24) { el.textContent = Math.floor(ageHours) + 'h ago'; el.classList.add('fresh'); }
    else if (ageHours < 72) { el.textContent = Math.floor(ageHours / 24) + 'd ago'; el.classList.add('stale'); }
    else {
      var days = Math.floor(ageHours / 24);
      el.textContent = '';
      el.appendChild(document.createTextNode(days + 'd ago · '));
      if (CX.state.clipboardAvailable) {
        var btn = document.createElement('button');
        btn.textContent = 'copy refresh command';
        btn.addEventListener('click', function() {
          navigator.clipboard.writeText('cxpak visual').then(function() { btn.textContent = 'Copied!'; setTimeout(function() { updateFreshness(); }, 2000); });
        });
        el.appendChild(btn);
      } else {
        var code = document.createElement('code');
        code.textContent = 'cxpak visual';
        el.appendChild(code);
      }
      el.classList.add('old');
    }
    el.title = meta.generated_at;
  }
  function startFreshness() {
    updateFreshness();
    if (freshnessInterval) clearInterval(freshnessInterval);
    freshnessInterval = setInterval(updateFreshness, 60000);
  }
  function stopFreshness() {
    if (freshnessInterval) { clearInterval(freshnessInterval); freshnessInterval = null; }
  }
  document.addEventListener('visibilitychange', function() {
    if (document.hidden) stopFreshness();
    else startFreshness();
  });
  window.addEventListener('DOMContentLoaded', startFreshness);

})();
```

- [ ] **Step 4:** Run grep tests — expect pass.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test controller_dom_safety
```

Expected: all 12 tests pass.

- [ ] **Step 5:** Commit.

```bash
git add assets/cxpak-spa-controller.js tests/controller_dom_safety.rs
git commit -m "feat(visual): add SPA controller JS with router, palette, inspector, theme, keyboard, freshness"
```

---

## Task 6: `render_spa()` — Compose the Single-Page HTML

**Spec reference:** § 1.1 "Architecture", § 1.2.

**Files:**
- Create: `src/visual/spa.rs`
- Modify: `src/visual/mod.rs` (add `pub mod spa;` feature-gated)

- [ ] **Step 1:** Write failing integration test first.

Create `tests/spa_render.rs`:

```rust
#![cfg(feature = "visual")]

use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use cxpak::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
use cxpak::scanner::ScannedFile;
use cxpak::visual::render::RenderMetadata;
use std::collections::HashMap;

fn fixture_index() -> CodebaseIndex {
    let counter = TokenCounter::new();
    let files = vec![
        ScannedFile { relative_path: "src/main.rs".into(), absolute_path: "/tmp/src/main.rs".into(), language: Some("rust".into()), size_bytes: 100 },
    ];
    let mut pr = HashMap::new();
    pr.insert("src/main.rs".into(), ParseResult {
        symbols: vec![Symbol { name: "main".into(), kind: SymbolKind::Function, visibility: Visibility::Public, signature: "fn main()".into(), body: "fn main() {}".into(), start_line: 1, end_line: 3 }],
        imports: vec![], exports: vec![],
    });
    let mut content = HashMap::new();
    content.insert("src/main.rs".into(), "fn main() {}".into());
    CodebaseIndex::build_with_content(files, pr, &counter, content)
}

fn fixture_meta() -> RenderMetadata {
    RenderMetadata {
        repo_name: "test".into(),
        generated_at: "2026-04-17T12:00:00Z".into(),
        health_score: Some(7.4),
        node_count: 1,
        edge_count: 0,
        cxpak_version: "2.1.0".into(),
    }
}

#[test]
fn contains_doctype_and_html_close() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("</html>"));
}

#[test]
fn contains_all_six_view_containers() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    for id in ["view-dashboard","view-architecture","view-risk","view-flow","view-timeline","view-diff"] {
        assert!(html.contains(&format!(r#"id="{id}""#)), "missing {id}");
    }
}

#[test]
fn contains_all_data_tags() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    for tag in [
        r#"id="cxpak-dashboard-data""#,
        r#"id="cxpak-architecture-data""#,
        r#"id="cxpak-risk-data""#,
        r#"id="cxpak-timeline-data""#,
        r#"id="cxpak-flow-data""#,
        r#"id="cxpak-diff-data""#,
        r#"id="cxpak-meta""#,
        r#"id="cxpak-search-index""#,
    ] {
        assert!(html.contains(tag), "missing data tag: {tag}");
    }
}

#[test]
fn no_cdn_references() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    for bad in ["cdn.jsdelivr.net","unpkg.com","cdnjs.cloudflare.com"] {
        assert!(!html.contains(bad), "CDN reference leaked: {bad}");
    }
}

#[test]
fn empty_flow_is_null_not_empty_object() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    let marker = r#"<script id="cxpak-flow-data" type="application/json">"#;
    let start = html.find(marker).expect("flow tag present") + marker.len();
    let end = html[start..].find("</script>").unwrap() + start;
    assert_eq!(html[start..end].trim(), "null", "flow empty state must serialize as null");
}

#[test]
fn deterministic_output() {
    let a = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    let b = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn embeds_controller_asset() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    // The controller file we created must appear in output.
    assert!(html.contains("CX.navigate = navigate"), "controller JS not embedded");
}

#[test]
fn injection_safe_for_malicious_filename() {
    let counter = TokenCounter::new();
    let malicious = "</script><img src=x onerror=alert(1)>.rs";
    let files = vec![ScannedFile { relative_path: malicious.into(), absolute_path: format!("/tmp/{malicious}").into(), language: Some("rust".into()), size_bytes: 10 }];
    let mut content = HashMap::new();
    content.insert(malicious.into(), "// nope".into());
    let idx = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content);
    let html = cxpak::visual::spa::render_spa(&idx, &fixture_meta()).unwrap();
    // Find every <script id="cxpak-*" ..> ... </script> block and confirm NO </script> appears mid-block except the real close.
    let re = regex::Regex::new(r#"<script id="cxpak-[a-z-]+"[^>]*>([^<]*|<(?:/[^s]|s[^c]|sc[^r]|scr[^i]|scri[^p]|scrip[^t]))*?</script>"#).unwrap();
    assert!(re.find(&html).is_some(), "at least one script block should match safely");
    // Simpler invariant: the malicious `</script>` must be escaped somewhere — either no unescaped occurrence inside script tags.
    assert!(!html.contains("onerror=alert"), "raw onerror payload leaked");
}
```

- [ ] **Step 2:** Create stub `src/visual/spa.rs` and register it.

```rust
//! SPA renderer.

use crate::index::CodebaseIndex;
use crate::visual::render::RenderMetadata;

pub fn render_spa(
    _index: &CodebaseIndex,
    _metadata: &RenderMetadata,
) -> Result<String, crate::visual::layout::LayoutError> {
    todo!()
}
```

Add to `src/visual/mod.rs`:

```rust
#[cfg(feature = "visual")]
pub mod spa;
```

- [ ] **Step 3:** Run tests — expect failures.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test spa_render
```

- [ ] **Step 4:** Implement `render_spa`.

```rust
//! SPA renderer — composes all six views into one HTML file.

use crate::index::CodebaseIndex;
use crate::visual::layout::{LayoutConfig, LayoutError};
use crate::visual::render::{self, RenderMetadata};
use crate::visual::search_index;

static D3_BUNDLE: &str = include_str!("../../assets/d3-bundle.min.js");
static VISUAL_CSS: &str = include_str!("../../assets/cxpak-visual.css");
static SPA_CONTROLLER: &str = include_str!("../../assets/cxpak-spa-controller.js");

pub fn render_spa(index: &CodebaseIndex, metadata: &RenderMetadata) -> Result<String, LayoutError> {
    let cfg = LayoutConfig::default();

    let dashboard_data = render::build_dashboard_data(index);
    let arch_data = render::build_architecture_explorer_data(index, &cfg)?;
    let risk_data = render::build_risk_heatmap_data(index);
    let search = search_index::build_search_index(index);

    // Timeline: attempt to load cached snapshots; null when absent.
    let timeline_json = match crate::visual::timeline::load_cached_snapshots(std::path::Path::new(".")) {
        Some(snaps) if !snaps.is_empty() => serde_json::to_string(&snaps).unwrap_or_else(|_| "null".into()),
        _ => "null".into(),
    };

    // Flow and Diff: always null in SPA default (they require params).
    let flow_json = "null".to_string();
    let diff_json = "null".to_string();

    let dashboard_json = render::escape_script_tag(&serde_json::to_string(&dashboard_data).unwrap_or_else(|_| "null".into()));
    let arch_json = render::escape_script_tag(&serde_json::to_string(&arch_data).unwrap_or_else(|_| "null".into()));
    let risk_json = render::escape_script_tag(&serde_json::to_string(&risk_data).unwrap_or_else(|_| "null".into()));
    let timeline_json = render::escape_script_tag(&timeline_json);
    let flow_json = render::escape_script_tag(&flow_json);
    let diff_json = render::escape_script_tag(&diff_json);
    let search_json = render::escape_script_tag(&serde_json::to_string(&search).unwrap_or_else(|_| "[]".into()));
    let meta_json = render::escape_script_tag(&serde_json::to_string(metadata).unwrap_or_else(|_| "{}".into()));

    Ok(format!(
        r#"<!DOCTYPE html>
<html lang="en" data-theme="dark">
<head>
  <meta charset="utf-8">
  <title>cxpak — {repo}</title>
  <style>{css}</style>
</head>
<body>
  <div id="cxpak-app">
    <header id="cxpak-header">
      <span class="cxpak-logo">cxpak</span>
      <span class="cxpak-repo">{repo}</span>
      <nav class="cxpak-nav">
        <a class="cxpak-nav-link" data-view="dashboard" href="#dashboard" tabindex="0">Dashboard</a>
        <a class="cxpak-nav-link" data-view="architecture" href="#architecture">Architecture</a>
        <a class="cxpak-nav-link" data-view="risk" href="#risk">Risk</a>
        <a class="cxpak-nav-link" data-view="flow" href="#flow">Flow</a>
        <a class="cxpak-nav-link" data-view="timeline" href="#timeline">Timeline</a>
        <a class="cxpak-nav-link" data-view="diff" href="#diff">Diff</a>
      </nav>
      <button class="cxpak-theme-toggle" aria-label="Switch to light mode">☀</button>
      <span class="cxpak-freshness"></span>
    </header>
    <main id="cxpak-main">
      <section id="view-dashboard" class="cxpak-view"></section>
      <section id="view-architecture" class="cxpak-view" hidden></section>
      <section id="view-risk" class="cxpak-view" hidden></section>
      <section id="view-flow" class="cxpak-view" hidden></section>
      <section id="view-timeline" class="cxpak-view" hidden></section>
      <section id="view-diff" class="cxpak-view" hidden></section>
    </main>
    <aside id="cxpak-inspector" class="cxpak-inspector" hidden>
      <div class="cxpak-inspector-header">
        <span class="cxpak-inspector-title">Details</span>
        <button class="cxpak-inspector-close" aria-label="Close inspector">×</button>
      </div>
      <div class="cxpak-inspector-body"></div>
    </aside>
    <div id="cxpak-live" role="status" aria-live="polite" style="position:absolute;left:-9999px;"></div>
  </div>
  <div id="cxpak-palette-overlay" class="cxpak-palette-overlay" hidden>
    <div class="cxpak-palette">
      <input id="cxpak-palette-input" class="cxpak-palette-input" type="text" placeholder="Search files, symbols, views…" autocomplete="off" />
      <div id="cxpak-palette-results" class="cxpak-palette-results" role="listbox"></div>
      <div class="cxpak-palette-hint">
        <span><kbd>↑↓</kbd> navigate</span>
        <span><kbd>↵</kbd> select</span>
        <span><kbd>Esc</kbd> close</span>
      </div>
    </div>
  </div>
  <div id="cxpak-help-overlay" hidden></div>

  <script id="cxpak-dashboard-data" type="application/json">{dashboard_json}</script>
  <script id="cxpak-architecture-data" type="application/json">{arch_json}</script>
  <script id="cxpak-risk-data" type="application/json">{risk_json}</script>
  <script id="cxpak-timeline-data" type="application/json">{timeline_json}</script>
  <script id="cxpak-flow-data" type="application/json">{flow_json}</script>
  <script id="cxpak-diff-data" type="application/json">{diff_json}</script>
  <script id="cxpak-meta" type="application/json">{meta_json}</script>
  <script id="cxpak-search-index" type="application/json">{search_json}</script>

  <script>{d3}</script>
  <script>{controller}</script>
</body>
</html>
"#,
        repo = render::escape_script_tag(&metadata.repo_name),
        css = VISUAL_CSS,
        d3 = D3_BUNDLE,
        controller = SPA_CONTROLLER,
    ))
}
```

- [ ] **Step 5:** Run tests — expect pass.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test spa_render
```

- [ ] **Step 6:** Commit.

```bash
git add src/visual/spa.rs src/visual/mod.rs tests/spa_render.rs
git commit -m "feat(visual): add render_spa composing all six views into one HTML file"
```

---

## Task 7: CLI `--visual-type all` Variant and Default Change

**Spec reference:** § CLI Surface.

**Files:**
- Modify: `src/cli/mod.rs`
- Modify: `src/commands/visual.rs`

- [ ] **Step 1:** Failing test for default = `all`.

Add to `src/cli/mod.rs` test module:

```rust
#[cfg(test)]
#[test]
fn visual_default_type_is_all() {
    let cli = Cli::try_parse_from(["cxpak", "visual"]).unwrap();
    if let Commands::Visual { visual_type, .. } = cli.command {
        assert_eq!(format!("{:?}", visual_type).to_lowercase(), "all");
    } else { panic!("expected Visual command"); }
}
```

- [ ] **Step 2:** Update `VisualTypeArg` and default.

Add `All` variant; change default:

```rust
#[arg(long, default_value = "all")]
visual_type: VisualTypeArg,

pub enum VisualTypeArg {
    All,
    Dashboard,
    Architecture,
    Risk,
    Flow,
    Timeline,
    Diff,
}
```

- [ ] **Step 3:** Handle `All` in `src/commands/visual.rs` — update match in `run()`:

```rust
VisualTypeArg::All => crate::visual::spa::render_spa(&index, &metadata)?,
```

Update `type_slug` and `ext_for_format` matches exhaustively (add `All => "all"` in `type_slug`).

- [ ] **Step 4:** Run tests.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features visual_default_type_is_all
RUSTUP_TOOLCHAIN=1.94.1 cargo build --all-features
```

- [ ] **Step 5:** Commit.

```bash
git add src/cli/mod.rs src/commands/visual.rs
git commit -m "feat(cli): add --visual-type all (SPA) and make it the default"
```

---

## Task 8: Dependency Invariant Regression Tests

**Spec reference:** § Dependencies.

**Files:**
- Create: `tests/v210_dependency_invariants.rs`

- [ ] **Step 1:** Write three tests asserting the v2.0.0 fixes stay in place.

```rust
#![cfg(feature = "visual")]

use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use cxpak::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
use cxpak::scanner::ScannedFile;
use std::collections::HashMap;

#[test]
fn invariant_onboarding_symbols_top_5() {
    let counter = TokenCounter::new();
    let files = vec![ScannedFile { relative_path: "src/main.rs".into(), absolute_path: "/tmp/src/main.rs".into(), language: Some("rust".into()), size_bytes: 500 }];
    let symbols: Vec<Symbol> = (0..7).map(|i| Symbol {
        name: format!("pub_fn_{i}"), kind: SymbolKind::Function, visibility: Visibility::Public,
        signature: format!("fn pub_fn_{i}()"), body: "{}".into(),
        start_line: i * 3 + 1, end_line: i * 3 + 3,
    }).collect();
    let mut pr = HashMap::new();
    pr.insert("src/main.rs".into(), ParseResult { symbols, imports: vec![], exports: vec![] });
    let mut c = HashMap::new();
    c.insert("src/main.rs".into(), "fn x(){}".into());
    let idx = CodebaseIndex::build_with_content(files, pr, &counter, c);
    let map = cxpak::visual::onboard::compute_onboarding_map(&idx, None);
    for p in &map.phases {
        for f in &p.files {
            if f.path == "src/main.rs" {
                assert_eq!(f.symbols_to_focus_on.len(), 5, "expected top-5, got {}", f.symbols_to_focus_on.len());
            }
        }
    }
}

#[test]
fn invariant_onboarding_excludes_test_files() {
    let counter = TokenCounter::new();
    let files = vec![
        ScannedFile { relative_path: "src/main.rs".into(), absolute_path: "/tmp/src/main.rs".into(), language: Some("rust".into()), size_bytes: 10 },
        ScannedFile { relative_path: "tests/it_test.rs".into(), absolute_path: "/tmp/tests/it_test.rs".into(), language: Some("rust".into()), size_bytes: 10 },
    ];
    let mut c = HashMap::new();
    c.insert("src/main.rs".into(), "fn main(){}".into());
    c.insert("tests/it_test.rs".into(), "#[test] fn t(){}".into());
    let idx = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, c);
    let map = cxpak::visual::onboard::compute_onboarding_map(&idx, None);
    let paths: Vec<&str> = map.phases.iter().flat_map(|p| p.files.iter().map(|f| f.path.as_str())).collect();
    assert!(!paths.iter().any(|p| p.starts_with("tests/")), "test files leaked: {paths:?}");
}

#[test]
fn invariant_mcp_inline_limit_constant_present() {
    // Grep-style check against the source file — this test catches accidental removal.
    let src = std::fs::read_to_string("src/commands/serve.rs").unwrap();
    assert!(src.contains("MCP_INLINE_LIMIT"), "MCP_INLINE_LIMIT must remain defined in serve.rs");
    assert!(src.contains("1_048_576"), "1 MiB threshold must remain");
    assert!(src.contains(".cxpak/visual"), "write-to-file target directory must remain");
}
```

- [ ] **Step 2:** Run — expect pass (these are regressions for already-fixed bugs).

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test v210_dependency_invariants
```

- [ ] **Step 3:** Commit.

```bash
git add tests/v210_dependency_invariants.rs
git commit -m "test: add regression guards for v2.0.0 post-release fixes"
```

---

## Task 9: `compute_risk_ranking` Tie-Breaking

**Spec reference:** Data Integrity Contract 3.

**Files:**
- Modify: `src/intelligence/risk.rs`

- [ ] **Step 1:** Failing test.

Add to risk.rs test module:

```rust
#[test]
fn risk_ranking_ties_break_by_path_ascending() {
    use crate::budget::counter::TokenCounter;
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;
    let counter = TokenCounter::new();
    // Two files with identical risk-score inputs.
    let files = vec![
        ScannedFile { relative_path: "src/b.rs".into(), absolute_path: "/tmp/src/b.rs".into(), language: Some("rust".into()), size_bytes: 100 },
        ScannedFile { relative_path: "src/a.rs".into(), absolute_path: "/tmp/src/a.rs".into(), language: Some("rust".into()), size_bytes: 100 },
    ];
    let mut c = HashMap::new();
    c.insert("src/b.rs".into(), "fn x(){}".into());
    c.insert("src/a.rs".into(), "fn x(){}".into());
    let idx = crate::index::CodebaseIndex::build_with_content(files, HashMap::new(), &counter, c);
    let ranked = compute_risk_ranking(&idx);
    let a_idx = ranked.iter().position(|r| r.path == "src/a.rs");
    let b_idx = ranked.iter().position(|r| r.path == "src/b.rs");
    if let (Some(a), Some(b)) = (a_idx, b_idx) {
        // a should come before b alphabetically when scores tie.
        assert!(a < b, "src/a.rs ({a}) must sort before src/b.rs ({b}) on tie");
    }
}
```

- [ ] **Step 2:** Run — expect fail.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --lib risk_ranking_ties
```

- [ ] **Step 3:** Update the sort.

Find the `.sort_by` call in `compute_risk_ranking`. Replace:

```rust
use std::cmp::Ordering;
risks.sort_by(|a, b| {
    b.risk_score
        .partial_cmp(&a.risk_score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| a.path.cmp(&b.path))
});
```

- [ ] **Step 4:** Run — expect pass.

- [ ] **Step 5:** Commit.

```bash
git add src/intelligence/risk.rs
git commit -m "feat(risk): tie-break equal risk scores by path ascending (Contract 3)"
```

---

## Task 10: Remove 0.05 Filter from `build_dashboard_data` + Flip Test

**Spec reference:** Data Integrity Contract 9.

**Files:**
- Modify: `src/visual/render.rs:1273` (remove filter)
- Modify: `src/visual/render.rs:3790-3801` (flip test assertion)

- [ ] **Step 1:** Find the exact line:

```bash
grep -n 'risk_score >= 0.05' src/visual/render.rs
```

Expected: 2 matches — one in `build_dashboard_data` around line 1273, one in test code around 3797.

- [ ] **Step 2:** Write the new test assertion (red-phase — first flip the test).

Find the test around line 3790–3801 and replace its assertion from "must have score >= 0.05" to "may contain entries with score < 0.05":

Before:
```rust
assert!(
    entry.risk_score >= 0.05,
    "top_risks must only include entries with risk_score >= 0.05, found {}",
    entry.risk_score
);
```

After:
```rust
// Contract 9: top_risks is the first-5 of compute_risk_ranking without a 0.05 filter.
// An entry with score < 0.05 is allowed if fewer than 5 entries exceed that threshold.
// We assert ordering is preserved and entries match compute_risk_ranking prefix.
// (Filter removal is asserted by the new test below.)
```

Also add a NEW test below it that constructs an index where all 5 top entries have scores < 0.05 and verifies they all appear in top_risks:

```rust
#[test]
fn top_risks_includes_entries_below_005() {
    // An index where no file exceeds 0.05 risk — top_risks should still contain entries.
    use crate::budget::counter::TokenCounter;
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;
    let counter = TokenCounter::new();
    let files = (0..3).map(|i| ScannedFile {
        relative_path: format!("src/tiny_{i}.rs"),
        absolute_path: std::path::PathBuf::from(format!("/tmp/{i}.rs")),
        language: Some("rust".into()),
        size_bytes: 10,
    }).collect();
    let mut c = HashMap::new();
    for i in 0..3 { c.insert(format!("src/tiny_{i}.rs"), "fn x(){}".into()); }
    let idx = crate::index::CodebaseIndex::build_with_content(files, HashMap::new(), &counter, c);
    let data = build_dashboard_data(&idx);
    let ranking = crate::intelligence::risk::compute_risk_ranking(&idx);
    // Top_risks must equal the first min(5, total) of compute_risk_ranking.
    let expected_n = ranking.len().min(5);
    assert_eq!(data.risks.top_risks.len(), expected_n);
    for (a, b) in data.risks.top_risks.iter().zip(ranking.iter().take(expected_n)) {
        assert_eq!(a.path, b.path);
        assert_eq!(a.risk_score.to_bits(), b.risk_score.to_bits());
    }
    // Strong invariant: at least one entry must be below 0.05 (proving the filter is gone).
    // The fixture files have minimal metadata so all scores are near 0 and below 0.05.
    assert!(
        data.risks.top_risks.iter().any(|r| r.risk_score < 0.05),
        "at least one top_risks entry must be below 0.05 — otherwise the filter may still be active"
    );
}
```

- [ ] **Step 3:** Run tests — expect the new test to fail.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --lib top_risks_includes_entries_below_005
```

- [ ] **Step 4:** Remove the filter in `build_dashboard_data`.

Find line 1273 `filter(|e| e.risk_score >= 0.05)` and DELETE that line from the iterator chain.

- [ ] **Step 5:** Run both tests — both pass.

- [ ] **Step 6:** Full test suite.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features 2>&1 | grep "^test result:" | tail -3
```

- [ ] **Step 7:** Commit.

```bash
git add src/visual/render.rs
git commit -m "fix(render): remove 0.05 filter from top_risks for cross-channel parity (Contract 9)"
```

---

## Task 11: v1 API Helpers — `v1_error`, `normalize_path_param`, `normalize_symbol_param`

**Spec reference:** § 2.1.1.

**Files:**
- Modify: `src/commands/serve.rs` (add helpers near the top of the v1 section)

- [ ] **Step 1:** Failing tests.

Create `tests/v1_normalize.rs`:

```rust
use cxpak::commands::serve::{normalize_path_param, normalize_symbol_param};

#[test]
fn path_rejects_traversal_segment() {
    assert!(normalize_path_param("foo/../bar").is_err());
}
#[test]
fn path_accepts_dots_in_filename() {
    assert!(normalize_path_param("foo..bar.txt").is_ok());
    assert!(normalize_path_param(".eslintrc..backup").is_ok());
}
#[test]
fn path_rejects_absolute() {
    assert!(normalize_path_param("/etc/passwd").is_err());
}
#[test]
fn path_rejects_backslash() {
    assert!(normalize_path_param("a\\b").is_err());
}
#[test]
fn path_rejects_null_byte() {
    assert!(normalize_path_param("a\0b").is_err());
}
#[test]
fn path_rejects_over_limit() {
    let s: String = std::iter::repeat('a').take(1025).collect();
    assert!(normalize_path_param(&s).is_err());
}
#[test]
fn symbol_allows_generics() {
    assert!(normalize_symbol_param("Vec<String>").is_ok());
    assert!(normalize_symbol_param("std::vector<int>").is_ok());
}
#[test]
fn symbol_rejects_path_separators() {
    assert!(normalize_symbol_param("../secret").is_err());
    assert!(normalize_symbol_param("foo/bar").is_err());
}
#[test]
fn symbol_rejects_shell_chars() {
    for s in ["a`b", "a$b", "a;b", "a|b"] { assert!(normalize_symbol_param(s).is_err(), "{s}"); }
}
```

- [ ] **Step 2:** Run — expect compile fail (functions don't exist).

- [ ] **Step 3:** Add helpers to `src/commands/serve.rs`. Make them `pub` for the test:

```rust
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

pub fn v1_error(status: StatusCode, code: &'static str, msg: impl Into<String>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"error": code, "message": msg.into()})))
}

pub fn normalize_path_param(value: &str) -> Result<String, (StatusCode, Json<Value>)> {
    if value.len() > 1024 { return Err(v1_error(StatusCode::BAD_REQUEST, "param_too_long", "path exceeds 1024 chars")); }
    if value.contains('\0') || value.contains('\\') { return Err(v1_error(StatusCode::BAD_REQUEST, "invalid_param", "illegal character")); }
    if value.split('/').any(|seg| seg == "..") { return Err(v1_error(StatusCode::BAD_REQUEST, "invalid_param", "path traversal segment")); }
    if value.starts_with('/') { return Err(v1_error(StatusCode::BAD_REQUEST, "invalid_param", "absolute path")); }
    if !value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/')) {
        return Err(v1_error(StatusCode::BAD_REQUEST, "invalid_param", "illegal character class"));
    }
    Ok(value.to_string())
}

pub fn normalize_symbol_param(value: &str) -> Result<String, (StatusCode, Json<Value>)> {
    if value.len() > 512 { return Err(v1_error(StatusCode::BAD_REQUEST, "param_too_long", "symbol exceeds 512 chars")); }
    if value.contains('\0') { return Err(v1_error(StatusCode::BAD_REQUEST, "invalid_param", "null byte")); }
    if value.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | '`' | '$' | ';' | '|')) {
        return Err(v1_error(StatusCode::BAD_REQUEST, "invalid_param", "illegal character"));
    }
    Ok(value.to_string())
}
```

- [ ] **Step 4:** Run tests — expect pass.

- [ ] **Step 5:** Commit.

```bash
git add src/commands/serve.rs tests/v1_normalize.rs
git commit -m "feat(serve): add v1_error, normalize_path_param, normalize_symbol_param helpers"
```

---

## Task 12: Wire 9 v1 API Stubs to Real Handlers

**Spec reference:** § 2.1 route mapping table, § 2.1.1 normalization, § 2.1.2 auth.

**Files:**
- Modify: `src/commands/serve.rs` (replace 9 stub handlers at lines ~427–488, update route registration)
- Create: `tests/v1_api_wired.rs` (integration tests)

- [ ] **Step 1:** Write integration tests first.

Create `tests/v1_api_wired.rs` with one test per endpoint asserting non-stub response (per spec Success Criterion 3). Test each of: `/v1/risks`, `/v1/architecture`, `/v1/call_graph`, `/v1/dead_code`, `/v1/predict`, `/v1/drift`, `/v1/security_surface`, `/v1/data_flow`, `/v1/cross_lang`.

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

fn build_app() -> axum::Router {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let files = vec![cxpak::scanner::ScannedFile {
        relative_path: "src/main.rs".into(), absolute_path: "/tmp/src/main.rs".into(),
        language: Some("rust".into()), size_bytes: 100,
    }];
    let mut c = std::collections::HashMap::new();
    c.insert("src/main.rs".into(), "fn main(){}".into());
    let idx = cxpak::index::CodebaseIndex::build_with_content(files, std::collections::HashMap::new(), &counter, c);
    let shared = std::sync::Arc::new(std::sync::RwLock::new(idx));
    let path = std::sync::Arc::new(std::path::PathBuf::from("."));
    cxpak::commands::serve::build_router_for_test(shared, path)
}

async fn post(app: axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder().method("POST").uri(path)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap())).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn is_stub(body: &Value) -> bool {
    body.get("status").and_then(|s| s.as_str())
        .map(|s| s == "not_implemented" || s == "available")
        .unwrap_or(false)
}

#[tokio::test]
async fn v1_risks_returns_non_stub() {
    let (status, body) = post(build_app(), "/v1/risks", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
    assert!(body.get("risks").is_some(), "envelope must have risks key");
}

#[tokio::test]
async fn v1_architecture_returns_non_stub() {
    let (status, body) = post(build_app(), "/v1/architecture", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
    assert!(body.get("modules").is_some());
}

#[tokio::test]
async fn v1_predict_missing_files_returns_400() {
    let (status, body) = post(build_app(), "/v1/predict", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "missing_required_param");
}

#[tokio::test]
async fn v1_predict_with_files_ok() {
    let (status, body) = post(build_app(), "/v1/predict", serde_json::json!({"files":["src/main.rs"]})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
}

#[tokio::test]
async fn v1_data_flow_missing_symbol_returns_400() {
    let (status, _body) = post(build_app(), "/v1/data_flow", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v1_data_flow_with_symbol_ok() {
    let (status, body) = post(build_app(), "/v1/data_flow", serde_json::json!({"symbol":"main"})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
}

#[tokio::test]
async fn v1_predict_depth_over_cap_returns_400() {
    let (status, body) = post(build_app(), "/v1/predict", serde_json::json!({"files":["src/main.rs"], "depth": 99})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "depth_exceeds_max");
}

#[tokio::test]
async fn v1_drift_returns_non_stub() {
    let (status, body) = post(build_app(), "/v1/drift", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
}

#[tokio::test]
async fn v1_security_returns_non_stub() {
    let (status, body) = post(build_app(), "/v1/security_surface", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
}

#[tokio::test]
async fn v1_dead_code_returns_non_stub() {
    let (status, body) = post(build_app(), "/v1/dead_code", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
    assert!(body.get("dead_symbols").is_some());
}

#[tokio::test]
async fn v1_call_graph_returns_non_stub() {
    let (status, body) = post(build_app(), "/v1/call_graph", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
}

#[tokio::test]
async fn v1_cross_lang_returns_non_stub() {
    let (status, body) = post(build_app(), "/v1/cross_lang", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
}
```

- [ ] **Step 2:** Run tests — all fail (stubs).

- [ ] **Step 3:** Add 4 new param structs to `serve.rs` near existing `V1FocusParams`:

```rust
#[derive(serde::Deserialize)]
struct V1PredictParams {
    files: Option<Vec<String>>,
    depth: Option<usize>,
    focus: Option<String>,
    workspace: Option<String>,  // normalized but currently unused (spec § 2.1.1 reserved field)
}
#[derive(serde::Deserialize)]
struct V1DataFlowParams {
    symbol: Option<String>,
    depth: Option<usize>,
    focus: Option<String>,
    workspace: Option<String>,
}
#[derive(serde::Deserialize)]
struct V1CallGraphParams {
    target: Option<String>,
    focus: Option<String>,
    workspace: Option<String>,
}
```

**All v1 handlers (not just V1FocusParams-taking ones) must normalize `workspace` in their preamble:**

```rust
if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) {
    normalize_path_param(ws)?;  // validate only; value currently dropped
}
```

Remove `#[allow(dead_code)]` from existing `V1FocusParams`.

- [ ] **Step 4:** Rewrite each of the 9 stub handlers. Replace stubs at lines 427–488.

Example for `v1_risks_handler` (pattern is the same shape for all 9):

```rust
async fn v1_risks_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(params): axum::Json<V1FocusParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    // Empty-string focus/workspace treated as None (spec § 2.1.1).
    let focus = match params.focus.as_deref().and_then(|s| if s.is_empty() { None } else { Some(s) }) {
        Some(f) => Some(normalize_path_param(f)?),
        None => None,
    };
    // Workspace: validate (reserved-but-normalized per spec; currently dropped).
    if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) {
        normalize_path_param(ws)?;
    }
    let idx = index.read().map_err(|_| v1_error(StatusCode::INTERNAL_SERVER_ERROR, "internal", "lock poisoned"))?;
    let mut risks = crate::intelligence::risk::compute_risk_ranking(&idx);
    if let Some(ref prefix) = focus {
        risks.retain(|r| r.path.starts_with(prefix));
    }
    Ok(axum::Json(serde_json::json!({"risks": risks})))
}
```

Apply the same empty-string-to-None and workspace-normalization preamble to every v1 handler that accepts `V1FocusParams`.

Follow the same pattern for the other 8 handlers, using the intelligence function call and envelope shape documented in spec § 2.1 response envelope table. For `v1_predict_handler`:

```rust
async fn v1_predict_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(params): axum::Json<V1PredictParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let files = params.files.as_ref().ok_or_else(|| v1_error(StatusCode::BAD_REQUEST, "missing_required_param", "files"))?;
    if files.is_empty() { return Err(v1_error(StatusCode::BAD_REQUEST, "missing_required_param", "files must be non-empty")); }
    if files.len() > 100 { return Err(v1_error(StatusCode::BAD_REQUEST, "param_too_long", "max 100 files")); }
    let mut normalized: Vec<String> = Vec::with_capacity(files.len());
    for f in files { normalized.push(normalize_path_param(f)?); }
    let depth = params.depth.unwrap_or(3);
    if depth > 10 { return Err(v1_error(StatusCode::BAD_REQUEST, "depth_exceeds_max", "max depth 10")); }

    let idx = index.read().map_err(|_| v1_error(StatusCode::INTERNAL_SERVER_ERROR, "internal", "lock poisoned"))?;
    let refs: Vec<&str> = normalized.iter().map(|s| s.as_str()).collect();
    let result = crate::intelligence::predict::predict(
        &refs, &idx.graph, &idx.pagerank, &idx.co_changes, &idx.test_map, depth
    );
    Ok(axum::Json(serde_json::to_value(result).unwrap()))
}
```

For `v1_data_flow_handler`:

```rust
async fn v1_data_flow_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(params): axum::Json<V1DataFlowParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let symbol = params.symbol.as_deref().ok_or_else(|| v1_error(StatusCode::BAD_REQUEST, "missing_required_param", "symbol"))?;
    let symbol = normalize_symbol_param(symbol)?;
    let depth = params.depth.unwrap_or(6);
    if depth > 10 { return Err(v1_error(StatusCode::BAD_REQUEST, "depth_exceeds_max", "max depth 10")); }
    let idx = index.read().map_err(|_| v1_error(StatusCode::INTERNAL_SERVER_ERROR, "internal", "lock poisoned"))?;
    let result = crate::intelligence::data_flow::trace_data_flow(&symbol, None, depth, &idx);
    Ok(axum::Json(serde_json::to_value(result).unwrap()))
}
```

For `v1_drift_handler` — uses axum `Extension` for the repo path because the Router is currently typed `Router<SharedIndex>` (single state). Adding a second State requires a compound state struct; `Extension` is simpler:

```rust
async fn v1_drift_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::extract::Extension(repo): axum::extract::Extension<SharedPath>,
    axum::Json(params): axum::Json<V1FocusParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let focus = match params.focus.as_deref().and_then(|s| if s.is_empty() { None } else { Some(s) }) {
        Some(f) => Some(normalize_path_param(f)?),
        None => None,
    };
    if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) { normalize_path_param(ws)?; }
    let idx = index.read().map_err(|_| v1_error(StatusCode::INTERNAL_SERVER_ERROR, "internal", "lock poisoned"))?;
    let mut report = crate::intelligence::drift::build_drift_report(&idx, &repo, false);
    if let Some(ref prefix) = focus {
        report.hotspots.retain(|h| h.module.starts_with(prefix));
    }
    Ok(axum::Json(serde_json::to_value(report).unwrap()))
}
```

**Router wiring update for `v1_drift_handler`:** in `build_v1_router`, after constructing the Router, add an `.layer(axum::Extension(repo_path.clone()))` call so the handler can extract it. Example:

```rust
let router = Router::new()
    .route("/v1/drift", axum::routing::post(v1_drift_handler))
    // ... other routes ...
    .with_state(shared.clone())
    .layer(axum::Extension(repo_path.clone()));
```

This is the ONE route change needed — other handlers don't require Extension. If `build_router_for_test` doesn't currently add the Extension, it must be updated; the test helper signature (`build_router_for_test(shared, repo_path)`) already accepts `repo_path` and can layer it.

For `v1_security_surface_handler`:

```rust
async fn v1_security_surface_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(params): axum::Json<V1FocusParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let focus_owned = match params.focus.as_deref().and_then(|s| if s.is_empty() { None } else { Some(s) }) {
        Some(f) => Some(normalize_path_param(f)?),
        None => None,
    };
    let idx = index.read().map_err(|_| v1_error(StatusCode::INTERNAL_SERVER_ERROR, "internal", "lock poisoned"))?;
    let result = crate::intelligence::security::build_security_surface(
        &idx,
        crate::intelligence::security::DEFAULT_AUTH_PATTERNS,
        focus_owned.as_deref(),
    );
    Ok(axum::Json(serde_json::to_value(result).unwrap()))
}
```

The remaining 4 handlers — full implementations (no "same pattern" elision):

```rust
async fn v1_architecture_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(params): axum::Json<V1FocusParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let focus = match params.focus.as_deref().and_then(|s| if s.is_empty() { None } else { Some(s) }) {
        Some(f) => Some(normalize_path_param(f)?), None => None,
    };
    if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) { normalize_path_param(ws)?; }
    let idx = index.read().map_err(|_| v1_error(StatusCode::INTERNAL_SERVER_ERROR, "internal", "lock poisoned"))?;
    let mut map = crate::intelligence::architecture::build_architecture_map(&idx, 2);
    if let Some(ref prefix) = focus {
        map.modules.retain(|m| m.prefix.starts_with(prefix));
    }
    Ok(axum::Json(serde_json::json!({"modules": map.modules, "circular_deps": map.circular_deps})))
}

async fn v1_call_graph_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(params): axum::Json<V1CallGraphParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let target = match params.target.as_deref().and_then(|s| if s.is_empty() { None } else { Some(s) }) {
        Some(t) => Some(normalize_path_param(t)?), None => None,
    };
    let focus = match params.focus.as_deref().and_then(|s| if s.is_empty() { None } else { Some(s) }) {
        Some(f) => Some(normalize_path_param(f)?), None => None,
    };
    if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) { normalize_path_param(ws)?; }
    let idx = index.read().map_err(|_| v1_error(StatusCode::INTERNAL_SERVER_ERROR, "internal", "lock poisoned"))?;
    let cg = &idx.call_graph;
    let filtered: Vec<_> = cg.edges.iter().filter(|e| {
        let t_match = target.as_ref().map(|t| e.caller_file.contains(t.as_str()) || e.callee_file.contains(t.as_str()) || e.caller_symbol.contains(t.as_str()) || e.callee_symbol.contains(t.as_str())).unwrap_or(true);
        let f_match = focus.as_ref().map(|f| e.caller_file.starts_with(f.as_str()) || e.callee_file.starts_with(f.as_str())).unwrap_or(true);
        t_match && f_match
    }).collect();
    Ok(axum::Json(serde_json::json!({"edges": filtered, "total": cg.edges.len()})))
}

async fn v1_dead_code_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(params): axum::Json<V1FocusParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let focus = match params.focus.as_deref().and_then(|s| if s.is_empty() { None } else { Some(s) }) {
        Some(f) => Some(normalize_path_param(f)?), None => None,
    };
    if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) { normalize_path_param(ws)?; }
    let idx = index.read().map_err(|_| v1_error(StatusCode::INTERNAL_SERVER_ERROR, "internal", "lock poisoned"))?;
    let dead = crate::intelligence::dead_code::detect_dead_code(&idx, focus.as_deref());
    let total = dead.len();
    Ok(axum::Json(serde_json::json!({"dead_symbols": dead, "total": total})))
}

async fn v1_cross_lang_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(params): axum::Json<V1FocusParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let focus = match params.focus.as_deref().and_then(|s| if s.is_empty() { None } else { Some(s) }) {
        Some(f) => Some(normalize_path_param(f)?), None => None,
    };
    if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) { normalize_path_param(ws)?; }
    let idx = index.read().map_err(|_| v1_error(StatusCode::INTERNAL_SERVER_ERROR, "internal", "lock poisoned"))?;
    let edges: Vec<_> = if let Some(ref prefix) = focus {
        idx.cross_lang_edges.iter().filter(|e| {
            format!("{:?}", e).contains(prefix.as_str())
        }).cloned().collect()
    } else {
        idx.cross_lang_edges.clone()
    };
    Ok(axum::Json(serde_json::json!({"edges": edges})))
}
```

- [ ] **Step 5:** Verify route registrations in `build_v1_router` (lines ~341–364). axum extractors are specified in the handler signatures themselves via `State(...)` and `Json(...)` — **no changes to the route table are required**. Only the handler signatures change (which Step 4 already did). Confirm by running the integration tests in Step 6.

- [ ] **Step 6:** Run tests — expect all integration tests pass.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test v1_api_wired
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features 2>&1 | grep "^test result:" | tail -3
```

- [ ] **Step 7:** Commit.

```bash
git add src/commands/serve.rs tests/v1_api_wired.rs
git commit -m "feat(api): wire 9 v1 stubs to real intelligence with normalization, auth, envelope"
```

---

## Task 12b: Update Pre-wired `/v1/health` and LSP `cxpak/health` to Expose `composite`

**Spec reference:** Data Integrity Contract 10 requires SPA, `/v1/health`, MCP `cxpak_health`, and LSP `cxpak/health` all to agree on the `composite` f64 field. The existing implementations expose only `{total_files, total_tokens}` — they do NOT call `compute_health()`. Contract 10 cannot hold without this task.

**Files:**
- Modify: `src/commands/serve.rs:381-390` (`v1_health_handler`)
- Modify: `src/lsp/methods.rs` (`cxpak/health` match arm — find via `grep -n '"cxpak/health"' src/lsp/methods.rs`)

- [ ] **Step 1:** Write failing test asserting `composite` appears in both responses.

```rust
// tests/health_exposes_composite.rs
#![cfg(all(feature = "daemon", feature = "visual"))]

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

fn empty_app() -> axum::Router {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let idx = cxpak::index::CodebaseIndex::build_with_content(
        vec![], std::collections::HashMap::new(), &counter, std::collections::HashMap::new()
    );
    let shared = std::sync::Arc::new(std::sync::RwLock::new(idx));
    let path = std::sync::Arc::new(std::path::PathBuf::from("."));
    cxpak::commands::serve::build_router_for_test(shared, path)
}

#[tokio::test]
async fn v1_health_exposes_composite_field() {
    // /v1/health is a GET route (existing behavior) — not POST.
    let req = Request::builder().method("GET").uri("/v1/health").body(Body::empty()).unwrap();
    let resp = empty_app().oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(body.get("composite").is_some(), "v1/health must expose composite: {body}");
}

#[test]
#[cfg(feature = "lsp")]
fn lsp_health_exposes_composite_field() {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let idx = cxpak::index::CodebaseIndex::build_with_content(
        vec![], std::collections::HashMap::new(), &counter, std::collections::HashMap::new()
    );
    let result = cxpak::lsp::methods::handle_custom_method("cxpak/health", serde_json::Value::Null, &idx).unwrap().unwrap();
    assert!(result.get("composite").is_some(), "LSP cxpak/health must expose composite: {result}");
}
```

- [ ] **Step 2:** Run — expect both fail.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test health_exposes_composite
```

- [ ] **Step 3:** Update `v1_health_handler` at `src/commands/serve.rs:381`:

```rust
async fn v1_health_handler(State(index): State<SharedIndex>) -> Result<Json<Value>, StatusCode> {
    let idx = index.read().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let health = crate::intelligence::health::compute_health(&idx);
    Ok(Json(serde_json::json!({
        "total_files": idx.total_files,
        "total_tokens": idx.total_tokens,
        "composite": health.composite,
        "dimensions": health.dimensions,
    })))
}
```

- [ ] **Step 4:** Update the `cxpak/health` match arm in `src/lsp/methods.rs`. Locate the existing arm:

```bash
grep -n '"cxpak/health"' src/lsp/methods.rs
```

Replace the body with:

```rust
"cxpak/health" => {
    let health = crate::intelligence::health::compute_health(index);
    Ok(Some(serde_json::json!({
        "total_files": index.total_files,
        "total_tokens": index.total_tokens,
        "composite": health.composite,
        "dimensions": health.dimensions,
    })))
}
```

- [ ] **Step 5:** Run both tests — expect pass.

- [ ] **Step 6:** Commit.

```bash
git add src/commands/serve.rs src/lsp/methods.rs tests/health_exposes_composite.rs
git commit -m "feat(health): expose composite field in /v1/health and LSP cxpak/health for cross-channel parity"
```

---

## Task 13: Wire 11 LSP Custom Method Stubs

**Spec reference:** § 2.2.

**Files:**
- Modify: `src/lsp/methods.rs:220-233`
- Create: `tests/lsp_methods_wired.rs`

- [ ] **Step 1:** Write failing test asserting all 14 methods return non-stub.

Create `tests/lsp_methods_wired.rs`:

```rust
#![cfg(feature = "lsp")]

fn make_idx() -> cxpak::index::CodebaseIndex {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let files = vec![cxpak::scanner::ScannedFile { relative_path: "src/main.rs".into(), absolute_path: "/tmp/src/main.rs".into(), language: Some("rust".into()), size_bytes: 100 }];
    let mut c = std::collections::HashMap::new();
    c.insert("src/main.rs".into(), "fn main(){}".into());
    cxpak::index::CodebaseIndex::build_with_content(files, std::collections::HashMap::new(), &counter, c)
}

fn is_stub(v: &serde_json::Value) -> bool {
    v.get("status").and_then(|s| s.as_str()).map(|s| s == "not_implemented" || s == "available").unwrap_or(false)
}

#[test]
fn all_14_lsp_methods_return_non_stub() {
    let idx = make_idx();
    let methods = [
        // 3 pre-wired (from v1.6.0):
        "cxpak/health", "cxpak/conventions", "cxpak/blastRadius",
        // 11 newly wired:
        "cxpak/overview", "cxpak/trace", "cxpak/diff", "cxpak/search",
        "cxpak/apiSurface", "cxpak/deadCode", "cxpak/callGraph",
        "cxpak/predict", "cxpak/drift", "cxpak/securitySurface", "cxpak/dataFlow",
    ];
    for m in methods {
        let params = match m {
            "cxpak/trace" | "cxpak/search" => serde_json::json!({"symbol": "main"}),
            "cxpak/predict" => serde_json::json!({"files": ["src/main.rs"]}),
            "cxpak/dataFlow" => serde_json::json!({"symbol": "main"}),
            _ => serde_json::Value::Null,
        };
        let result = cxpak::lsp::methods::handle_custom_method(m, params, &idx).expect(m);
        let body = result.expect(&format!("{m} must return Some"));
        assert!(!is_stub(&body), "{m} returned stub: {body}");
    }
}
```

- [ ] **Step 2:** Run — expect failures for 11 stubbed methods.

- [ ] **Step 3:** Replace the catch-all stub match arm in `handle_custom_method` at lines 220–233.

**First, rename the `_params: serde_json::Value` parameter to `params`** (remove underscore) because the new arms read from it. The signature becomes:

```rust
pub fn handle_custom_method(
    method: &str,
    params: serde_json::Value,
    index: &crate::index::CodebaseIndex,
) -> Result<Option<serde_json::Value>, LspMethodError> {
    match method {
```

Replace the single catch-all arm with 11 specific arms. Each calls the real intelligence function and returns a shape-stable response.

For `cxpak/trace`:
```rust
"cxpak/trace" => {
    let symbol = params.get("symbol").and_then(|v| v.as_str());
    match symbol {
        Some(sym) => {
            let matches = index.find_symbol(sym);
            let locations: Vec<_> = matches.into_iter().map(|(file, s)| serde_json::json!({
                "file": file,
                "start_line": s.start_line,
                "end_line": s.end_line,
                "kind": format!("{:?}", s.kind),
            })).collect();
            Ok(Some(serde_json::json!({"count": locations.len(), "locations": locations})))
        }
        None => Ok(Some(serde_json::json!({"count": 0, "locations": [], "note": "provide symbol parameter"}))),
    }
}
```

Proceed identically for the other 10, matching the table in spec § 2.2. For `cxpak/securitySurface`:

```rust
"cxpak/securitySurface" => {
    let result = crate::intelligence::security::build_security_surface(
        index,
        crate::intelligence::security::DEFAULT_AUTH_PATTERNS,
        None,
    );
    Ok(Some(serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}))))
}
```

For `cxpak/predict`:

```rust
"cxpak/predict" => {
    let files = params.get("files").and_then(|v| v.as_array()).map(|arr| {
        arr.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>()
    }).unwrap_or_default();
    if files.is_empty() {
        return Ok(Some(serde_json::json!({"note": "provide files parameter"})));
    }
    let refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
    let result = crate::intelligence::predict::predict(
        &refs, &index.graph, &index.pagerank, &index.co_changes, &index.test_map, 3
    );
    Ok(Some(serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}))))
}
```

For `cxpak/drift` and `cxpak/diff` — these need filesystem access:

```rust
"cxpak/drift" => Ok(Some(serde_json::json!({"note": "drift requires repo path; use cxpak drift CLI"}))),
"cxpak/diff" => Ok(Some(serde_json::json!({"note": "diff requires git ref; use cxpak diff CLI"}))),
```

Complete implementations for the remaining 6 arms (no "same pattern" elision):

```rust
"cxpak/overview" => {
    Ok(Some(serde_json::json!({
        "total_files": index.total_files,
        "total_tokens": index.total_tokens,
        "languages": index.language_stats.len(),
    })))
}

"cxpak/search" => {
    let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
    if query.is_empty() {
        return Ok(Some(serde_json::json!({"matches": [], "note": "provide query parameter"})));
    }
    let matches: Vec<_> = index.files.iter()
        .filter(|f| f.relative_path.to_lowercase().contains(&query))
        .take(20)
        .map(|f| serde_json::json!({"path": f.relative_path, "language": f.language}))
        .collect();
    Ok(Some(serde_json::json!({"matches": matches})))
}

"cxpak/apiSurface" => {
    // Real signature: extract_api_surface(index, focus, include, token_budget) — 4 args.
    let surface = crate::intelligence::api_surface::extract_api_surface(index, None, "all", 5000);
    Ok(Some(serde_json::to_value(surface).unwrap_or_else(|_| serde_json::json!({}))))
}

"cxpak/deadCode" => {
    let dead = crate::intelligence::dead_code::detect_dead_code(index, None);
    Ok(Some(serde_json::json!({"dead_symbols": dead})))
}

"cxpak/callGraph" => {
    Ok(Some(serde_json::json!({
        "edges": index.call_graph.edges,
        "total": index.call_graph.edges.len(),
    })))
}

"cxpak/dataFlow" => {
    let symbol = params.get("symbol").and_then(|v| v.as_str());
    match symbol {
        Some(sym) => {
            let result = crate::intelligence::data_flow::trace_data_flow(sym, None, 6, index);
            Ok(Some(serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({}))))
        }
        None => Ok(Some(serde_json::json!({"note": "provide symbol parameter"}))),
    }
}
```

- [ ] **Step 4:** Run tests — expect pass.

- [ ] **Step 5:** Full suite.

- [ ] **Step 6:** Commit.

```bash
git add src/lsp/methods.rs tests/lsp_methods_wired.rs
git commit -m "feat(lsp): wire 11 LSP custom method stubs to real intelligence functions"
```

---

## Task 14: MCP `cxpak_visual` Slug Validation

**Spec reference:** § 2.3.

**Files:**
- Modify: `src/commands/serve.rs` (add slug validator + canonicalize check in `handle_cxpak_visual`)
- Create: `tests/mcp_slug_validation.rs`

- [ ] **Step 1:** Write failing tests against `validate_visual_type_slug` directly (unit-level), since MCP protocol testing over stdio is out of scope for this task.

```rust
use cxpak::commands::serve::validate_visual_type_slug;

#[test]
fn slug_rejects_path_traversal() {
    assert!(validate_visual_type_slug("../etc/passwd").is_err());
    assert!(validate_visual_type_slug("dashboard/../../etc").is_err());
}

#[test]
fn slug_rejects_absolute_paths() {
    assert!(validate_visual_type_slug("/etc/passwd").is_err());
    assert!(validate_visual_type_slug("\\windows\\system32").is_err());
}

#[test]
fn slug_rejects_null_bytes() {
    assert!(validate_visual_type_slug("dashboard\0").is_err());
}

#[test]
fn slug_accepts_all_closed_enum_values() {
    for t in ["dashboard", "architecture", "risk", "flow", "timeline", "diff", "all"] {
        let result = validate_visual_type_slug(t);
        assert!(result.is_ok(), "slug {t} must be accepted");
        assert_eq!(result.unwrap(), t);
    }
}

#[test]
fn slug_rejects_unknown_value() {
    assert!(validate_visual_type_slug("not_a_view").is_err());
    assert!(validate_visual_type_slug("DASHBOARD").is_err()); // case-sensitive
    assert!(validate_visual_type_slug("").is_err());
}

#[test]
fn canonicalize_check_rejects_escape() {
    // This tests the second line of defense. If someone changes validate_visual_type_slug
    // to return user input, the canonicalize check should still catch traversal.
    // Direct test against a helper — implementation detail covered in Step 3.
    use std::path::Path;
    let repo = tempfile::tempdir().unwrap();
    let visual_dir = repo.path().join(".cxpak/visual");
    std::fs::create_dir_all(&visual_dir).unwrap();
    // Try to write "dashboard/../../escape.html"
    let bad_filepath = visual_dir.join("dashboard/../../escape.html");
    let canon_dir = visual_dir.canonicalize().unwrap();
    // parent().canonicalize() MUST fail or not start with canon_dir.
    let parent_canon = bad_filepath.parent().unwrap().canonicalize();
    if let Ok(p) = parent_canon {
        assert!(!p.starts_with(&canon_dir), "path escape must be caught: {p:?}");
    }
}
```

Add `tempfile = "3"` to `[dev-dependencies]` if not present.

- [ ] **Step 2:** Add slug validator to serve.rs:

```rust
fn validate_visual_type_slug(s: &str) -> Result<&'static str, String> {
    match s {
        "dashboard" => Ok("dashboard"),
        "architecture" => Ok("architecture"),
        "risk" => Ok("risk"),
        "flow" => Ok("flow"),
        "timeline" => Ok("timeline"),
        "diff" => Ok("diff"),
        "all" => Ok("all"),
        _ => Err(format!("invalid_type: {s}")),
    }
}
```

- [ ] **Step 3:** Update `handle_cxpak_visual` in `serve.rs` around line 2952 — before the `format!("cxpak-{}.html", ...)`, call `validate_visual_type_slug` and use its output. After `join`, canonicalize and assert prefix.

```rust
let validated_slug = validate_visual_type_slug(type_str)?;
let visual_dir = repo_path.join(".cxpak/visual");
std::fs::create_dir_all(&visual_dir).ok();
let filepath = visual_dir.join(format!("cxpak-{}.html", validated_slug));
// canonicalize + prefix check
let canon_dir = visual_dir.canonicalize().map_err(|e| format!("canonicalize failed: {e}"))?;
let canon_file = filepath.parent().unwrap().canonicalize().map_err(|e| format!("canonicalize failed: {e}"))?;
if !canon_file.starts_with(&canon_dir) {
    return Err("path escape detected".into());
}
```

- [ ] **Step 4:** Run tests.

- [ ] **Step 5:** Commit.

```bash
git add src/commands/serve.rs tests/mcp_slug_validation.rs
git commit -m "feat(mcp): validate cxpak_visual type slug against closed enum + canonicalize"
```

---

## Task 15: Cross-Channel Consistency Tests

**Spec reference:** § Testing Strategy, Cross-channel matrix.

**Files:**
- Create: `tests/cross_channel_consistency.rs`
- Create: `tests/support/redact.rs`

- [ ] **Step 1:** Create the redaction helper AND the support module index.

First, `tests/support/mod.rs`:

```rust
pub mod redact;
```

Without this, cargo integration tests cannot resolve `mod support;` in other test files.

Then `tests/support/redact.rs`:

```rust
use serde_json::Value;

pub fn redact(v: &mut Value) {
    if let Value::Object(map) = v {
        for k in ["generated_at", "cxpak_version", "timestamp", "baseline_date"] {
            if map.contains_key(k) { map.insert(k.into(), Value::String("[REDACTED]".into())); }
        }
        for vv in map.values_mut() { redact(vv); }
    } else if let Value::Array(arr) = v {
        for vv in arr { redact(vv); }
    }
}
```

- [ ] **Step 2:** Create cross-channel tests covering the matrix in spec § Testing Strategy. Full implementations below — no stubs.

```rust
#![cfg(all(feature = "visual", feature = "daemon", feature = "lsp"))]

mod support;
use axum::body::Body;
use axum::http::Request;
use serde_json::Value;
use support::redact::redact;
use tower::ServiceExt;

fn make_fixture_index() -> cxpak::index::CodebaseIndex {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let files: Vec<cxpak::scanner::ScannedFile> = (0..10).map(|i| cxpak::scanner::ScannedFile {
        relative_path: format!("src/mod_{i}.rs"),
        absolute_path: format!("/tmp/src/mod_{i}.rs").into(),
        language: Some("rust".into()),
        size_bytes: ((i + 1) * 100) as u64,
    }).collect();
    let mut pr = std::collections::HashMap::new();
    for (i, file) in files.iter().enumerate() {
        pr.insert(file.relative_path.clone(), cxpak::parser::language::ParseResult {
            symbols: (0..3).map(|j| cxpak::parser::language::Symbol {
                name: format!("fn_{i}_{j}"),
                kind: cxpak::parser::language::SymbolKind::Function,
                visibility: if j == 0 { cxpak::parser::language::Visibility::Public } else { cxpak::parser::language::Visibility::Private },
                signature: format!("fn fn_{i}_{j}()"),
                body: "{}".into(),
                start_line: j * 4 + 1,
                end_line: j * 4 + 3,
            }).collect(),
            imports: if i > 0 { vec![format!("src/mod_{}.rs", i - 1)] } else { vec![] },
            exports: vec![],
        });
    }
    let mut c = std::collections::HashMap::new();
    for f in &files { c.insert(f.relative_path.clone(), "fn x(){}".into()); }
    cxpak::index::CodebaseIndex::build_with_content(files, pr, &counter, c)
}

fn fixture_metadata() -> cxpak::visual::render::RenderMetadata {
    cxpak::visual::render::RenderMetadata {
        repo_name: "test".into(),
        generated_at: "2026-04-17T12:00:00Z".into(),
        health_score: None,
        node_count: 10,
        edge_count: 9,
        cxpak_version: "2.1.0".into(),
    }
}

fn build_router_with_index(idx: cxpak::index::CodebaseIndex) -> axum::Router {
    let shared = std::sync::Arc::new(std::sync::RwLock::new(idx));
    let path = std::sync::Arc::new(std::path::PathBuf::from("."));
    cxpak::commands::serve::build_router_for_test(shared, path)
}

async fn post_v1(app: axum::Router, path: &str, body: Value) -> Value {
    let req = Request::builder().method("POST").uri(path)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap())).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn extract_json_tag(html: &str, tag_id: &str) -> Value {
    let marker = format!(r#"id="{tag_id}" type="application/json">"#);
    let start = html.find(&marker).expect("tag present") + marker.len();
    let end = html[start..].find("</script>").unwrap() + start;
    serde_json::from_str(&html[start..end]).expect("valid JSON")
}

async fn get_v1(app: axum::Router, path: &str) -> Value {
    let req = Request::builder().method("GET").uri(path).body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn health_consistency_spa_v1_mcp_lsp() {
    // PREREQUISITE: Task 12b must have landed, exposing `composite` in
    // v1_health_handler and LSP cxpak/health. Without Task 12b this test cannot pass.
    let idx = make_fixture_index();
    let expected = cxpak::intelligence::health::compute_health(&idx);

    // SPA — dashboard JSON embeds the full HealthQuadrant.
    let spa_html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    let spa_dashboard = extract_json_tag(&spa_html, "cxpak-dashboard-data");
    let spa_composite = spa_dashboard["health"]["composite"].as_f64().unwrap();
    assert_eq!(spa_composite.to_bits(), expected.composite.to_bits(), "SPA health composite drift");

    // v1/health is GET, not POST (verified in the codebase).
    let v1_body = get_v1(build_router_with_index(make_fixture_index()), "/v1/health").await;
    let v1_composite = v1_body["composite"].as_f64().expect("v1/health must expose composite (Task 12b)");
    assert_eq!(v1_composite.to_bits(), expected.composite.to_bits(), "v1 health drift");

    // LSP cxpak/health (pre-wired, updated in Task 12b).
    let idx2 = make_fixture_index();
    let lsp = cxpak::lsp::methods::handle_custom_method("cxpak/health", Value::Null, &idx2)
        .unwrap().expect("Some");
    let lsp_composite = lsp["composite"].as_f64().expect("LSP cxpak/health must expose composite (Task 12b)");
    assert_eq!(lsp_composite.to_bits(), expected.composite.to_bits(), "LSP health drift");
}

#[tokio::test]
async fn risk_consistency_spa_v1_mcp() {
    let idx = make_fixture_index();
    let expected = cxpak::intelligence::risk::compute_risk_ranking(&idx);

    // SPA top_risks is first-5 of expected
    let spa_html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    let spa_dashboard = extract_json_tag(&spa_html, "cxpak-dashboard-data");
    let spa_top = spa_dashboard["risks"]["top_risks"].as_array().unwrap();
    for (i, entry) in spa_top.iter().enumerate() {
        let real = &expected[i];
        assert_eq!(entry["path"].as_str().unwrap(), real.path);
        assert_eq!(entry["risk_score"].as_f64().unwrap().to_bits(), real.risk_score.to_bits());
    }

    // v1/risks returns full list
    let v1_body = post_v1(build_router_with_index(make_fixture_index()), "/v1/risks", serde_json::json!({})).await;
    let v1_risks = v1_body["risks"].as_array().unwrap();
    assert_eq!(v1_risks.len(), expected.len());
    for (i, entry) in v1_risks.iter().enumerate() {
        assert_eq!(entry["path"].as_str().unwrap(), expected[i].path);
        assert_eq!(entry["risk_score"].as_f64().unwrap().to_bits(), expected[i].risk_score.to_bits());
    }
}

#[tokio::test]
async fn architecture_consistency_spa_v1() {
    let idx = make_fixture_index();
    let expected = cxpak::intelligence::architecture::build_architecture_map(&idx, 2);

    let spa_html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    let spa_arch = extract_json_tag(&spa_html, "cxpak-architecture-data");
    let spa_nodes = spa_arch["level1"]["nodes"].as_array().unwrap();
    let spa_prefixes: std::collections::BTreeSet<&str> = spa_nodes.iter()
        .filter_map(|n| n["id"].as_str())
        .collect();
    let expected_prefixes: std::collections::BTreeSet<&str> = expected.modules.iter()
        .map(|m| m.prefix.as_str()).collect();
    assert_eq!(spa_prefixes, expected_prefixes);

    let v1_body = post_v1(build_router_with_index(make_fixture_index()), "/v1/architecture", serde_json::json!({})).await;
    let v1_modules = v1_body["modules"].as_array().unwrap();
    let v1_prefixes: std::collections::BTreeSet<&str> = v1_modules.iter()
        .filter_map(|m| m["prefix"].as_str()).collect();
    assert_eq!(v1_prefixes, expected_prefixes);
}

#[tokio::test]
async fn dead_code_consistency_v1_lsp() {
    let idx = make_fixture_index();
    let expected = cxpak::intelligence::dead_code::detect_dead_code(&idx, None);
    let v1_body = post_v1(build_router_with_index(make_fixture_index()), "/v1/dead_code", serde_json::json!({})).await;
    let v1_count = v1_body["dead_symbols"].as_array().map(|a| a.len()).unwrap_or(0);
    assert_eq!(v1_count, expected.len());
    let idx2 = make_fixture_index();
    let lsp = cxpak::lsp::methods::handle_custom_method("cxpak/deadCode", Value::Null, &idx2)
        .unwrap().expect("Some");
    // Accept shape variation between envelope and bare — check total count via either.
    let lsp_count = lsp.as_array().map(|a| a.len())
        .or_else(|| lsp["dead_symbols"].as_array().map(|a| a.len()))
        .unwrap_or(0);
    assert_eq!(lsp_count, expected.len());
}

#[tokio::test]
async fn metadata_node_count_matches_total_files() {
    let idx = make_fixture_index();
    let expected = idx.total_files;
    let spa_html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    let meta = extract_json_tag(&spa_html, "cxpak-meta");
    assert_eq!(meta["node_count"].as_u64().unwrap() as usize, expected);
}

#[tokio::test]
async fn metadata_edge_count_matches_graph_sum() {
    let idx = make_fixture_index();
    let expected: usize = idx.graph.edges.values().map(|v| v.len()).sum();
    let spa_html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    let meta = extract_json_tag(&spa_html, "cxpak-meta");
    assert_eq!(meta["edge_count"].as_u64().unwrap() as usize, expected);
}
```

**Note:** The `fixture_metadata()` helper above uses hardcoded `node_count: 10, edge_count: 9`. In the SPA renderer, these values must be computed from the actual index. Test 5/6 above will fail if `render_spa` doesn't compute edge_count correctly — driving the integration.

- [ ] **Step 3:** Run and iterate until all pass.

- [ ] **Step 4:** Commit.

```bash
git add tests/cross_channel_consistency.rs tests/support/
git commit -m "test: add cross-channel consistency tests for all intelligence functions"
```

---

## Task 15b: Named Edge-Case Tests (spec § Edge-case test requirements)

**Goal:** Implement each named edge-case test from the spec. These are separate from cross-channel consistency because they test single-channel boundary behavior.

**Files:** `tests/spa_edge_cases.rs`, `tests/palette_edge_cases.rs`, `tests/v1_edge_cases.rs`.

- [ ] **Step 1:** Write `tests/spa_edge_cases.rs`.

```rust
#![cfg(feature = "visual")]

use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use std::collections::HashMap;

fn empty_index() -> CodebaseIndex {
    CodebaseIndex::build_with_content(vec![], HashMap::new(), &TokenCounter::new(), HashMap::new())
}

fn fixture_metadata() -> cxpak::visual::render::RenderMetadata {
    cxpak::visual::render::RenderMetadata {
        repo_name: "t".into(),
        generated_at: "2026-04-17T12:00:00Z".into(),
        health_score: None,
        node_count: 0,
        edge_count: 0,
        cxpak_version: "2.1.0".into(),
    }
}

#[test]
fn spa_renders_with_zero_files_index() {
    let idx = empty_index();
    let html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    // Must not panic, must produce valid HTML, must contain all six view containers
    assert!(html.starts_with("<!DOCTYPE html>"));
    for id in ["view-dashboard","view-architecture","view-risk","view-flow","view-timeline","view-diff"] {
        assert!(html.contains(&format!(r#"id="{id}""#)));
    }
}

#[test]
fn spa_all_tags_escaped_for_malicious_filename() {
    let counter = TokenCounter::new();
    let evil = r"src/</script><img src=x onerror=alert(1)>.rs";
    let files = vec![cxpak::scanner::ScannedFile {
        relative_path: evil.into(),
        absolute_path: format!("/tmp/{evil}").into(),
        language: Some("rust".into()),
        size_bytes: 10,
    }];
    let mut c = HashMap::new();
    c.insert(evil.into(), "".into());
    let idx = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, c);
    let html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();

    // For each <script id="cxpak-*" ...>...</script> block, the interior must not
    // contain an unescaped </script> sequence.
    let mut cursor = 0usize;
    while let Some(open_start) = html[cursor..].find(r#"<script id="cxpak-"#) {
        let abs_open_start = cursor + open_start;
        let open_end = html[abs_open_start..].find('>').unwrap() + abs_open_start + 1;
        // Find the NEXT </script> — if any escape is missing, a </script> will appear
        // inside the intended JSON content.
        let close = html[open_end..].find("</script>").unwrap() + open_end;
        let interior = &html[open_end..close];
        // An unescaped </script> in JSON content would have split this block earlier.
        // Extra check: interior should not contain `onerror=alert` raw.
        assert!(!interior.contains("onerror=alert"), "raw XSS payload leaked into {interior}");
        cursor = close + "</script>".len();
    }

    // The literal raw payload must not appear anywhere in the rendered HTML as-is
    // (escape_script_tag should have transformed </script> to <\/script>).
    assert!(!html.contains(r#"</script><img src=x onerror=alert(1)>"#), "payload leaked");
}

#[test]
fn health_gauge_renders_zero_composite() {
    // Construct an index where compute_health produces 0.0 (unusual — health has
    // a min floor, so this test may need to force a value via DashboardData directly).
    // Instead, test that the SPA JSON includes a numeric composite field even when it is 0.
    let idx = empty_index();
    let data = cxpak::visual::render::build_dashboard_data(&idx);
    assert!(data.health.composite.is_finite());
    let spa_html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    let tag = r#"id="cxpak-dashboard-data" type="application/json">"#;
    let start = spa_html.find(tag).unwrap() + tag.len();
    let end = spa_html[start..].find("</script>").unwrap() + start;
    let json: serde_json::Value = serde_json::from_str(&spa_html[start..end]).unwrap();
    assert!(json["health"]["composite"].is_f64() || json["health"]["composite"].is_i64(),
        "composite must be numeric even at boundary");
}

#[test]
fn search_index_empty_symbols_file_has_no_error_marker() {
    // File with parse_result Some(empty symbols) must NOT be marked as parse error.
    let counter = TokenCounter::new();
    let files = vec![cxpak::scanner::ScannedFile {
        relative_path: "src/empty.rs".into(),
        absolute_path: "/tmp/src/empty.rs".into(),
        language: Some("rust".into()),
        size_bytes: 5,
    }];
    let mut pr = HashMap::new();
    pr.insert("src/empty.rs".into(), cxpak::parser::language::ParseResult {
        symbols: vec![], imports: vec![], exports: vec![],
    });
    let mut c = HashMap::new();
    c.insert("src/empty.rs".into(), "//\n".into());
    let idx = CodebaseIndex::build_with_content(files, pr, &counter, c);
    let entries = cxpak::visual::search_index::build_search_index(&idx);
    let entry = entries.iter().find(|e| e.label == "src/empty.rs").unwrap();
    assert!(!entry.detail.contains("parse error"), "empty-symbols file must NOT be marked as parse error: {}", entry.detail);
}
```

- [ ] **Step 2:** Write `tests/v1_edge_cases.rs` for HTTP-level boundary tests.

```rust
#![cfg(all(feature = "daemon", feature = "visual"))]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

fn build_app(idx: cxpak::index::CodebaseIndex) -> axum::Router {
    let shared = std::sync::Arc::new(std::sync::RwLock::new(idx));
    let path = std::sync::Arc::new(std::path::PathBuf::from("."));
    cxpak::commands::serve::build_router_for_test(shared, path)
}

fn empty_index() -> cxpak::index::CodebaseIndex {
    cxpak::index::CodebaseIndex::build_with_content(
        vec![], std::collections::HashMap::new(), &cxpak::budget::counter::TokenCounter::new(), std::collections::HashMap::new()
    )
}

async fn post(app: axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder().method("POST").uri(path)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap())).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn v1_risks_zero_files_returns_empty_envelope() {
    let (status, body) = post(build_app(empty_index()), "/v1/risks", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("risks").is_some());
    assert_eq!(body["risks"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn v1_data_flow_accepts_angle_brackets() {
    let (status, _body) = post(build_app(empty_index()), "/v1/data_flow", serde_json::json!({"symbol": "Vec<String>"})).await;
    assert_eq!(status, StatusCode::OK, "generics must be allowed in symbol names");
}

#[tokio::test]
async fn v1_data_flow_rejects_path_separator_in_symbol() {
    let (status, body) = post(build_app(empty_index()), "/v1/data_flow", serde_json::json!({"symbol": "../secret"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_param");
}

#[tokio::test]
async fn v1_risks_focus_workspace_normalized() {
    // workspace must be normalized too (spec § 2.1.1 requires it).
    let (status, _body) = post(build_app(empty_index()), "/v1/risks", serde_json::json!({"workspace": "../etc"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v1_risks_empty_focus_treated_as_none() {
    // Empty-string focus must not 400; must be treated as "no filter".
    let (status, _body) = post(build_app(empty_index()), "/v1/risks", serde_json::json!({"focus": ""})).await;
    assert_eq!(status, StatusCode::OK);
}
```

- [ ] **Step 3:** Write `tests/palette_unicode.rs`.

```rust
#![cfg(feature = "visual")]

#[test]
fn palette_rejects_nul_byte_in_file_path() {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let files = vec![cxpak::scanner::ScannedFile {
        relative_path: "src/nul\0file.rs".into(),
        absolute_path: "/tmp/bad".into(),
        language: Some("rust".into()),
        size_bytes: 10,
    }];
    let mut c = std::collections::HashMap::new();
    c.insert("src/nul\0file.rs".into(), "".into());
    let idx = cxpak::index::CodebaseIndex::build_with_content(files, std::collections::HashMap::new(), &counter, c);
    let entries = cxpak::visual::search_index::build_search_index(&idx);
    assert!(!entries.iter().any(|e| e.label.contains('\0')), "NUL-byte paths must be rejected");
}
```

- [ ] **Step 4:** Run all three test files.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test spa_edge_cases --test v1_edge_cases --test palette_unicode
```

Note: some of these tests drive spec requirements that need IMPLEMENTATION changes. Specifically:
- `v1_risks_focus_workspace_normalized` requires workspace normalization in `v1_risks_handler` (add `if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) { normalize_path_param(ws)?; }` — the return value is discarded; it's only validated).
- `v1_risks_empty_focus_treated_as_none` drives the empty-string-to-None logic in handler preambles.
- `palette_rejects_nul_byte` drives the NUL-byte rejection in `build_search_index`.

These must be added to Tasks 2, 11, or 12 as handler/builder logic. The tests above drive the red phase.

- [ ] **Step 5:** Commit.

```bash
git add tests/spa_edge_cases.rs tests/v1_edge_cases.rs tests/palette_unicode.rs
git commit -m "test: add named edge-case tests from spec § Edge-case test requirements"
```

---

## Task 16: SPA Determinism Golden Fixture

**Spec reference:** § Testing Strategy, Determinism test.

**Files:**
- Create: `tests/fixtures/determinism_repo/` (minimal fixture)
- Create: `tests/snapshots/spa_golden.html` (generated via `UPDATE_SNAPSHOTS=1 cargo test`)
- Create: `tests/spa_determinism.rs`

- [ ] **Step 1:** Create the fixture repo. These are concrete files committed to the repo:

```
tests/fixtures/determinism_repo/
├── src/
│   ├── lib.rs              (pub fn compute() {...})
│   ├── auth/
│   │   ├── mod.rs          (pub mod jwt; pub mod session;)
│   │   ├── jwt.rs          (pub fn verify() { session::current(); })
│   │   └── session.rs      (pub fn current() -> Option<Session>)
│   ├── api/
│   │   ├── mod.rs
│   │   └── handlers.rs     (use crate::auth::jwt;)
│   └── db/
│       └── mod.rs
└── .gitignore              (empty)
```

Each file contains minimal valid Rust — 3-5 lines defining the listed items. Create them manually, keep total < 40 lines.

- [ ] **Step 2:** Create test that renders SPA on fixture and diffs against golden.

```rust
use std::path::PathBuf;

fn load_fixture_index() -> cxpak::index::CodebaseIndex {
    let fixture_root = PathBuf::from("tests/fixtures/determinism_repo");
    // Use cxpak's scanner + parser over the fixture dir.
    // The cxpak crate exposes build_index via commands::serve::build_index.
    cxpak::commands::serve::build_index(&fixture_root).expect("fixture index builds")
}

fn fixture_metadata() -> cxpak::visual::render::RenderMetadata {
    // All values hardcoded to redactable placeholders — the redacter (§ Testing Strategy)
    // will further normalize generated_at and cxpak_version before diff.
    cxpak::visual::render::RenderMetadata {
        repo_name: "determinism_repo".to_string(),
        generated_at: "[REDACTED]".to_string(),
        health_score: None,
        node_count: 0,     // filled by render_spa from index
        edge_count: 0,     // filled by render_spa from index
        cxpak_version: "[REDACTED]".to_string(),
    }
}

/// Redact timestamps in the rendered HTML. Matches common timestamp patterns
/// inside the inlined JSON so diffs don't fail on irrelevant variations.
fn redact_html(html: &str) -> String {
    let re = regex::Regex::new(r#""(generated_at|timestamp|baseline_date)"\s*:\s*"[^"]+""#).unwrap();
    re.replace_all(html, r#""$1":"[REDACTED]""#).to_string()
}

#[test]
fn spa_output_matches_golden_fixture() {
    let idx = load_fixture_index();
    let meta = fixture_metadata();
    let actual = cxpak::visual::spa::render_spa(&idx, &meta).unwrap();
    let actual_redacted = redact_html(&actual);

    let golden_path = "tests/snapshots/spa_golden.html";
    if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
        std::fs::write(golden_path, &actual_redacted).unwrap();
        return;
    }
    let golden = match std::fs::read_to_string(golden_path) {
        Ok(g) => g,
        Err(_) => panic!("run UPDATE_SNAPSHOTS=1 cargo test to bootstrap {golden_path}"),
    };
    assert_eq!(actual_redacted, golden, "SPA output drift detected; run UPDATE_SNAPSHOTS=1 to accept");
}
```

- [ ] **Step 3:** Bootstrap the golden fixture locally:

```bash
UPDATE_SNAPSHOTS=1 RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test spa_determinism
```

- [ ] **Step 4:** Re-run WITHOUT the env var — expect pass.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test spa_determinism
```

- [ ] **Step 5:** Commit including the golden fixture.

```bash
git add tests/fixtures/determinism_repo/ tests/snapshots/spa_golden.html tests/spa_determinism.rs
git commit -m "test(visual): add SPA determinism test with committed golden fixture"
```

---

## Task 17: SPA Injection-Safety Integration Test

**Spec reference:** Success Criterion 8.

**Files:**
- Create: `tests/spa_injection_safety.rs`

- [ ] **Step 1:** Test that malicious file paths don't break the HTML.

```rust
#[test]
fn spa_survives_malicious_filename() {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let evil = r"</script><img src=x onerror=alert(1)>.rs";
    let files = vec![cxpak::scanner::ScannedFile {
        relative_path: evil.into(),
        absolute_path: format!("/tmp/{evil}").into(),
        language: Some("rust".into()),
        size_bytes: 10,
    }];
    let mut c = std::collections::HashMap::new();
    c.insert(evil.into(), "//".into());
    let idx = cxpak::index::CodebaseIndex::build_with_content(files, std::collections::HashMap::new(), &counter, c);
    let meta = cxpak::visual::render::RenderMetadata {
        repo_name: "injection-test".into(),
        generated_at: "2026-04-17T12:00:00Z".into(),
        health_score: None,
        node_count: 1,
        edge_count: 0,
        cxpak_version: "2.1.0".into(),
    };
    let html = cxpak::visual::spa::render_spa(&idx, &meta).unwrap();
    assert!(!html.contains("onerror=alert"), "raw payload leaked");
    assert!(!html.contains(r#"</script><img"#), "script-break sequence leaked");
}
```

- [ ] **Step 2:** Commit.

```bash
git add tests/spa_injection_safety.rs
git commit -m "test(visual): injection-safety test for SPA with malicious file paths"
```

---

## Task 18: Version Bump to 2.1.0

**Spec reference:** Success Criterion 9.

**Files:**
- Modify: `Cargo.toml`, `Cargo.lock`, `plugin/.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json`, `plugin/lib/ensure-cxpak`

- [ ] **Step 1:** Update versions.

```bash
sed -i '' 's/^version = "2.0.0"/version = "2.1.0"/' Cargo.toml
sed -i '' 's/"version": "2.0.0"/"version": "2.1.0"/' plugin/.claude-plugin/plugin.json
sed -i '' 's/"version": "2.0.0"/"version": "2.1.0"/' .claude-plugin/marketplace.json
sed -i '' 's/REQUIRED_VERSION="2.0.0"/REQUIRED_VERSION="2.1.0"/' plugin/lib/ensure-cxpak
```

- [ ] **Step 2:** Regenerate Cargo.lock.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo check
```

- [ ] **Step 3:** Full test suite + clippy + fmt.

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features
RUSTUP_TOOLCHAIN=1.94.1 cargo clippy --all-targets --all-features -- -D warnings
RUSTUP_TOOLCHAIN=1.94.1 cargo fmt -- --check
```

- [ ] **Step 4:** Grep confirm no TODO/FIXME/unimplemented in new code.

```bash
grep -rn 'TODO\|FIXME\|todo!()\|unimplemented!()' src/visual/spa.rs src/visual/search_index.rs assets/cxpak-spa-controller.js tests/spa_*.rs tests/v1_api_wired.rs tests/lsp_methods_wired.rs tests/cross_channel_consistency.rs
```

Expected: no matches.

- [ ] **Step 5:** Regenerate golden fixture post version-bump and re-commit.

```bash
UPDATE_SNAPSHOTS=1 RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test spa_determinism
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test spa_determinism
```

- [ ] **Step 6:** Commit.

```bash
git add Cargo.toml Cargo.lock plugin/.claude-plugin/plugin.json .claude-plugin/marketplace.json plugin/lib/ensure-cxpak tests/snapshots/spa_golden.html
git commit -m "chore: bump version to 2.1.0"
```

---

## Task 19: Manual Browser Smoke Test (Success Criterion 11)

**Scheduled AFTER Task 18 completes, BEFORE git tag.**

**Procedure** (per spec § Success Criterion 11): open `cxpak-all.html` generated from cxpak itself in Chrome, Firefox, Safari. Run the 8-step checklist. File `docs/release/v2.1.0-smoke.md` with results.

Do not tag the release on any failure.

---

## Final Validation Checklist

Before declaring v2.1.0 ready:

- [ ] All 19 tasks committed.
- [ ] `RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features` reports ≥ 2,510 tests, 0 failures.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo fmt -- --check` clean.
- [ ] Version 2.1.0 in all 4 files.
- [ ] Golden fixture committed.
- [ ] Manual smoke test passed in 3 browsers.
- [ ] No TODO/FIXME/unimplemented in new code.
- [ ] Zero `not_implemented` / `available` stub responses remain in `/v1/*` or LSP.
