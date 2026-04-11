# cxpak v0.6.1 Bugfix Patch — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix three bugs shipped in v0.6.0: dead `--focus`/`--timing` in trace and diff, wrong tokenizer, and binary git recency scoring.

**Architecture:** Three independent fixes, each testable in isolation. Bug 1 (dead code) adds timing instrumentation and ranking/focus integration to `trace.rs` and `diff.rs`, following the existing pattern in `overview.rs`. Bug 2 (tokenizer) is a two-line swap. Bug 3 (recency) replaces a global scalar with per-file scores from the churn list.

**Tech Stack:** Rust, tiktoken-rs (`o200k_base`), git2, tree-sitter, cargo test

---

### Task 1: Fix tokenizer — cl100k_base → o200k_base

**Files:**
- Modify: `src/budget/counter.rs:1` and `src/budget/counter.rs:16`

**Step 1: Verify existing tests pass**

Run: `cargo test --test-threads=1 -p cxpak counter -- --nocapture`
Expected: All 4 tests PASS

**Step 2: Change the import and initialization**

In `src/budget/counter.rs`, make two changes:

Line 1 — change:
```rust
use tiktoken_rs::cl100k_base;
```
to:
```rust
use tiktoken_rs::o200k_base;
```

Line 16 — change:
```rust
bpe: cl100k_base().expect("failed to load cl100k_base tokenizer"),
```
to:
```rust
bpe: o200k_base().expect("failed to load o200k_base tokenizer"),
```

**Step 3: Run tests to verify nothing broke**

Run: `cargo test --test-threads=1 -p cxpak counter -- --nocapture`
Expected: All 4 tests PASS (they use range assertions like `count > 0 && count < 30`, not exact values)

**Step 4: Commit**

```bash
git add src/budget/counter.rs
git commit -m "fix: use o200k_base tokenizer (Claude/GPT-4o) instead of cl100k_base"
```

---

### Task 2: Fix binary git_recency → per-file recency from churn

**Files:**
- Modify: `src/index/ranking.rs:24-63` (the `rank_files` function)
- Test: `src/index/ranking.rs` (inline `#[cfg(test)]` module)

**Step 1: Write a failing test for per-file recency differentiation**

Add this test to the `mod tests` block in `src/index/ranking.rs` (after the existing `test_apply_focus_no_match` test):

```rust
#[test]
fn test_recency_differs_per_file() {
    let graph = DependencyGraph::new();
    // file_churn list: hot.rs has 10 commits, cold.rs has 1.
    // Order matters: hot.rs is index 0 (highest churn = highest recency).
    let git = make_git_context(vec![("hot.rs", 10), ("cold.rs", 1)], vec!["2026-03-12"]);

    let paths = vec!["hot.rs".into(), "cold.rs".into(), "absent.rs".into()];
    let scores = rank_files(&paths, &graph, Some(&git));

    let hot = scores.iter().find(|s| s.path == "hot.rs").unwrap();
    let cold = scores.iter().find(|s| s.path == "cold.rs").unwrap();
    let absent = scores.iter().find(|s| s.path == "absent.rs").unwrap();

    // hot.rs should have higher recency than cold.rs
    assert!(
        hot.git_recency > cold.git_recency,
        "hot.rs recency {} should be > cold.rs recency {}",
        hot.git_recency, cold.git_recency
    );
    // absent.rs is not in churn list → recency 0.0
    assert_eq!(absent.git_recency, 0.0, "file not in churn list should have 0.0 recency");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --test-threads=1 -p cxpak test_recency_differs_per_file -- --nocapture`
Expected: FAIL — currently both files get recency 1.0 (binary check)

**Step 3: Implement per-file recency**

Replace the recency computation in `rank_files` (lines 34-40 and the per-file usage in the closure). The full updated function body:

In `src/index/ranking.rs`, replace lines 24-64 (the entire `rank_files` function body) with:

