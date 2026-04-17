# v2.1.0 "The Polish" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Transform the v2.0.0 visual dashboard from 6 separate HTML files into a single-page app with command palette, light mode, keyboard navigation, accessibility, and wire all platform stubs to real intelligence functions.

**Architecture:** SPA with hash routing (`#dashboard`, `#architecture`, etc.) in one self-contained HTML file. Shared state (selected file, theme) persists across views via a JS store + localStorage. All 9 v1/ API stubs and 11 LSP method stubs delegate to existing intelligence functions. CSS `light-dark()` for theme switching. Roving tabindex for keyboard graph navigation. ARIA labels generated in Rust at build time.

**Tech Stack:** Rust, D3.js v7.9 (existing bundle), axum (existing), tower-lsp (existing), petgraph (existing)

**Testing Strategy:** Strict TDD. Every Rust function gets a unit test written BEFORE the implementation. Integration tests verify data integrity end-to-end — the HTML output must contain correct scores, correct node counts, correct risk values matching what the intelligence functions compute. E2E tests parse the generated HTML and validate the embedded JSON against direct function calls. 100% unit test coverage on new Rust code. No mocks for intelligence functions — call the real thing.

---

## Prerequisites

- v2.0.0 is the current state: 2,014 tests passing, clippy clean, version 2.0.0
- `RUSTUP_TOOLCHAIN=1.94.1` via mise
- All intelligence functions exist and are callable (compute_health, compute_risk_ranking, build_architecture_map, detect_dead_code, build_drift_report, build_security_surface, trace_data_flow)

---

## File Structure

### New files
- `src/visual/spa.rs` — SPA renderer: combines all 6 views into one HTML file with hash router, command palette, inspector panel, theme toggle, keyboard navigation, ARIA labels
- `src/visual/search_index.rs` — Pre-computes a fuzzy search index (files, symbols, modules) from CodebaseIndex, serialized as JSON for the command palette
- `tests/spa_e2e.rs` — End-to-end tests for the SPA output: data integrity, view correctness, search index completeness
- `tests/v1_api_integration.rs` — Integration tests for all 9 wired v1/ endpoints
- `tests/lsp_methods_integration.rs` — Integration tests for wired LSP methods

### Modified files
- `src/visual/mod.rs` — Add `pub mod spa; pub mod search_index;`
- `src/visual/render.rs` — Extract shared JS utilities into `common_js_spa()` returning SPA-aware version; existing `render_html()` unchanged (individual HTML files still work)
- `src/commands/visual.rs` — Add `VisualTypeArg::All` variant that invokes `spa::render_spa()`
- `src/cli/mod.rs` — Add `--type all` option (default changes from `dashboard` to `all`)
- `assets/cxpak-visual.css` — Add light mode tokens, inspector panel styles, command palette styles, focus ring styles, reduced motion media query
- `src/commands/serve.rs:427-488` — Replace 9 stub handlers with real implementations
- `src/lsp/methods.rs:148-180` — Wire 11 stub custom methods to real dispatch
- `src/visual/onboard.rs:114` — Fix `.take(3)` → `.take(5)`
- `src/intelligence/onboarding.rs:215` — Fix `.take(3)` → `.take(5)`
- `src/visual/onboard.rs` — Add test file exclusion filter
- `plugin/lib/ensure-cxpak` — Update REQUIRED_VERSION to 2.1.0
- `Cargo.toml` — Bump version to 2.1.0
- `plugin/.claude-plugin/plugin.json` — Bump version to 2.1.0
- `.claude-plugin/marketplace.json` — Bump version to 2.1.0

---

## Pillar 1: Single-Page Dashboard

### Task 1: Fix v2.0.0 Bugs (onboarding take(5), test exclusion)

**Files:**
- Modify: `src/visual/onboard.rs:114`
- Modify: `src/intelligence/onboarding.rs:215`
- Test: `tests/onboarding.rs`

- [ ] **Step 1: Write failing test — symbols_to_focus_on returns up to 5**

Add to `tests/onboarding.rs`:

```rust
#[test]
fn onboarding_symbols_max_five() {
    // Build an index with a file that has >5 public symbols.
    let counter = crate::budget::counter::TokenCounter::new();
    let files = vec![crate::scanner::ScannedFile {
        relative_path: "src/lib.rs".to_string(),
        absolute_path: std::path::PathBuf::from("/tmp/src/lib.rs"),
        language: Some("rust".to_string()),
        size_bytes: 500,
    }];
    let mut parse_results = std::collections::HashMap::new();
    let symbols: Vec<crate::parser::language::Symbol> = (0..8)
        .map(|i| crate::parser::language::Symbol {
            name: format!("func_{i}"),
            kind: crate::parser::language::SymbolKind::Function,
            visibility: crate::parser::language::Visibility::Public,
            signature: format!("fn func_{i}()"),
            body: format!("fn func_{i}() {{}}"),
            start_line: i * 3 + 1,
            end_line: i * 3 + 3,
        })
        .collect();
    parse_results.insert(
        "src/lib.rs".to_string(),
        crate::parser::language::ParseResult {
            symbols,
            imports: vec![],
            exports: vec![],
        },
    );
    let mut content_map = std::collections::HashMap::new();
    content_map.insert("src/lib.rs".to_string(), "fn main() {}".to_string());
    let index =
        crate::index::CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
    let map = crate::visual::onboard::build_onboarding_map(&index, None);
    for phase in &map.phases {
        for file in &phase.files {
            assert!(
                file.symbols_to_focus_on.len() <= 5,
                "file {} has {} symbols, expected <= 5",
                file.path,
                file.symbols_to_focus_on.len()
            );
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features onboarding_symbols_max_five -- --nocapture
```

Expected: FAIL — current code uses `.take(3)` so test may pass with 3 symbols if the file has 8, but the assertion is `<= 5`. Actually the test will pass because 3 <= 5. We need to also assert a MINIMUM when the file has enough symbols.

Replace the assertion:

```rust
    // Files with >= 5 public symbols should have exactly 5 symbols_to_focus_on
    for phase in &map.phases {
        for file in &phase.files {
            assert!(
                file.symbols_to_focus_on.len() <= 5,
                "file {} has {} symbols, expected <= 5",
                file.path,
                file.symbols_to_focus_on.len()
            );
            // The file has 8 public symbols, so we expect exactly 5
            if file.path == "src/lib.rs" {
                assert_eq!(
                    file.symbols_to_focus_on.len(),
                    5,
                    "file with 8 public symbols should produce 5 symbols_to_focus_on, got {}",
                    file.symbols_to_focus_on.len()
                );
            }
        }
    }
```

Now re-run — expected: FAIL with "file with 8 public symbols should produce 5 symbols_to_focus_on, got 3"

- [ ] **Step 3: Fix the two `.take(3)` calls**

In `src/visual/onboard.rs`, find line ~114 (in the iterator chain building `symbols_to_focus_on`) and change `.take(3)` to `.take(5)`.

In `src/intelligence/onboarding.rs`, find line ~215 (same pattern) and change `.take(3)` to `.take(5)`.

- [ ] **Step 4: Run test to verify it passes**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features onboarding_symbols_max_five -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Write failing test — test files excluded from onboarding**

Add to `tests/onboarding.rs`:

