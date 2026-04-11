# cxpak v0.6.1 — Bugfix Patch

**Goal:** Fix three bugs shipped in v0.6.0: dead code (`--focus`/`--timing` no-ops in trace and diff), wrong tokenizer, and useless git recency scoring.

---

## Bug 1: `--focus` and `--timing` are no-ops in trace and diff

### Problem

`src/commands/trace.rs` and `src/commands/diff.rs` accept `--focus` and `--timing` via CLI but the parameters are prefixed with `_` and never used. The flags parse correctly but do nothing — silent no-ops.

`src/commands/overview.rs` has the full implementation and serves as the reference.

### Fix: trace.rs

**Timing:** Wrap each pipeline stage (scan, parse, index, graph, search, render) with `std::time::Instant` and print durations to stderr when `timing` is true. Follow overview.rs pattern.

**Focus:** After building the dependency graph (step 5), compute `ranking::rank_files()` and `ranking::apply_focus()`. Use scores to reorder the relevant files so higher-scored files get budget priority in sections 8 (source code) and 9 (signatures). This means the `render_symbol_source` and `render_relevant_signatures` functions receive files pre-sorted by importance.

Changes:
- `src/commands/trace.rs`: Remove `_` prefix from `focus` and `timing` params. Add timing instrumentation. Add ranking + focus integration after graph build.
- New imports: `use crate::index::ranking;`, `use crate::git;`

### Fix: diff.rs

**Timing:** Same pattern — wrap scan, git extract, parse, index, graph walk, render stages.

**Focus:** After building the index and dependency graph (steps 4-5), compute rankings and apply focus. The focus boost affects which context files (non-changed but reachable) get budget priority in `render_context_signatures`.

Changes:
- `src/commands/diff.rs`: Remove `_` prefix from `focus` and `timing` params. Add timing instrumentation. Add ranking + focus integration.
- New imports: same as trace.

### Tests

- Integration test: `cxpak trace --tokens 50k --timing --focus src/ my_symbol` — verify timing output on stderr, verify focus-boosted files appear earlier
- Integration test: `cxpak diff --tokens 50k --timing --focus src/` — same verification
- Unit tests for ranking integration within trace/diff are covered by existing ranking.rs tests

---

## Bug 2: Wrong tokenizer

### Problem

`src/budget/counter.rs` line 1 imports `cl100k_base` and line 16 initializes with `cl100k_base()`. This is the GPT-3.5/GPT-4 tokenizer. Claude and GPT-4o use `o200k_base`. Token counts are systematically wrong — not by a huge margin, but wrong.

### Fix

```rust
// Before
use tiktoken_rs::cl100k_base;
// ...
bpe: cl100k_base().expect("failed to load cl100k_base tokenizer"),

// After
use tiktoken_rs::o200k_base;
// ...
bpe: o200k_base().expect("failed to load o200k_base tokenizer"),
```

`o200k_base()` is available in tiktoken-rs 0.6.0 (already our dependency). No Cargo.toml change needed.

### Impact

Token counts will shift. Budgets will behave slightly differently. This is correct — the old counts were wrong.

### Tests

Existing tests in `counter.rs` are count-range assertions (`count > 0 && count < 30`), not exact values. They should still pass. Verify.

---

## Bug 3: Binary git recency

### Problem

`src/index/ranking.rs` lines 35-40:

```rust
let git_recency = git_context
    .and_then(|g| g.commits.first())
    .map(|_| 1.0)
    .unwrap_or(0.0);
```

This checks whether *any* commit exists. In any active repo, it's always 1.0 for every file. It contributes 0.3 weight to every file equally — pure noise.

### Fix

Compute per-file recency from git context. Use `file_churn` data (which already has per-file commit counts) combined with commit dates.

```rust
// Build a per-file recency map from the git context.
// Files with recent commits get higher recency scores.
let recency_map: HashMap<&str, f64> = git_context
    .map(|g| {
        // Use file_churn order as a proxy for recency — files with more
        // commits in the recent window are more "recently active".
        // Normalize to 0.0-1.0 range.
        let max = g.file_churn.len().max(1) as f64;
        g.file_churn
            .iter()
            .enumerate()
            .map(|(i, f)| {
                // Higher rank (lower index) = more active = higher recency
                let score = 1.0 - (i as f64 / max);
                (f.path.as_str(), score)
            })
            .collect()
    })
    .unwrap_or_default();
```

Then in the per-file loop:
```rust
let git_recency = recency_map.get(path.as_str()).copied().unwrap_or(0.0);
```

This gives files that appear in the churn list (recently active files) a recency score proportional to their activity rank. Files not in the churn list get 0.0.

### Tests

- Update `test_rank_files_basic` and `test_rank_files_with_git` to verify that files with more churn get higher recency
- Add test: two files, one in churn list and one not — verify recency differs
- Existing `test_rank_files_no_graph_no_git` should still pass (all 0.0 with no git context)

---

## Release

- Version bump: 0.6.0 → 0.6.1 in `Cargo.toml`, `plugin/.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json`
- Tag: `v0.6.1`
- CI: same workflow as v0.6.0 (cross-compile + crates.io publish)