```rust
pub fn rank_files(
    file_paths: &[String],
    graph: &DependencyGraph,
    git_context: Option<&GitContext>,
) -> Vec<FileScore> {
    let churn_map: HashMap<&str, usize> = git_context
        .map(|g| {
            g.file_churn
                .iter()
                .map(|f| (f.path.as_str(), f.commit_count))
                .collect()
        })
        .unwrap_or_default();

    let max_churn = churn_map.values().copied().max().unwrap_or(1) as f64;

    // Build a per-file recency map from the git context.
    // Files with more recent activity (lower index in the churn list,
    // which is sorted by commit count descending) get higher scores.
    let recency_map: HashMap<&str, f64> = git_context
        .map(|g| {
            let max = g.file_churn.len().max(1) as f64;
            g.file_churn
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let score = 1.0 - (i as f64 / max);
                    (f.path.as_str(), score)
                })
                .collect()
        })
        .unwrap_or_default();

    file_paths
        .iter()
        .map(|path| {
            let in_degree = graph.dependents(path).len();
            let out_degree = graph.dependencies(path).map(|d| d.len()).unwrap_or(0);
            let file_churn = churn_map.get(path.as_str()).copied().unwrap_or(0) as f64 / max_churn;
            let git_recency = recency_map.get(path.as_str()).copied().unwrap_or(0.0);

            let composite = in_degree as f64 * 0.4
                + out_degree as f64 * 0.1
                + git_recency * 0.3
                + file_churn * 0.2;

            FileScore {
                path: path.clone(),
                in_degree,
                out_degree,
                git_recency,
                git_churn: file_churn,
                composite,
            }
        })
        .collect()
}
```

**Step 4: Run all ranking tests**

Run: `cargo test --test-threads=1 -p cxpak ranking -- --nocapture`
Expected: All tests PASS including:
- `test_recency_differs_per_file` — NEW, passes
- `test_rank_files_no_graph_no_git` — still passes (all 0.0 with no git context)
- `test_rank_files_basic` — still passes (no git context, recency is 0.0)
- `test_rank_files_with_git` — still passes (hot.rs has higher churn AND recency)
- `test_rank_files_empty` — still passes
- `test_apply_focus` — still passes
- `test_apply_focus_no_match` — still passes

**Step 5: Commit**

```bash
git add src/index/ranking.rs
git commit -m "fix: compute per-file git recency from churn list instead of binary check"
```

---

### Task 3: Wire --timing into trace.rs

**Files:**
- Modify: `src/commands/trace.rs:1-23` (imports and function signature)
- Modify: `src/commands/trace.rs:24-178` (function body — add timing instrumentation)

**Step 1: Remove `_` prefix from `timing` parameter and add timing instrumentation**

In `src/commands/trace.rs`:

1. Change line 22: `_timing: bool,` → `timing: bool,`

2. Add `use std::time::Instant;` at the top (after line 9, before line 10). The import block becomes:

```rust
use crate::budget::counter::TokenCounter;
use crate::budget::degrader;
use crate::cli::OutputFormat;
use crate::index::graph::DependencyGraph;
use crate::index::CodebaseIndex;
use crate::output::{self, OutputSections};
use crate::scanner::Scanner;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::time::Instant;
```

3. Add timing instrumentation around each pipeline stage. Wrap the existing stages:

After `let counter = TokenCounter::new();` (line 24), add:
```rust
    let total_start = Instant::now();
```

Before `// 1. Scan` (line 26), add:
```rust
    let scan_start = Instant::now();
```

After `if files.is_empty()` block (after line 38), add:
```rust
    if timing {
        eprintln!("cxpak [timing]: scan       {:.1?}", scan_start.elapsed());
    }
```

Before `// 2. Parse` (line 41), add:
```rust
    let parse_start = Instant::now();
```

After the parse call (after line 41), add:
```rust
    if timing {
        eprintln!("cxpak [timing]: parse      {:.1?}", parse_start.elapsed());
    }
```

Before `// 3. Index` (line 43), add:
```rust
    let index_start = Instant::now();
```

After the index verbose print (after line 51), add:
```rust
    if timing {
        eprintln!("cxpak [timing]: index      {:.1?}", index_start.elapsed());
    }
```

Before `// 5. Build dependency graph` (line 77), add:
```rust
    let graph_start = Instant::now();
```

After the graph build (after line 78), add:
```rust
    if timing {
        eprintln!("cxpak [timing]: graph      {:.1?}", graph_start.elapsed());
    }
```

Before `// 6. Find the target` (line 80), add:
```rust
    let search_start = Instant::now();
```

After the graph walk block (after line 135), add:
```rust
    if timing {
        eprintln!("cxpak [timing]: search     {:.1?}", search_start.elapsed());
    }
```

Before `// 8. Build output sections` (line 137), add:
```rust
    let render_start = Instant::now();
```

Before the `match out` block (before line 164), add:
```rust
    if timing {
        eprintln!("cxpak [timing]: render     {:.1?}", render_start.elapsed());
        eprintln!("cxpak [timing]: total      {:.1?}", total_start.elapsed());
    }
```

**Step 2: Run the full test suite to verify nothing broke**

Run: `cargo test --test-threads=1 -- --nocapture`
Expected: All tests PASS (timing is `false` by default, so no behavior change)

**Step 3: Commit**