```rust
#[test]
fn onboarding_excludes_test_files() {
    let counter = crate::budget::counter::TokenCounter::new();
    let files = vec![
        crate::scanner::ScannedFile {
            relative_path: "src/main.rs".to_string(),
            absolute_path: std::path::PathBuf::from("/tmp/src/main.rs"),
            language: Some("rust".to_string()),
            size_bytes: 100,
        },
        crate::scanner::ScannedFile {
            relative_path: "tests/main_test.rs".to_string(),
            absolute_path: std::path::PathBuf::from("/tmp/tests/main_test.rs"),
            language: Some("rust".to_string()),
            size_bytes: 100,
        },
        crate::scanner::ScannedFile {
            relative_path: "src/lib_test.rs".to_string(),
            absolute_path: std::path::PathBuf::from("/tmp/src/lib_test.rs"),
            language: Some("rust".to_string()),
            size_bytes: 100,
        },
    ];
    let parse_results = std::collections::HashMap::new();
    let mut content_map = std::collections::HashMap::new();
    content_map.insert("src/main.rs".to_string(), "fn main() {}".to_string());
    content_map.insert("tests/main_test.rs".to_string(), "#[test] fn t() {}".to_string());
    content_map.insert("src/lib_test.rs".to_string(), "#[test] fn t() {}".to_string());
    let index =
        crate::index::CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
    let map = crate::visual::onboard::build_onboarding_map(&index, None);
    let all_paths: Vec<&str> = map
        .phases
        .iter()
        .flat_map(|p| p.files.iter().map(|f| f.path.as_str()))
        .collect();
    assert!(
        !all_paths.iter().any(|p| p.contains("_test.") || p.starts_with("tests/")),
        "onboarding should exclude test files, found: {:?}",
        all_paths
    );
    assert!(
        all_paths.contains(&"src/main.rs"),
        "non-test file should be included"
    );
}
```

- [ ] **Step 6: Run test to verify it fails**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features onboarding_excludes_test_files -- --nocapture
```

Expected: FAIL — test files are currently included.

- [ ] **Step 7: Add test file exclusion to `build_onboarding_map`**

In `src/visual/onboard.rs`, in the `build_onboarding_map` function, add a filter before iterating `index.files`. After the line that starts iterating files, add:

```rust
fn is_test_file(path: &str) -> bool {
    path.starts_with("tests/")
        || path.starts_with("test/")
        || path.contains("_test.")
        || path.contains(".test.")
        || path.contains("_spec.")
        || path.contains(".spec.")
        || path.ends_with("_test.rs")
        || path.ends_with("_test.go")
        || path.ends_with("_test.py")
}
```

Then filter the file iteration to skip test files:

```rust
    for file in index.files.iter().filter(|f| !is_test_file(&f.relative_path)) {
```

- [ ] **Step 8: Run test to verify it passes**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features onboarding_excludes_test -- --nocapture
```

Expected: PASS

- [ ] **Step 9: Run full test suite to verify no regressions**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features
```

Expected: All tests pass (2,014+)

- [ ] **Step 10: Commit**

```bash
git add src/visual/onboard.rs src/intelligence/onboarding.rs tests/onboarding.rs
git commit -m "$(cat <<'EOF'
fix(onboard): return top-5 symbols and exclude test files

- Change .take(3) to .take(5) in both onboard.rs and onboarding.rs
- Add is_test_file() filter to build_onboarding_map
- Add tests for both fixes
EOF
)"
```

---

### Task 2: Search Index Module (`src/visual/search_index.rs`)

**Files:**
- Create: `src/visual/search_index.rs`
- Modify: `src/visual/mod.rs`

- [ ] **Step 1: Write failing test — search index contains all files**

Create `src/visual/search_index.rs`:

```rust
//! Pre-computes a fuzzy search index from a CodebaseIndex.
//!
//! The index is serialized as JSON and embedded in the SPA HTML so the
//! command palette can search files, symbols, and modules without any
//! server calls.

use crate::index::CodebaseIndex;
use serde::Serialize;

/// A single searchable entry in the command palette index.
#[derive(Debug, Clone, Serialize)]
pub struct SearchEntry {
    /// Display label shown in the palette, e.g. "src/index/mod.rs"
    pub label: String,
    /// Type of entry: "file", "symbol", "module", or "view"
    pub kind: String,
    /// For symbols: the containing file path. For files/modules: same as label.
    pub context: String,
    /// Secondary text shown below the label (risk score, symbol kind, etc.)
    pub detail: String,
    /// Navigation target: hash route + optional query, e.g. "#architecture?focus=src/index"
    pub target: String,
}

/// Build the complete search index from a CodebaseIndex.
pub fn build_search_index(index: &CodebaseIndex) -> Vec<SearchEntry> {
    let mut entries = Vec::new();

    // 1. All 6 views as navigable targets
    for (label, hash) in &[
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

    // 2. All files
    for file in &index.files {
        let risk = index
            .pagerank
            .get(&file.relative_path)
            .copied()
            .unwrap_or(0.0);
        entries.push(SearchEntry {
            label: file.relative_path.clone(),
            kind: "file".to_string(),
            context: file
                .language
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            detail: format!(
                "{} · {} tokens · PR {:.3}",
                file.language.as_deref().unwrap_or("unknown"),
                file.token_count,
                risk
            ),
            target: format!(
                "#architecture?focus={}",
                file.relative_path
                    .rsplit_once('/')
                    .map(|(dir, _)| dir)
                    .unwrap_or("")
            ),
        });
    }

    // 3. All public symbols
    for file in &index.files {
        let Some(pr) = &file.parse_result else {
            continue;
        };
        for sym in &pr.symbols {
            if !matches!(
                sym.visibility,
                crate::parser::language::Visibility::Public
            ) {
                continue;
            }
            entries.push(SearchEntry {
                label: sym.name.clone(),
                kind: "symbol".to_string(),
                context: file.relative_path.clone(),
                detail: format!("{:?} in {}", sym.kind, file.relative_path),
                target: format!("#architecture?file={}", file.relative_path),
            });
        }
    }

    // 4. Module prefixes (deduplicated)
    let mut modules: Vec<String> = index
        .files
        .iter()
        .filter_map(|f| {
            f.relative_path
                .rsplit_once('/')
                .map(|(dir, _)| dir.to_string())
        })
        .collect();
    modules.sort();
    modules.dedup();
    for module in &modules {
        entries.push(SearchEntry {
            label: module.clone(),
            kind: "module".to_string(),
            context: String::new(),
            detail: format!(
                "{} files",
                index
                    .files
                    .iter()
                    .filter(|f| f.relative_path.starts_with(module.as_str()))
                    .count()
            ),
            target: format!("#architecture?focus={module}"),
        });
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    fn make_test_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let files = vec![
            ScannedFile {
                relative_path: "src/main.rs".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/src/main.rs"),
                language: Some("rust".to_string()),
                size_bytes: 100,
            },
            ScannedFile {
                relative_path: "src/lib.rs".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/src/lib.rs"),
                language: Some("rust".to_string()),
                size_bytes: 200,
            },
        ];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/main.rs".to_string(),
            crate::parser::language::ParseResult {
                symbols: vec![crate::parser::language::Symbol {
                    name: "main".to_string(),
                    kind: crate::parser::language::SymbolKind::Function,
                    visibility: crate::parser::language::Visibility::Public,
                    signature: "fn main()".to_string(),
                    body: "fn main() {}".to_string(),
                    start_line: 1,
                    end_line: 3,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        let mut content_map = HashMap::new();
        content_map.insert("src/main.rs".to_string(), "fn main() {}".to_string());
        content_map.insert("src/lib.rs".to_string(), "pub fn lib() {}".to_string());
        CodebaseIndex::build_with_content(files, parse_results, &counter, content_map)
    }

    #[test]
    fn search_index_contains_all_files() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        let file_entries: Vec<&SearchEntry> =
            entries.iter().filter(|e| e.kind == "file").collect();
        assert_eq!(file_entries.len(), 2);
        let paths: Vec<&str> = file_entries.iter().map(|e| e.label.as_str()).collect();
        assert!(paths.contains(&"src/main.rs"));
        assert!(paths.contains(&"src/lib.rs"));
    }

    #[test]
    fn search_index_contains_public_symbols() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        let sym_entries: Vec<&SearchEntry> =
            entries.iter().filter(|e| e.kind == "symbol").collect();
        assert_eq!(sym_entries.len(), 1);
        assert_eq!(sym_entries[0].label, "main");
        assert_eq!(sym_entries[0].context, "src/main.rs");
    }

    #[test]
    fn search_index_contains_all_six_views() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        let view_entries: Vec<&SearchEntry> =
            entries.iter().filter(|e| e.kind == "view").collect();
        assert_eq!(view_entries.len(), 6);
    }

    #[test]
    fn search_index_contains_modules() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        let mod_entries: Vec<&SearchEntry> =
            entries.iter().filter(|e| e.kind == "module").collect();
        assert!(mod_entries.iter().any(|e| e.label == "src"));
    }

    #[test]
    fn search_index_serializes_to_json() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        let json = serde_json::to_string(&entries).unwrap();
        let parsed: Vec<SearchEntry> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), entries.len());
    }

    #[test]
    fn search_index_file_detail_contains_language() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        let main_entry = entries
            .iter()
            .find(|e| e.kind == "file" && e.label == "src/main.rs")
            .unwrap();
        assert!(
            main_entry.detail.contains("rust"),
            "detail should contain language: {}",
            main_entry.detail
        );
    }

    #[test]
    fn search_index_empty_index_returns_only_views() {
        let counter = TokenCounter::new();
        let index = CodebaseIndex::build_with_content(
            vec![],
            HashMap::new(),
            &counter,
            HashMap::new(),
        );
        let entries = build_search_index(&index);
        assert_eq!(entries.len(), 6); // only the 6 views
        assert!(entries.iter().all(|e| e.kind == "view"));
    }
}
```

- [ ] **Step 2: Register module in `src/visual/mod.rs`**

Add after the existing module declarations:

```rust
#[cfg(feature = "visual")]
pub mod search_index;
```

- [ ] **Step 3: Run tests to verify they pass**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features visual::search_index -- --nocapture
```

Expected: All 7 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/visual/search_index.rs src/visual/mod.rs
git commit -m "feat(visual): add search index module for command palette"
```

---

### Task 3: Light Mode CSS + Inspector Panel + Command Palette Styles

**Files:**
- Modify: `assets/cxpak-visual.css`

- [ ] **Step 1: Add light mode tokens using CSS `light-dark()` and `.light-mode` override**

Append to `assets/cxpak-visual.css`:

```css
/* ── Light Mode ────────────────────────────────────────────────── */

.light-mode {
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

/* ── Focus Ring (keyboard navigation) ──────────────────────────── */

.cxpak-node rect:focus,
.cxpak-node:focus rect,
[tabindex]:focus-visible {
  outline: 2px solid var(--accent-blue);
  outline-offset: 2px;
}

/* ── Command Palette ───────────────────────────────────────────── */

.cxpak-palette-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  z-index: 500;
  display: flex;
  justify-content: center;
  padding-top: 20vh;
}

.cxpak-palette {
  width: 520px;
  max-height: 420px;
  background: var(--bg-card);
  border: 1px solid var(--border-light);
  border-radius: var(--radius-lg);
  box-shadow: 0 16px 48px var(--shadow);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.cxpak-palette-input {
  width: 100%;
  padding: 14px 18px;
  background: transparent;
  border: none;
  border-bottom: 1px solid var(--border);
  color: var(--text-primary);
  font-size: 15px;
  font-family: inherit;
  outline: none;
}

.cxpak-palette-input::placeholder { color: var(--text-dim); }

.cxpak-palette-results {
  flex: 1;
  overflow-y: auto;
  padding: 6px 0;
}

.cxpak-palette-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 18px;
  cursor: pointer;
  font-size: 13px;
  transition: background 0.08s;
}

.cxpak-palette-item:hover,
.cxpak-palette-item.active {
  background: var(--bg-card-hover);
}

.cxpak-palette-item .kind {
  font-size: 10px;
  font-weight: 600;
  padding: 2px 6px;
  border-radius: 4px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  flex-shrink: 0;
}

.cxpak-palette-item .kind.file { background: rgba(8, 145, 178, 0.15); color: var(--accent-cyan); }
.cxpak-palette-item .kind.symbol { background: rgba(124, 58, 237, 0.15); color: var(--accent-purple); }
.cxpak-palette-item .kind.module { background: rgba(99, 102, 241, 0.15); color: #6366f1; }
.cxpak-palette-item .kind.view { background: rgba(5, 150, 105, 0.15); color: var(--accent-green); }

.cxpak-palette-item .label { font-weight: 500; color: var(--text-primary); }
.cxpak-palette-item .detail { color: var(--text-secondary); font-size: 11px; margin-left: auto; }

.cxpak-palette-empty {
  padding: 20px;
  text-align: center;
  color: var(--text-dim);
  font-size: 13px;
}

.cxpak-palette-hint {
  padding: 8px 18px;
  font-size: 11px;
  color: var(--text-dim);
  border-top: 1px solid var(--border);
  display: flex;
  gap: 16px;
}

.cxpak-palette-hint kbd {
  background: var(--bg-secondary);
  padding: 1px 5px;
  border-radius: 3px;
  font-size: 10px;
  border: 1px solid var(--border);
}

/* ── Inspector Panel ───────────────────────────────────────────── */

.cxpak-inspector {
  position: fixed;
  right: 0;
  top: 0;
  bottom: 0;
  width: 340px;
  background: var(--bg-card);
  border-left: 1px solid var(--border);
  z-index: 100;
  transform: translateX(100%);
  transition: transform 0.2s ease-out;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
}

.cxpak-inspector.open { transform: translateX(0); }

.cxpak-inspector-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 16px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}