```bash
git add src/commands/trace.rs
git commit -m "fix: wire --timing flag into trace command"
```

---

### Task 4: Wire --focus into trace.rs

**Files:**
- Modify: `src/commands/trace.rs:1-10` (add imports)
- Modify: `src/commands/trace.rs:21` (remove `_` from focus param)
- Modify: `src/commands/trace.rs` (add ranking/focus after graph build)

**Step 1: Remove `_` prefix from `focus` parameter and add ranking imports**

In `src/commands/trace.rs`:

1. Change line 21: `_focus: Option<&str>,` → `focus: Option<&str>,`

2. Add these imports after the existing ones (after `use std::time::Instant;`):

```rust
use crate::git;
use crate::index::ranking;
```

**Step 2: Add ranking/focus integration after graph build**

After the graph build and its timing print (after the `graph_start` timing block), add ranking integration. This goes right before `// 6. Find the target`:

```rust
    // 5b. Rank files and apply focus
    let git_ctx = git::extract_git_context(path, 20).ok();
    let file_paths: Vec<String> = index.files.iter().map(|f| f.relative_path.clone()).collect();
    let mut scores = ranking::rank_files(&file_paths, &graph, git_ctx.as_ref());
    if let Some(focus_path) = focus {
        ranking::apply_focus(&mut scores, focus_path, &graph);
    }

    // Build path→score map for ordering relevant files by importance
    let score_map: std::collections::HashMap<&str, f64> = scores
        .iter()
        .map(|s| (s.path.as_str(), s.composite))
        .collect();
```

Then, after `render_symbol_source` and `render_relevant_signatures` calls, the relevant files should be sorted by score. Modify the `render_relevant_signatures` call to pass files in score order. Actually, the simplest approach is to sort `index.files` by score before the render calls, same as overview.rs does:

Right after building the `score_map`, add:
```rust
    index.files.sort_by(|a, b| {
        let sa = score_map.get(a.relative_path.as_str()).unwrap_or(&0.0);
        let sb = score_map.get(b.relative_path.as_str()).unwrap_or(&0.0);
        sb.partial_cmp(sa).unwrap_or(std::cmp::Ordering::Equal)
    });
```

This requires changing `let index = ...` to `let mut index = ...` on the line that builds the index (~line 44).

**Step 3: Run the full test suite**

Run: `cargo test --test-threads=1 -- --nocapture`
Expected: All tests PASS

**Step 4: Commit**

```bash
git add src/commands/trace.rs
git commit -m "fix: wire --focus flag into trace command with ranking integration"
```

---

### Task 5: Wire --timing into diff.rs

**Files:**
- Modify: `src/commands/diff.rs:88-99` (function signature)
- Modify: `src/commands/diff.rs:100-228` (function body — add timing)

**Step 1: Remove `_` prefix from `timing` and add timing instrumentation**

In `src/commands/diff.rs`:

1. Change line 98: `_timing: bool,` → `timing: bool,`

2. Add timing instrumentation following the same pattern as trace.rs. The stages are:

After `// 1. Extract git changes` verbose print, before `let changes = extract_changes(...)`:
```rust
    let total_start = std::time::Instant::now();
    let extract_start = std::time::Instant::now();
```

After `if changes.is_empty()` block (after line 111):
```rust
    if timing {
        eprintln!("cxpak [timing]: extract    {:.1?}", extract_start.elapsed());
    }
```

Before `// 2. Scan repo`:
```rust
    let scan_start = std::time::Instant::now();
```

After `let counter = TokenCounter::new();` (line 127):
```rust
    if timing {
        eprintln!("cxpak [timing]: scan       {:.1?}", scan_start.elapsed());
    }
```

Before `// 3. Parse with cache`:
```rust
    let parse_start = std::time::Instant::now();
```

After the parse call:
```rust
    if timing {
        eprintln!("cxpak [timing]: parse      {:.1?}", parse_start.elapsed());
    }
```

Before `// 4. Build index`:
```rust
    let index_start = std::time::Instant::now();
```

After the index verbose block:
```rust
    if timing {
        eprintln!("cxpak [timing]: index      {:.1?}", index_start.elapsed());
    }
```

Before `// 5. Build dependency graph`:
```rust
    let graph_start = std::time::Instant::now();
```

After the graph build:
```rust
    if timing {
        eprintln!("cxpak [timing]: graph      {:.1?}", graph_start.elapsed());
    }
```

Before `// 8. Build diff section text`:
```rust
    let render_start = std::time::Instant::now();
```

Before the `match out` block:
```rust
    if timing {
        eprintln!("cxpak [timing]: render     {:.1?}", render_start.elapsed());
        eprintln!("cxpak [timing]: total      {:.1?}", total_start.elapsed());
    }
```