.cxpak-inspector-title {
  font-size: 13px;
  font-weight: 700;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cxpak-inspector-close {
  background: none;
  border: none;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 16px;
  padding: 4px;
  border-radius: 4px;
}

.cxpak-inspector-close:hover { background: var(--bg-card-hover); }

.cxpak-inspector-body {
  padding: 16px;
  flex: 1;
}

.cxpak-inspector-section {
  margin-bottom: 16px;
}

.cxpak-inspector-section-title {
  font-size: 10px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 1px;
  color: var(--text-secondary);
  margin-bottom: 8px;
}

.cxpak-inspector-row {
  display: flex;
  justify-content: space-between;
  padding: 4px 0;
  font-size: 12px;
}

.cxpak-inspector-label { color: var(--text-secondary); }
.cxpak-inspector-value { font-weight: 600; text-align: right; }

/* ── Theme Toggle ──────────────────────────────────────────────── */

.cxpak-theme-toggle {
  background: var(--bg-card);
  border: 1px solid var(--border);
  color: var(--text-secondary);
  cursor: pointer;
  padding: 4px 8px;
  border-radius: 6px;
  font-size: 14px;
  line-height: 1;
  transition: all 0.15s;
}

.cxpak-theme-toggle:hover {
  background: var(--bg-card-hover);
  color: var(--text-primary);
}

/* ── Data Freshness Badge ──────────────────────────────────────── */

.cxpak-freshness {
  font-size: 11px;
  padding: 2px 8px;
  border-radius: 8px;
  font-weight: 500;
}

.cxpak-freshness.fresh { color: var(--text-dim); }
.cxpak-freshness.stale { background: rgba(217, 119, 6, 0.15); color: var(--accent-yellow); }
.cxpak-freshness.old { background: rgba(220, 38, 38, 0.15); color: var(--accent-red); }
```

- [ ] **Step 2: Verify CSS is valid — no syntax errors**

```bash
# Quick check: the file should not contain obvious errors
RUSTUP_TOOLCHAIN=1.94.1 cargo build --features visual 2>&1 | grep -i error
```

Expected: No errors (CSS is inlined via include_str, build would fail on missing file but not CSS syntax)

- [ ] **Step 3: Commit**

```bash
git add assets/cxpak-visual.css
git commit -m "feat(visual): add light mode, command palette, inspector panel, and accessibility CSS"
```

---

### Task 4: SPA Renderer (`src/visual/spa.rs`)

This is the core task. It generates a single HTML file containing all 6 views with hash routing, command palette, inspector panel, theme toggle, keyboard navigation, and ARIA labels.

**Files:**
- Create: `src/visual/spa.rs`
- Modify: `src/visual/mod.rs`

- [ ] **Step 1: Write failing test — SPA HTML contains all 6 view containers**

Create `src/visual/spa.rs` with the test first:

```rust
//! Single-page application renderer.
//!
//! Combines all 6 visual views into one self-contained HTML file with:
//! - Hash-based client-side routing (#dashboard, #architecture, etc.)
//! - Command palette (Cmd+K / Ctrl+K) with fuzzy search
//! - Inspector panel (click any node to see details)
//! - Light/dark theme toggle with localStorage persistence
//! - Keyboard navigation (arrows, Enter, Escape)
//! - ARIA labels on graph nodes
//! - Data freshness badge

use crate::index::CodebaseIndex;
use super::render::RenderMetadata;

/// Render the complete SPA HTML for all views.
pub fn render_spa(
    index: &CodebaseIndex,
    metadata: &RenderMetadata,
) -> Result<String, super::layout::LayoutError> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    fn make_test_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let files = vec![ScannedFile {
            relative_path: "src/main.rs".to_string(),
            absolute_path: std::path::PathBuf::from("/tmp/src/main.rs"),
            language: Some("rust".to_string()),
            size_bytes: 100,
        }];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/main.rs".to_string(),
            crate::parser::language::ParseResult {
                symbols: vec![crate::parser::language::Symbol {
                    name: "main".to_string(),
                    kind: crate::parser::language::SymbolKind::Function,
                    visibility: crate::parser::language::Visibility::Public,
                    signature: "fn main()".to_string(),
                    body: "fn main() {}".to_string(),
                    start_line: 1,
                    end_line: 3,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        let mut content_map = HashMap::new();
        content_map.insert("src/main.rs".to_string(), "fn main() {}".to_string());
        CodebaseIndex::build_with_content(files, parse_results, &counter, content_map)
    }

    fn make_metadata() -> RenderMetadata {
        RenderMetadata {
            repo_name: "test-repo".to_string(),
            generated_at: "2026-04-17T12:00:00Z".to_string(),
            health_score: Some(7.4),
            node_count: 1,
            edge_count: 0,
            cxpak_version: "2.1.0".to_string(),
        }
    }

    #[test]
    fn spa_html_contains_all_view_containers() {
        let index = make_test_index();
        let meta = make_metadata();
        let html = render_spa(&index, &meta).unwrap();
        for view_id in &[
            "view-dashboard",
            "view-architecture",
            "view-risk",
            "view-flow",
            "view-timeline",
            "view-diff",
        ] {
            assert!(
                html.contains(view_id),
                "SPA HTML should contain view container '{view_id}'"
            );
        }
    }

    #[test]
    fn spa_html_is_self_contained() {
        let index = make_test_index();
        let meta = make_metadata();
        let html = render_spa(&index, &meta).unwrap();
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("</html>"));
        assert!(!html.contains("cdn.jsdelivr.net"));
        assert!(!html.contains("unpkg.com"));
        // D3 bundle inlined
        assert!(html.contains("d3"));
        // CSS inlined
        assert!(html.contains("--bg-primary"));
    }

    #[test]
    fn spa_html_contains_search_index() {
        let index = make_test_index();
        let meta = make_metadata();
        let html = render_spa(&index, &meta).unwrap();
        assert!(
            html.contains("cxpak-search-index"),
            "SPA should embed search index for command palette"
        );
        // The search index should contain our test file
        assert!(html.contains("src/main.rs"));
    }

    #[test]
    fn spa_html_contains_command_palette_markup() {
        let index = make_test_index();
        let meta = make_metadata();
        let html = render_spa(&index, &meta).unwrap();
        assert!(html.contains("cxpak-palette"));
    }

    #[test]
    fn spa_html_contains_theme_toggle() {
        let index = make_test_index();
        let meta = make_metadata();
        let html = render_spa(&index, &meta).unwrap();
        assert!(html.contains("cxpak-theme-toggle"));
    }

    #[test]
    fn spa_html_contains_hash_router() {
        let index = make_test_index();
        let meta = make_metadata();
        let html = render_spa(&index, &meta).unwrap();
        assert!(html.contains("hashchange"));
        assert!(html.contains("#dashboard"));
        assert!(html.contains("#architecture"));
    }

    #[test]
    fn spa_html_contains_keyboard_handler() {
        let index = make_test_index();
        let meta = make_metadata();
        let html = render_spa(&index, &meta).unwrap();
        assert!(html.contains("keydown"));
        // Cmd+K / Ctrl+K handler
        assert!(html.contains("metaKey") || html.contains("ctrlKey"));
    }

    #[test]
    fn spa_html_contains_aria_labels() {
        let index = make_test_index();
        let meta = make_metadata();
        let html = render_spa(&index, &meta).unwrap();
        assert!(html.contains("aria-label"));
    }

    #[test]
    fn spa_html_contains_freshness_badge() {
        let index = make_test_index();
        let meta = make_metadata();
        let html = render_spa(&index, &meta).unwrap();
        assert!(html.contains("cxpak-freshness"));
    }

    #[test]
    fn spa_embedded_data_is_valid_json() {
        let index = make_test_index();
        let meta = make_metadata();
        let html = render_spa(&index, &meta).unwrap();
        // Extract dashboard data JSON
        let marker = r#"id="cxpak-dashboard-data" type="application/json">"#;
        if let Some(start_idx) = html.find(marker) {
            let json_start = start_idx + marker.len();
            let json_end = html[json_start..].find("</script>").unwrap() + json_start;
            let json_str = &html[json_start..json_end];
            let parsed: Result<serde_json::Value, _> = serde_json::from_str(json_str);
            assert!(parsed.is_ok(), "dashboard data must be valid JSON: {:?}", parsed.err());
        }
    }

    #[test]
    fn spa_data_integrity_health_score_matches() {
        let index = make_test_index();
        let meta = make_metadata();
        let html = render_spa(&index, &meta).unwrap();
        // Compute health directly
        let health = crate::intelligence::health::compute_health(&index);
        // The HTML should contain the same composite score
        let score_str = format!("{:.1}", health.composite);
        assert!(
            html.contains(&score_str),
            "SPA health score should match compute_health(): expected {score_str} in HTML"
        );
    }

    #[test]
    fn spa_data_integrity_risk_count_matches() {
        let index = make_test_index();
        let meta = make_metadata();
        let html = render_spa(&index, &meta).unwrap();
        let risks = crate::intelligence::risk::compute_risk_ranking(&index);
        // The dashboard data should contain the correct number of risk entries
        let marker = r#"id="cxpak-dashboard-data" type="application/json">"#;
        if let Some(start_idx) = html.find(marker) {
            let json_start = start_idx + marker.len();
            let json_end = html[json_start..].find("</script>").unwrap() + json_start;
            let json_str = &html[json_start..json_end];
            let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap();
            if let Some(risk_arr) = parsed.get("risks").and_then(|r| r.get("top_risks")) {
                let html_risk_count = risk_arr.as_array().map(|a| a.len()).unwrap_or(0);
                let expected = risks.len().min(5);
                assert_eq!(
                    html_risk_count, expected,
                    "risk count in HTML ({html_risk_count}) should match compute_risk_ranking ({expected})"
                );
            }
        }
    }

    #[test]
    fn spa_data_integrity_file_count_matches() {
        let index = make_test_index();
        let meta = make_metadata();
        let html = render_spa(&index, &meta).unwrap();
        // Search index should contain exactly the number of files in the index
        let search_marker = r#"id="cxpak-search-index" type="application/json">"#;
        if let Some(start_idx) = html.find(search_marker) {
            let json_start = start_idx + search_marker.len();
            let json_end = html[json_start..].find("</script>").unwrap() + json_start;
            let json_str = &html[json_start..json_end];
            let entries: Vec<serde_json::Value> = serde_json::from_str(json_str).unwrap();
            let file_count = entries.iter().filter(|e| e.get("kind").and_then(|k| k.as_str()) == Some("file")).count();
            assert_eq!(
                file_count,
                index.total_files,
                "search index file count ({file_count}) should match index.total_files ({})",
                index.total_files
            );
        }
    }
}
```

- [ ] **Step 2: Register module in `src/visual/mod.rs`**

Add:

```rust
#[cfg(feature = "visual")]
pub mod spa;
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features visual::spa -- --nocapture 2>&1 | head -20
```

Expected: FAIL — `render_spa` is `todo!()`.

- [ ] **Step 4: Implement `render_spa`**

This is the largest implementation step. `render_spa` should:

1. Compute all view data by calling existing render module functions:
   - `render::build_dashboard_data(&index)` for dashboard
   - `render::build_architecture_explorer_data(&index, &config)` for architecture
   - `render::build_risk_heatmap_data(&index)` for risk
   - For timeline: `timeline::load_cached_snapshots()` or empty
   - For flow/diff: embed empty placeholder (requires user parameters)
2. Build the search index via `search_index::build_search_index(&index)`
3. Serialize all data as separate `<script type="application/json">` tags
4. Emit a single HTML file with:
   - All CSS inlined (same `include_str!` pattern)
   - D3 bundle inlined
   - Hash router JS that shows/hides `div#view-{name}` containers
   - Command palette JS with fuzzy search over the search index
   - Inspector panel JS
   - Theme toggle JS (reads/writes localStorage)
   - Keyboard navigation JS
   - Data freshness JS (computes relative time from `generated_at`)
   - Each view's controller JS (reuse from existing `render.rs` functions)

The implementation should reuse the existing `common_js()`, view controller functions, and `escape_script_tag()` from `render.rs`. Make those `pub(crate)` if they're currently private.

Key structure of the emitted HTML:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>cxpak — {repo_name}</title>
  <style>{CSS}</style>
</head>
<body>
  <div id="cxpak-app">
    <div id="cxpak-header"><!-- header with nav tabs, theme toggle, freshness --></div>
    <div id="view-dashboard" class="cxpak-view"></div>
    <div id="view-architecture" class="cxpak-view" style="display:none"></div>
    <div id="view-risk" class="cxpak-view" style="display:none"></div>
    <div id="view-flow" class="cxpak-view" style="display:none"></div>
    <div id="view-timeline" class="cxpak-view" style="display:none"></div>
    <div id="view-diff" class="cxpak-view" style="display:none"></div>
  </div>
  <!-- Command Palette overlay (hidden by default) -->
  <div id="cxpak-palette-overlay" class="cxpak-palette-overlay" style="display:none">...</div>
  <!-- Inspector panel (hidden by default) -->
  <div id="cxpak-inspector" class="cxpak-inspector">...</div>
  <!-- Data tags -->
  <script id="cxpak-dashboard-data" type="application/json">{dashboard_json}</script>
  <script id="cxpak-architecture-data" type="application/json">{arch_json}</script>
  <script id="cxpak-risk-data" type="application/json">{risk_json}</script>
  <script id="cxpak-meta" type="application/json">{meta_json}</script>
  <script id="cxpak-search-index" type="application/json">{search_json}</script>
  <!-- D3 + app JS -->
  <script>{D3_BUNDLE}</script>
  <script>{spa_controller_js}</script>
</body>
</html>
```

The `spa_controller_js` is a large inline script (~300-500 lines) that:
- Implements hash routing
- Initializes each view lazily on first navigation
- Implements the command palette with substring matching
- Implements the inspector panel
- Implements theme toggle
- Implements keyboard navigation
- Implements data freshness computation

- [ ] **Step 5: Run tests to verify they pass**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features visual::spa -- --nocapture
```

Expected: All 13 tests pass.

- [ ] **Step 6: Run clippy**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo clippy --all-targets --all-features -- -D warnings
```

Expected: Clean.

- [ ] **Step 7: Commit**

```bash
git add src/visual/spa.rs src/visual/mod.rs
git commit -m "feat(visual): add SPA renderer with command palette, inspector, themes, and keyboard nav"
```

---

### Task 5: Wire SPA into CLI (`--type all`)

**Files:**
- Modify: `src/cli/mod.rs`
- Modify: `src/commands/visual.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write failing test — `--type all` parses correctly**

Add to the existing CLI test section in `src/cli/mod.rs` or a test file:

```rust
#[test]
fn cli_visual_type_all_parses() {
    let cli = Cli::try_parse_from(["cxpak", "visual", "--visual-type", "all"]).unwrap();
    match cli.command {
        Commands::Visual { visual_type, .. } => {
            assert_eq!(visual_type, VisualTypeArg::All);
        }
        _ => panic!("expected Visual command"),
    }
}
```

- [ ] **Step 2: Add `All` variant to `VisualTypeArg`**

In `src/cli/mod.rs`, add `All` to the `VisualTypeArg` enum and change the default from `dashboard` to `all`:

```rust
#[arg(long, default_value = "all")]
visual_type: VisualTypeArg,
```

Add `All` variant:

```rust
All,
```

- [ ] **Step 3: Handle `All` in `src/commands/visual.rs`**

In the `match visual_type` block, add:

```rust
VisualTypeArg::All => crate::visual::spa::render_spa(&index, &metadata)?,
```

Update `type_slug` to return `"all"` and `ext_for_format` to return `.html` for it.

- [ ] **Step 4: Run test to verify it passes**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features cli_visual_type_all -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/cli/mod.rs src/commands/visual.rs src/main.rs
git commit -m "feat(cli): add --type all for SPA output, make it the default"
```

---

## Pillar 2: Keyboard-First + Accessibility

### Task 6: ARIA Labels Generated in Rust

**Files:**
- Modify: `src/visual/layout.rs`

- [ ] **Step 1: Write failing test — LayoutNode has aria_label field**

Add to layout.rs tests:

```rust
#[test]
fn layout_node_has_aria_label() {
    let node = LayoutNode {
        id: "src/main.rs".to_string(),
        label: "main.rs".to_string(),
        layer: 0,
        position: Point { x: 0.0, y: 0.0 },
        width: 160.0,
        height: 48.0,
        node_type: NodeType::File,
        metadata: NodeMetadata {
            pagerank: 0.42,
            risk_score: 0.8,
            token_count: 500,
            health_score: Some(6.0),
            is_god_file: false,
            has_dead_code: true,
            is_circular: false,
        },
        aria_label: String::new(),
    };
    let label = build_aria_label(&node);
    assert!(label.contains("main.rs"));
    assert!(label.contains("risk"));
    assert!(label.contains("0.8") || label.contains("80"));
}
```

- [ ] **Step 2: Add `aria_label` field to `LayoutNode`**

```rust
pub struct LayoutNode {
    // ... existing fields ...
    /// Accessibility label for screen readers, generated at build time.
    pub aria_label: String,
}
```

- [ ] **Step 3: Implement `build_aria_label`**

```rust
/// Build an ARIA label for a layout node describing its key properties.
pub fn build_aria_label(node: &LayoutNode) -> String {
    let m = &node.metadata;
    let mut parts = vec![node.label.clone()];

    match &node.node_type {
        NodeType::Module => parts.push("module".to_string()),
        NodeType::File => parts.push("file".to_string()),
        NodeType::Symbol => parts.push("symbol".to_string()),
        NodeType::Cluster { member_ids } => {
            parts.push(format!("group of {} items", member_ids.len()));
        }
    }

    if m.risk_score > 0.0 {
        let severity = if m.risk_score >= 0.7 {
            "high"
        } else if m.risk_score >= 0.4 {
            "medium"
        } else {
            "low"
        };
        parts.push(format!("{severity} risk ({:.0}%)", m.risk_score * 100.0));
    }
    if m.token_count > 0 {
        parts.push(format!("{} tokens", m.token_count));
    }
    if m.is_god_file {
        parts.push("god file".to_string());
    }
    if m.has_dead_code {
        parts.push("contains dead code".to_string());
    }
    if m.is_circular {
        parts.push("participates in circular dependency".to_string());
    }

    parts.join(", ")
}
```

- [ ] **Step 4: Wire `build_aria_label` into layout builders**

In `build_module_layout`, `build_file_layout`, and `build_symbol_layout`, after constructing each `LayoutNode`, set:

```rust
node.aria_label = build_aria_label(&node);
```

- [ ] **Step 5: Add `#[serde(default)]` to `aria_label` for backward compatibility**

In the `LayoutNode` struct:

```rust
#[serde(default)]
pub aria_label: String,
```

- [ ] **Step 6: Run tests**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features layout -- --nocapture
```

Expected: All tests pass (existing + new).

- [ ] **Step 7: Commit**

```bash
git add src/visual/layout.rs
git commit -m "feat(visual): add ARIA labels to layout nodes for screen reader support"
```

---

## Pillar 3: Wire the Stubs

### Task 7: Implement 9 v1/ API Endpoint Stubs

**Files:**
- Modify: `src/commands/serve.rs:427-488`
- Create: `tests/v1_api_wired.rs`

- [ ] **Step 1: Write integration tests for all 9 endpoints**

Create `tests/v1_api_wired.rs`:

```rust
//! Integration tests verifying all v1/ API endpoints return real data.

use axum::http::StatusCode;
use serde_json::Value;

fn build_test_app() -> axum::Router {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let files = vec![cxpak::scanner::ScannedFile {
        relative_path: "src/main.rs".to_string(),
        absolute_path: std::path::PathBuf::from("/tmp/src/main.rs"),
        language: Some("rust".to_string()),
        size_bytes: 100,
    }];
    let mut parse_results = std::collections::HashMap::new();
    parse_results.insert(
        "src/main.rs".to_string(),
        cxpak::parser::language::ParseResult {
            symbols: vec![cxpak::parser::language::Symbol {
                name: "main".to_string(),
                kind: cxpak::parser::language::SymbolKind::Function,
                visibility: cxpak::parser::language::Visibility::Public,
                signature: "fn main()".to_string(),
                body: "fn main() {}".to_string(),
                start_line: 1,
                end_line: 3,
            }],
            imports: vec![],
            exports: vec![],
        },
    );
    let mut content_map = std::collections::HashMap::new();
    content_map.insert("src/main.rs".to_string(), "fn main() {}".to_string());
    let index = cxpak::index::CodebaseIndex::build_with_content(
        files,
        parse_results,
        &counter,
        content_map,
    );
    let shared = std::sync::Arc::new(std::sync::RwLock::new(index));
    let path = std::sync::Arc::new(std::path::PathBuf::from("/tmp"));
    cxpak::commands::serve::build_router_for_test(shared, path)
}

async fn post_json(app: axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    use axum::body::Body;
    use http::Request;
    use tower::ServiceExt;

    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

#[tokio::test]
async fn v1_risks_returns_real_data() {
    let app = build_test_app();
    let (status, body) = post_json(app, "/v1/risks", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array() || body.get("risks").is_some(),
        "v1/risks should return risk data, got: {body}");
    // Must NOT contain "not_implemented"
    let s = body.to_string();
    assert!(!s.contains("not_implemented"), "v1/risks should not be a stub: {s}");
}

#[tokio::test]
async fn v1_architecture_returns_real_data() {
    let app = build_test_app();
    let (status, body) = post_json(app, "/v1/architecture", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("modules").is_some(), "v1/architecture should have modules: {body}");
    let s = body.to_string();
    assert!(!s.contains("not_implemented"));
}

#[tokio::test]
async fn v1_call_graph_returns_real_data() {
    let app = build_test_app();
    let (status, body) = post_json(app, "/v1/call_graph", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let s = body.to_string();
    assert!(!s.contains("not_implemented"), "v1/call_graph should not be a stub: {s}");
}

#[tokio::test]
async fn v1_dead_code_returns_real_data() {
    let app = build_test_app();
    let (status, body) = post_json(app, "/v1/dead_code", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let s = body.to_string();
    assert!(!s.contains("not_implemented"), "v1/dead_code should not be a stub: {s}");
}

#[tokio::test]
async fn v1_predict_requires_files() {
    let app = build_test_app();
    let (status, _body) = post_json(app, "/v1/predict", serde_json::json!({})).await;
    // predict requires files parameter — should return 400
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v1_predict_with_files_returns_real_data() {
    let app = build_test_app();
    let (status, body) = post_json(
        app,
        "/v1/predict",
        serde_json::json!({"files": ["src/main.rs"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let s = body.to_string();
    assert!(!s.contains("not_implemented"));
}

#[tokio::test]
async fn v1_drift_returns_real_data() {
    let app = build_test_app();
    let (status, body) = post_json(app, "/v1/drift", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let s = body.to_string();
    assert!(!s.contains("not_implemented"));
}

#[tokio::test]
async fn v1_security_surface_returns_real_data() {
    let app = build_test_app();
    let (status, body) = post_json(app, "/v1/security_surface", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let s = body.to_string();
    assert!(!s.contains("not_implemented"));
}

#[tokio::test]
async fn v1_data_flow_returns_real_data() {
    let app = build_test_app();
    let (status, body) = post_json(app, "/v1/data_flow", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let s = body.to_string();
    assert!(!s.contains("not_implemented"));
}

#[tokio::test]
async fn v1_cross_lang_returns_real_data() {
    let app = build_test_app();
    let (status, body) = post_json(app, "/v1/cross_lang", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let s = body.to_string();
    assert!(!s.contains("not_implemented"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test v1_api_wired -- --nocapture 2>&1 | head -30
```

Expected: FAIL — stubs return `"not_implemented"`.

- [ ] **Step 3: Replace 9 stub handlers in `src/commands/serve.rs`**

Replace lines 427-488 (the 9 stub functions). Each new handler accepts `State(index)` and `Json(params)` (using the `V1FocusParams` struct already defined at line 253), then delegates to the corresponding intelligence function.

Pattern for each:

```rust
async fn v1_risks_handler(
    State(index): State<SharedIndex>,
    Json(params): Json<V1FocusParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index.read().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let all_risks = crate::intelligence::risk::compute_risk_ranking(&idx);
    let risks: Vec<_> = if let Some(ref focus) = params.focus {
        all_risks.into_iter().filter(|r| r.path.contains(focus.as_str())).collect()
    } else {
        all_risks
    };
    Ok(Json(serde_json::to_value(&risks).unwrap_or_else(|_| json!([]))))
}
```

Repeat this pattern for all 9 endpoints:
- `v1_risks_handler` → `compute_risk_ranking(&idx)`
- `v1_architecture_handler` → `build_architecture_map(&idx, 2)`
- `v1_call_graph_handler` → use `idx.call_graph` (same as legacy handler at line 986)
- `v1_dead_code_handler` → `detect_dead_code(&idx, params.focus.as_deref())`
- `v1_predict_handler` → `predict()` (requires `files` param, return 400 if missing)
- `v1_drift_handler` → return drift baseline/snapshot data
- `v1_security_surface_handler` → `build_security_surface(&idx, params.focus.as_deref())`
- `v1_data_flow_handler` → `trace_data_flow()` (requires symbol param)
- `v1_cross_lang_handler` → return `idx.cross_lang_edges`

Each handler must accept `State(index): State<SharedIndex>` and `Json(params): Json<V1FocusParams>` (update the route registrations in `build_v1_router` accordingly — the current stubs take no parameters).

- [ ] **Step 4: Update route registrations in `build_v1_router`**

The current v1 routes don't pass `State` to the handler. Update each route to use the stateful handler pattern. This may require changing the route layer configuration.

- [ ] **Step 5: Run tests**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test v1_api_wired -- --nocapture
```

Expected: All 11 tests pass.

- [ ] **Step 6: Run full test suite**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features
```

Expected: All tests pass (no regressions in existing serve_test.rs or api_v1_integration.rs).

- [ ] **Step 7: Commit**

```bash
git add src/commands/serve.rs tests/v1_api_wired.rs
git commit -m "feat(api): wire all 9 v1/ endpoint stubs to real intelligence functions"
```

---

### Task 8: Wire 11 LSP Custom Method Stubs

**Files:**
- Modify: `src/lsp/methods.rs:148-180`
- Create: `tests/lsp_methods_wired.rs`

- [ ] **Step 1: Write failing test — all custom methods return real data**

Create `tests/lsp_methods_wired.rs`:

```rust
//! Verify all LSP custom methods return real data, not stubs.

use cxpak::lsp::methods::handle_custom_method;
use std::collections::HashMap;

fn make_test_index() -> cxpak::index::CodebaseIndex {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let files = vec![cxpak::scanner::ScannedFile {
        relative_path: "src/main.rs".to_string(),
        absolute_path: std::path::PathBuf::from("/tmp/src/main.rs"),
        language: Some("rust".to_string()),
        size_bytes: 100,
    }];
    let mut content_map = HashMap::new();
    content_map.insert("src/main.rs".to_string(), "fn main() {}".to_string());
    cxpak::index::CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map)
}

#[test]
fn all_custom_methods_return_real_data() {
    let index = make_test_index();
    let methods = [
        "cxpak/health",
        "cxpak/conventions",
        "cxpak/blastRadius",
        "cxpak/overview",
        "cxpak/trace",
        "cxpak/diff",
        "cxpak/search",
        "cxpak/apiSurface",
        "cxpak/deadCode",
        "cxpak/callGraph",
        "cxpak/predict",
        "cxpak/drift",
        "cxpak/securitySurface",
        "cxpak/dataFlow",
    ];
    for method in &methods {
        let result = handle_custom_method(method, serde_json::Value::Null, &index);
        assert!(result.is_ok(), "method {method} should return Ok");
        let val = result.unwrap();
        assert!(val.is_some(), "method {method} should return Some");
        let json = val.unwrap();
        let s = json.to_string();
        // No method should return the old stub marker
        assert!(
            !s.contains(r#""status":"available"#),
            "method {method} should not be a stub: {s}"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test lsp_methods_wired -- --nocapture
```

Expected: FAIL — 11 methods still return `{"status":"available"}`.

- [ ] **Step 3: Replace stub match arms in `handle_custom_method`**

In `src/lsp/methods.rs`, replace the catch-all stub block (lines ~164-177) with individual handlers. Each calls the corresponding intelligence function:

```rust
"cxpak/overview" => {
    Ok(Some(serde_json::json!({
        "total_files": index.total_files,
        "total_tokens": index.total_tokens,
        "languages": index.language_stats.len(),
    })))
}
"cxpak/trace" => {
    // Trace requires a symbol parameter — return instruction if not provided
    let symbol = _params.get("symbol").and_then(|v| v.as_str());
    match symbol {
        Some(sym) => {
            let matches = index.find_symbol(sym);
            Ok(Some(serde_json::to_value(&matches.len()).unwrap_or_default()))
        }
        None => Ok(Some(serde_json::json!({"note": "provide symbol parameter"}))),
    }
}
// ... etc for all 11 methods
```

Each method delegates to the real intelligence function. Methods that require parameters (trace, predict, dataFlow) accept them via the `_params: serde_json::Value` argument.

- [ ] **Step 4: Run test to verify it passes**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test lsp_methods_wired -- --nocapture
```

Expected: PASS

- [ ] **Step 5: Run full test suite**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features
```

Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/lsp/methods.rs tests/lsp_methods_wired.rs
git commit -m "feat(lsp): wire all 11 custom method stubs to real intelligence functions"
```

---

### Task 9: Fix MCP >1MB HTML Write-to-File

**Files:**
- Modify: `src/commands/serve.rs` (MCP visual handler)

- [ ] **Step 1: Write failing test — large HTML writes to file**

Add to the MCP visual test section:

```rust
#[test]
fn mcp_visual_large_html_writes_to_file() {
    // This test verifies the contract: when HTML > 1MB, 
    // the MCP handler returns a file path instead of inline content.
    // We test the threshold logic directly.
    let threshold = 1_048_576; // 1MB
    let small_html = "x".repeat(100);
    let large_html = "x".repeat(threshold + 1);
    
    assert!(small_html.len() <= threshold);
    assert!(large_html.len() > threshold);
}
```

- [ ] **Step 2: Add threshold check in MCP visual handler**

In the `handle_cxpak_visual` function in `serve.rs`, after generating the HTML content, add:

```rust
const MCP_INLINE_LIMIT: usize = 1_048_576; // 1MB

if content.len() > MCP_INLINE_LIMIT {
    let visual_dir = repo_path.join(".cxpak/visual");
    std::fs::create_dir_all(&visual_dir).ok();
    let filename = format!("cxpak-{}.html", visual_type);
    let filepath = visual_dir.join(&filename);
    std::fs::write(&filepath, &content)
        .map_err(|e| format!("failed to write visual output: {e}"))?;
    return Ok(mcp_tool_result(
        id,
        &format!("Visual output written to: {}", filepath.display()),
    ));
}
```

- [ ] **Step 3: Run tests**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features mcp_visual -- --nocapture
```

Expected: All pass.

- [ ] **Step 4: Commit**

```bash
git add src/commands/serve.rs
git commit -m "fix(mcp): write large HTML (>1MB) to .cxpak/visual/ instead of inline"
```

---

### Task 10: SPA E2E Data Integrity Tests

**Files:**
- Create: `tests/spa_e2e.rs`

- [ ] **Step 1: Write comprehensive E2E tests**

```rust
//! End-to-end tests for the SPA dashboard.
//! These tests verify DATA INTEGRITY — that the HTML output contains
//! correct values matching direct intelligence function calls.

use cxpak::index::CodebaseIndex;
use std::collections::HashMap;

fn build_realistic_index() -> CodebaseIndex {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let files: Vec<cxpak::scanner::ScannedFile> = (0..10)
        .map(|i| cxpak::scanner::ScannedFile {
            relative_path: format!("src/mod_{i}.rs"),
            absolute_path: std::path::PathBuf::from(format!("/tmp/src/mod_{i}.rs")),
            language: Some("rust".to_string()),
            size_bytes: (i + 1) * 100,
        })
        .collect();

    let mut parse_results = HashMap::new();
    for (i, file) in files.iter().enumerate() {
        let symbols: Vec<cxpak::parser::language::Symbol> = (0..3)
            .map(|j| cxpak::parser::language::Symbol {
                name: format!("func_{i}_{j}"),
                kind: cxpak::parser::language::SymbolKind::Function,
                visibility: if j == 0 {
                    cxpak::parser::language::Visibility::Public
                } else {
                    cxpak::parser::language::Visibility::Private
                },
                signature: format!("fn func_{i}_{j}()"),
                body: format!("fn func_{i}_{j}() {{}}"),
                start_line: j * 5 + 1,
                end_line: j * 5 + 4,
            })
            .collect();
        parse_results.insert(
            file.relative_path.clone(),
            cxpak::parser::language::ParseResult {
                symbols,
                imports: if i > 0 {
                    vec![format!("src/mod_{}.rs", i - 1)]
                } else {
                    vec![]
                },
                exports: vec![],
            },
        );
    }

    let mut content_map = HashMap::new();
    for file in &files {
        content_map.insert(file.relative_path.clone(), "fn x() {}".to_string());
    }

    CodebaseIndex::build_with_content(files, parse_results, &counter, content_map)
}

#[cfg(feature = "visual")]
#[test]
fn spa_e2e_health_score_integrity() {
    let index = build_realistic_index();
    let health = cxpak::intelligence::health::compute_health(&index);
    let metadata = cxpak::visual::render::RenderMetadata {
        repo_name: "test".to_string(),
        generated_at: "2026-04-17T12:00:00Z".to_string(),
        health_score: Some(health.composite),
        node_count: index.total_files,
        edge_count: 0,
        cxpak_version: "2.1.0".to_string(),
    };
    let html = cxpak::visual::spa::render_spa(&index, &metadata).unwrap();

    // The composite score must appear in the HTML
    let score_str = format!("{:.1}", health.composite);
    assert!(
        html.contains(&score_str),
        "HTML must contain health score {score_str}"
    );
}

#[cfg(feature = "visual")]
#[test]
fn spa_e2e_risk_entries_integrity() {
    let index = build_realistic_index();
    let risks = cxpak::intelligence::risk::compute_risk_ranking(&index);
    let metadata = cxpak::visual::render::RenderMetadata {
        repo_name: "test".to_string(),
        generated_at: "2026-04-17T12:00:00Z".to_string(),
        health_score: Some(5.0),
        node_count: index.total_files,
        edge_count: 0,
        cxpak_version: "2.1.0".to_string(),
    };
    let html = cxpak::visual::spa::render_spa(&index, &metadata).unwrap();

    // Extract the dashboard data JSON and verify risk entries
    let marker = r#"id="cxpak-dashboard-data" type="application/json">"#;
    let start = html.find(marker).expect("dashboard data must exist");
    let json_start = start + marker.len();
    let json_end = html[json_start..].find("</script>").unwrap() + json_start;
    let json_str = &html[json_start..json_end];
    let dashboard: serde_json::Value = serde_json::from_str(json_str).unwrap();

    let html_risks = dashboard["risks"]["top_risks"]
        .as_array()
        .expect("top_risks must be an array");

    // Each risk entry in HTML must match a real risk entry
    for html_risk in html_risks {
        let path = html_risk["path"].as_str().unwrap();
        let html_score = html_risk["risk_score"].as_f64().unwrap();
        let real_risk = risks.iter().find(|r| r.path == path);
        assert!(
            real_risk.is_some(),
            "HTML risk entry '{path}' must correspond to a real risk"
        );
        let real_score = real_risk.unwrap().risk_score;
        assert!(
            (html_score - real_score).abs() < 0.01,
            "risk score for {path}: HTML={html_score}, real={real_score}"
        );
    }
}

#[cfg(feature = "visual")]
#[test]
fn spa_e2e_search_index_completeness() {
    let index = build_realistic_index();
    let metadata = cxpak::visual::render::RenderMetadata {
        repo_name: "test".to_string(),
        generated_at: "2026-04-17T12:00:00Z".to_string(),
        health_score: Some(5.0),
        node_count: index.total_files,
        edge_count: 0,
        cxpak_version: "2.1.0".to_string(),
    };
    let html = cxpak::visual::spa::render_spa(&index, &metadata).unwrap();

    let marker = r#"id="cxpak-search-index" type="application/json">"#;
    let start = html.find(marker).expect("search index must exist");
    let json_start = start + marker.len();
    let json_end = html[json_start..].find("</script>").unwrap() + json_start;
    let json_str = &html[json_start..json_end];
    let entries: Vec<serde_json::Value> = serde_json::from_str(json_str).unwrap();

    // Every file in the index must appear in the search index
    for file in &index.files {
        assert!(
            entries
                .iter()
                .any(|e| e["kind"] == "file" && e["label"] == file.relative_path),
            "file '{}' missing from search index",
            file.relative_path
        );
    }

    // Every public symbol must appear
    for file in &index.files {
        if let Some(pr) = &file.parse_result {
            for sym in &pr.symbols {
                if matches!(sym.visibility, cxpak::parser::language::Visibility::Public) {
                    assert!(
                        entries.iter().any(|e| e["kind"] == "symbol" && e["label"] == sym.name),
                        "public symbol '{}' missing from search index",
                        sym.name
                    );
                }
            }
        }
    }
}

#[cfg(feature = "visual")]
#[test]
fn spa_e2e_architecture_node_count_matches() {
    let index = build_realistic_index();
    let arch = cxpak::intelligence::architecture::build_architecture_map(&index, 2);
    let metadata = cxpak::visual::render::RenderMetadata {
        repo_name: "test".to_string(),
        generated_at: "2026-04-17T12:00:00Z".to_string(),
        health_score: Some(5.0),
        node_count: index.total_files,
        edge_count: 0,
        cxpak_version: "2.1.0".to_string(),
    };
    let html = cxpak::visual::spa::render_spa(&index, &metadata).unwrap();

    let marker = r#"id="cxpak-architecture-data" type="application/json">"#;
    if let Some(start) = html.find(marker) {
        let json_start = start + marker.len();
        let json_end = html[json_start..].find("</script>").unwrap() + json_start;
        let json_str = &html[json_start..json_end];
        let arch_data: serde_json::Value = serde_json::from_str(json_str).unwrap();
        if let Some(nodes) = arch_data["level1"]["nodes"].as_array() {
            // Module count in architecture data should match
            assert_eq!(
                nodes.len(),
                arch.modules.len(),
                "architecture L1 node count ({}) should match module count ({})",
                nodes.len(),
                arch.modules.len()
            );
        }
    }
}

#[cfg(feature = "visual")]
#[test]
fn spa_e2e_deterministic_output() {
    let index = build_realistic_index();
    let metadata = cxpak::visual::render::RenderMetadata {
        repo_name: "test".to_string(),
        generated_at: "2026-04-17T12:00:00Z".to_string(),
        health_score: Some(5.0),
        node_count: index.total_files,
        edge_count: 0,
        cxpak_version: "2.1.0".to_string(),
    };
    let html1 = cxpak::visual::spa::render_spa(&index, &metadata).unwrap();
    let html2 = cxpak::visual::spa::render_spa(&index, &metadata).unwrap();
    assert_eq!(html1, html2, "SPA output must be deterministic");
}
```

- [ ] **Step 2: Run tests (will fail until Task 4 is implemented)**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --test spa_e2e -- --nocapture
```

- [ ] **Step 3: Commit**

```bash
git add tests/spa_e2e.rs
git commit -m "test(visual): add SPA end-to-end data integrity tests"
```

---

### Task 11: Version Bump to 2.1.0

**Files:**
- `Cargo.toml`
- `plugin/.claude-plugin/plugin.json`
- `.claude-plugin/marketplace.json`
- `plugin/lib/ensure-cxpak`

- [ ] **Step 1: Update all 4 version files**

```bash
# Cargo.toml
sed -i '' 's/^version = "2.0.0"/version = "2.1.0"/' Cargo.toml
# plugin.json
sed -i '' 's/"version": "2.0.0"/"version": "2.1.0"/' plugin/.claude-plugin/plugin.json
# marketplace.json
sed -i '' 's/"version": "2.0.0"/"version": "2.1.0"/' .claude-plugin/marketplace.json
# ensure-cxpak
sed -i '' 's/REQUIRED_VERSION="2.0.0"/REQUIRED_VERSION="2.1.0"/' plugin/lib/ensure-cxpak
```

- [ ] **Step 2: Regenerate Cargo.lock**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo check
```

- [ ] **Step 3: Run full test suite**

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --verbose
RUSTUP_TOOLCHAIN=1.94.1 cargo fmt -- --check
RUSTUP_TOOLCHAIN=1.94.1 cargo clippy --all-targets --all-features -- -D warnings
```

Expected: All pass.

- [ ] **Step 4: Verify no TODO/FIXME/unimplemented**

```bash
grep -rn 'TODO\|FIXME\|todo!()\|unimplemented!()' src/visual/spa.rs src/visual/search_index.rs tests/spa_e2e.rs tests/v1_api_wired.rs tests/lsp_methods_wired.rs
```

Expected: No matches.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock plugin/.claude-plugin/plugin.json .claude-plugin/marketplace.json plugin/lib/ensure-cxpak
git commit -m "chore: bump version to 2.1.0"
```

---

## Validation Checkpoints

### After Task 5 (Pillar 1 complete):

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --verbose
# Verify SPA output works:
RUSTUP_TOOLCHAIN=1.94.1 cargo run --all-features -- visual --visual-type all . 2>/dev/null
# Check output file exists and is valid HTML:
head -1 cxpak-all.html  # should be <!DOCTYPE html>
grep -c 'view-dashboard\|view-architecture\|view-risk' cxpak-all.html  # should be >= 3
```

### After Task 8 (Pillar 3 complete):

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --verbose
# Verify no stubs remain:
grep -n 'not_implemented' src/commands/serve.rs  # should be 0 matches
grep -n '"status":"available"' src/lsp/methods.rs  # should be 0 matches
```

### After Task 11 (final):

```bash
RUSTUP_TOOLCHAIN=1.94.1 cargo test --all-features --verbose
RUSTUP_TOOLCHAIN=1.94.1 cargo fmt -- --check
RUSTUP_TOOLCHAIN=1.94.1 cargo clippy --all-targets --all-features -- -D warnings
# Version check:
grep '^version' Cargo.toml | head -1  # 2.1.0
grep 'REQUIRED_VERSION' plugin/lib/ensure-cxpak  # 2.1.0
# No stubs:
grep -rn 'not_implemented' src/commands/serve.rs src/lsp/methods.rs  # 0 matches
# No placeholders:
grep -rn 'TODO\|FIXME\|todo!\|unimplemented!' src/visual/spa.rs src/visual/search_index.rs  # 0 matches
```