**Step 2: Run tests**

Run: `cargo test --test-threads=1 -p cxpak diff -- --nocapture`
Expected: All 7 diff tests PASS

**Step 3: Commit**

```bash
git add src/commands/diff.rs
git commit -m "fix: wire --timing flag into diff command"
```

---

### Task 6: Wire --focus into diff.rs

**Files:**
- Modify: `src/commands/diff.rs:97` (remove `_` from focus)
- Modify: `src/commands/diff.rs` (add ranking/focus after graph build)

**Step 1: Remove `_` prefix from `focus` and add ranking imports**

In `src/commands/diff.rs`:

1. Change line 97: `_focus: Option<&str>,` → `focus: Option<&str>,`

2. Add imports at the top (after existing `use` block):
```rust
use crate::git;
use crate::index::ranking;
```

**Step 2: Add ranking/focus integration**

After the graph build and its timing print (after `graph_start` timing block), before `// 6. Determine the set of changed file paths`, add:

```rust
    // 5b. Rank files and apply focus
    let git_ctx = git::extract_git_context(path, 20).ok();
    let file_paths: Vec<String> = index.files.iter().map(|f| f.relative_path.clone()).collect();
    let mut scores = ranking::rank_files(&file_paths, &graph, git_ctx.as_ref());
    if let Some(focus_path) = focus {
        ranking::apply_focus(&mut scores, focus_path, &graph);
    }

    // Sort index files by score so higher-ranked context files get budget priority
    let score_map: std::collections::HashMap<&str, f64> = scores
        .iter()
        .map(|s| (s.path.as_str(), s.composite))
        .collect();
    index.files.sort_by(|a, b| {
        let sa = score_map.get(a.relative_path.as_str()).unwrap_or(&0.0);
        let sb = score_map.get(b.relative_path.as_str()).unwrap_or(&0.0);
        sb.partial_cmp(sa).unwrap_or(std::cmp::Ordering::Equal)
    });
```

This requires changing `let index = CodebaseIndex::build(...)` to `let mut index = CodebaseIndex::build(...)` (~line 133).

**Step 3: Run tests**

Run: `cargo test --test-threads=1 -- --nocapture`
Expected: All tests PASS

**Step 4: Commit**

```bash
git add src/commands/diff.rs
git commit -m "fix: wire --focus flag into diff command with ranking integration"
```

---

### Task 7: Version bump 0.6.0 → 0.6.1

**Files:**
- Modify: `Cargo.toml:3` — `version = "0.6.0"` → `version = "0.6.1"`
- Modify: `plugin/.claude-plugin/plugin.json:4` — `"version": "0.6.0"` → `"version": "0.6.1"`
- Modify: `.claude-plugin/marketplace.json:12` — `"version": "0.6.0"` → `"version": "0.6.1"`

**Step 1: Bump all three version strings**

In `Cargo.toml` line 3:
```
version = "0.6.1"
```

In `plugin/.claude-plugin/plugin.json` line 4:
```json
"version": "0.6.1",
```

In `.claude-plugin/marketplace.json` line 12:
```json
"version": "0.6.1"
```

**Step 2: Run full test suite + clippy + fmt**

Run: `cargo fmt -- --check && cargo clippy --all-targets -- -D warnings && cargo test --test-threads=1 -- --nocapture`
Expected: All checks PASS

**Step 3: Commit**

```bash
git add Cargo.toml plugin/.claude-plugin/plugin.json .claude-plugin/marketplace.json
git commit -m "chore: bump version to 0.6.1"
```

---

### Task 8: Final verification

**Step 1: Run the full suite one final time**

Run: `cargo test --test-threads=1 -- --nocapture`
Expected: All tests PASS

**Step 2: Run clippy and fmt**

Run: `cargo fmt -- --check && cargo clippy --all-targets -- -D warnings`
Expected: No warnings, no formatting issues

**Step 3: Smoke test the three fixes manually**

```bash
# Tokenizer: cxpak should work (no crash from missing tokenizer)
cargo run -- overview --tokens 10k .

# Timing: should print timing to stderr
cargo run -- trace --tokens 10k --timing TokenCounter . 2>&1 | grep "cxpak \[timing\]"
cargo run -- diff --tokens 10k --timing . 2>&1 | grep "cxpak \[timing\]"

# Focus: should not crash (functional verification is via ranking tests)
cargo run -- trace --tokens 10k --focus src/ TokenCounter .
cargo run -- diff --tokens 10k --focus src/ .
```

Expected: All commands succeed. Timing commands show `cxpak [timing]:` lines on stderr.
