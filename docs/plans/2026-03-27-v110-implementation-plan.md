# v1.1.0 Implementation Plan: Repository DNA

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract the codebase's actual patterns as a quantified convention profile, include a ~1000 token DNA section in every `auto_context` call, and verify code changes against the profile with evidence-based findings.

**Architecture:** New `src/conventions/` module (11 files) built at index time with incremental updates. Convention profile stored on `CodebaseIndex`. `auto_context` gains a DNA section (step 0, ~1000 tokens, never degraded). Two new MCP tools: `cxpak_verify` (checks changed lines against conventions) and `cxpak_conventions` (returns full profile). All extraction is deterministic from AST + graph + git2.

**Tech Stack:** Rust, tree-sitter (existing AST), git2 (existing dependency), regex (existing)

**Spec:** `docs/plans/2026-03-27-v110-design.md`

---

## File Structure

### New Files
- `src/conventions/mod.rs` — public types, `ConventionProfile`, `PatternObservation`, `PatternStrength`, `build_convention_profile()`
- `src/conventions/naming.rs` — naming style classification + extraction
- `src/conventions/imports.rs` — import style, grouping, re-export detection
- `src/conventions/errors.rs` — error return types, unwrap detection, ? propagation (Rust only)
- `src/conventions/deps.rs` — dependency direction, layering, circular detection
- `src/conventions/testing.rs` — coverage, mock detection, test naming, density
- `src/conventions/visibility.rs` — public/private distribution, doc comment coverage
- `src/conventions/functions.rs` — function length stats per directory
- `src/conventions/git_health.rs` — churn (30d/180d), bug-fix density, revert detection
- `src/conventions/render.rs` — render DNA markdown (~1000 tokens)
- `src/conventions/verify.rs` — diff scoping via git2, convention checking, violation + suggestion generation

### Modified Files
- `src/main.rs` — add `pub mod conventions;`
- `src/index/mod.rs` — add `pub conventions: ConventionProfile` field to `CodebaseIndex`
- `src/commands/serve.rs` — add 2 MCP tools (verify #12, conventions #13), call `build_convention_profile` in `build_index()`
- `src/auto_context/mod.rs` — add step 0 (render DNA), add `dna` field to `AutoContextResult`
- `src/auto_context/briefing.rs` — no signature change (caller subtracts DNA tokens before calling)

---

## Stream 1: Core Types + Pattern Extraction

### Task 1: Scaffold conventions module + core types

**Files:**
- Create: `src/conventions/mod.rs` + 10 empty submodule files
- Modify: `src/main.rs`

- [ ] **Step 1: Create all module files**

`src/conventions/mod.rs`:
```rust
pub mod naming;
pub mod imports;
pub mod errors;
pub mod deps;
pub mod testing;
pub mod visibility;
pub mod functions;
pub mod git_health;
pub mod render;
pub mod verify;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PatternStrength {
    Convention,  // ≥90%
    Trend,       // 70-89%
    Mixed,       // 50-69%
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternObservation {
    pub name: String,
    pub dominant: String,
    pub count: usize,
    pub total: usize,
    pub percentage: f64,
    pub strength: PatternStrength,
    pub exceptions: Vec<String>,
}

impl PatternObservation {
    pub fn new(name: &str, dominant: &str, count: usize, total: usize) -> Option<Self> {
        if total == 0 { return None; }
        let percentage = (count as f64 / total as f64) * 100.0;
        if percentage < 50.0 { return None; } // below 50% = no dominant pattern
        let strength = if percentage >= 90.0 {
            PatternStrength::Convention
        } else if percentage >= 70.0 {
            PatternStrength::Trend
        } else {
            PatternStrength::Mixed
        };
        Some(Self {
            name: name.to_string(),
            dominant: dominant.to_string(),
            count, total, percentage, strength,
            exceptions: Vec::new(),
        })
    }

    pub fn with_exceptions(mut self, exceptions: Vec<String>) -> Self {
        self.exceptions = exceptions;
        self
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConventionProfile {
    pub naming: naming::NamingConventions,
    pub imports: imports::ImportConventions,
    pub errors: errors::ErrorConventions,
    pub dependencies: deps::DependencyConventions,
    pub testing: testing::TestingConventions,
    pub visibility: visibility::VisibilityConventions,
    pub functions: functions::FunctionConventions,
    pub git_health: git_health::GitHealthProfile,
}
```

Empty scaffolds for all 10 submodules.

- [ ] **Step 2: Add `pub mod conventions;` to `src/main.rs`**

- [ ] **Step 3: Write tests for `PatternObservation::new()`**

```rust
#[test]
fn test_pattern_observation_convention() {
    let obs = PatternObservation::new("fn_naming", "snake_case", 95, 100).unwrap();
    assert_eq!(obs.percentage, 95.0);
    assert!(matches!(obs.strength, PatternStrength::Convention));
}

#[test]
fn test_pattern_observation_trend() {
    let obs = PatternObservation::new("fn_naming", "snake_case", 75, 100).unwrap();
    assert!(matches!(obs.strength, PatternStrength::Trend));
}

#[test]
fn test_pattern_observation_mixed() {
    let obs = PatternObservation::new("fn_naming", "snake_case", 55, 100).unwrap();
    assert!(matches!(obs.strength, PatternStrength::Mixed));
}

#[test]
fn test_pattern_observation_below_50_returns_none() {
    assert!(PatternObservation::new("fn_naming", "snake_case", 40, 100).is_none());
}

#[test]
fn test_pattern_observation_zero_total_returns_none() {
    assert!(PatternObservation::new("fn_naming", "snake_case", 0, 0).is_none());
}
```

- [ ] **Step 4: Verify compilation + tests**

Run: `cargo test conventions --verbose`

- [ ] **Step 5: Commit**

```bash
git add src/conventions/ src/main.rs
git commit -m "feat: scaffold conventions module with core types for v1.1.0"
```

### Task 2: Implement naming pattern extraction

**Files:**
- Modify: `src/conventions/naming.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_classify_snake_case() {
    assert_eq!(classify_name("handle_request"), NamingStyle::SnakeCase);
    assert_eq!(classify_name("rate_limit"), NamingStyle::SnakeCase);
}

#[test]
fn test_classify_camel_case() {
    assert_eq!(classify_name("handleRequest"), NamingStyle::CamelCase);
}

#[test]
fn test_classify_pascal_case() {
    assert_eq!(classify_name("HandleRequest"), NamingStyle::PascalCase);
    assert_eq!(classify_name("UserService"), NamingStyle::PascalCase);
}

#[test]
fn test_classify_screaming_snake() {
    assert_eq!(classify_name("MAX_RETRIES"), NamingStyle::ScreamingSnake);
    assert_eq!(classify_name("API_KEY"), NamingStyle::ScreamingSnake);
}

#[test]
fn test_classify_single_word() {
    assert_eq!(classify_name("main"), NamingStyle::SnakeCase);
    assert_eq!(classify_name("Main"), NamingStyle::PascalCase);
}

#[test]
fn test_extract_naming_all_snake() {
    // Build index with all snake_case functions → Convention
}

#[test]
fn test_extract_naming_mixed() {
    // Build index with 80% snake, 20% camel → Trend
}

#[test]
fn test_extract_naming_types_separate() {
    // Functions snake_case, types PascalCase → both detected separately
}
```

- [ ] **Step 2: Implement `NamingStyle`, `classify_name()`, `extract_naming()`**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamingStyle {
    SnakeCase,
    CamelCase,
    PascalCase,
    ScreamingSnake,
    KebabCase,
    Other,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NamingConventions {
    pub function_style: Option<PatternObservation>,
    pub type_style: Option<PatternObservation>,
    pub file_style: Option<PatternObservation>,
    pub constant_style: Option<PatternObservation>,
    pub additional: Vec<PatternObservation>,
    // Per-file tracking for incremental updates
    #[serde(skip)]
    pub file_contributions: HashMap<String, FileNamingContribution>,
}

pub fn classify_name(name: &str) -> NamingStyle { ... }
pub fn extract_naming(index: &CodebaseIndex) -> NamingConventions { ... }
```

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/conventions/naming.rs
git commit -m "feat: naming convention extraction with style classification"
```

### Task 3: Implement import pattern extraction

**Files:** `src/conventions/imports.rs`

- [ ] **Step 1: Write tests** — absolute vs relative detection, grouping, re-exports
- [ ] **Step 2: Implement `extract_imports()`** — analyze `Import.source` strings
- [ ] **Step 3: Run tests**
- [ ] **Step 4: Commit**

```bash
git commit -m "feat: import convention extraction (style, grouping, re-exports)"
```

### Task 4: Implement error handling pattern extraction

**Files:** `src/conventions/errors.rs`

- [ ] **Step 1: Write tests** — Result return type count, unwrap detection, ? propagation (Rust only)
- [ ] **Step 2: Implement `extract_errors()`** — signature regex + body string search
- [ ] **Step 3: Run tests**
- [ ] **Step 4: Commit**

```bash
git commit -m "feat: error handling convention extraction (Result, unwrap, ? propagation)"
```

### Task 5: Implement dependency direction extraction

**Files:** `src/conventions/deps.rs`

- [ ] **Step 1: Write tests** — strict layering detection, partial layering, circular deps
- [ ] **Step 2: Implement `extract_deps()`** — analyze `DependencyGraph` edge directions per directory pair
- [ ] **Step 3: Run tests**
- [ ] **Step 4: Commit**

```bash
git commit -m "feat: dependency direction convention extraction (layering, circulars)"
```

### Task 6: Implement testing pattern extraction

**Files:** `src/conventions/testing.rs`

- [ ] **Step 1: Write tests** — coverage per dir from test_map, mock detection, test naming, density
- [ ] **Step 2: Implement `extract_testing()`** — query test_map + search test file content
- [ ] **Step 3: Run tests**
- [ ] **Step 4: Commit**

```bash
git commit -m "feat: testing convention extraction (coverage, mocks, naming, density)"
```

### Task 7: Implement visibility + function length extraction

**Files:** `src/conventions/visibility.rs`, `src/conventions/functions.rs`

- [ ] **Step 1: Write tests** — pub/private ratio, doc comment coverage, avg/median function length per directory
- [ ] **Step 2: Implement `extract_visibility()` and `extract_functions()`**
- [ ] **Step 3: Run tests**
- [ ] **Step 4: Commit**

```bash
git commit -m "feat: visibility and function length convention extraction"
```

### Task 8: Implement git health extraction

**Files:** `src/conventions/git_health.rs`

- [ ] **Step 1: Write tests**

```rust
#[test]
fn test_churn_30day_window() { ... }
#[test]
fn test_churn_180day_window() { ... }
#[test]
fn test_bugfix_density() { ... }
#[test]
fn test_revert_detection() {
    // Commit message: 'Revert "add unwrap to jwt" This reverts commit abc123'
    // → detected, original message extracted
}
#[test]
fn test_empty_repo_no_error() { ... }
```

- [ ] **Step 2: Implement `extract_git_health()`**

Uses `git2::Repository` (NOT CLI). Takes `repo_path: &Path`.
- Walk commits with `revwalk`, filter by time windows (30d, 180d)
- Count modifications per file from diff stats
- Bug-fix: match commit message against `fix|bug|patch|hotfix` (case-insensitive)
- Reverts: match `revert|Revert`, extract original commit message via `This reverts commit <hash>` regex

- [ ] **Step 3: Run tests**
- [ ] **Step 4: Commit**

```bash
git commit -m "feat: git health extraction (churn 30d/180d, bug-fix density, reverts)"
```

### Task 9: Build convention profile orchestrator + wire to CodebaseIndex

**Files:**
- Modify: `src/conventions/mod.rs`
- Modify: `src/index/mod.rs`
- Modify: `src/commands/serve.rs`

- [ ] **Step 1: Implement `build_convention_profile()`**

```rust
pub fn build_convention_profile(
    index: &CodebaseIndex,
    repo_path: &Path,
) -> ConventionProfile {
    ConventionProfile {
        naming: naming::extract_naming(index),
        imports: imports::extract_imports(index),
        errors: errors::extract_errors(index),
        dependencies: deps::extract_deps(index),
        testing: testing::extract_testing(index),
        visibility: visibility::extract_visibility(index),
        functions: functions::extract_functions(index),
        git_health: git_health::extract_git_health(repo_path),
    }
}
```

- [ ] **Step 2: Add `conventions` field to `CodebaseIndex`**

```rust
pub struct CodebaseIndex {
    // ... existing fields ...
    pub conventions: ConventionProfile,
}
```

Add `conventions: ConventionProfile::default()` to EVERY `Self { ... }` struct literal in `build()` and `build_with_content()` in `src/index/mod.rs`. There are exactly 2 construction sites — both must be updated.

- [ ] **Step 3: Populate in `build_index()` in `serve.rs`**

After existing index construction:
```rust
index.conventions = crate::conventions::build_convention_profile(&index, path);
```

Same pattern for CLI commands that build indices (`overview.rs`, `trace.rs`, `diff.rs` — all call `build_index` from `serve.rs`).

- [ ] **Step 4: Hook incremental convention updates into `watch.rs`**

In the serve loop where `process_watcher_changes` calls `apply_incremental_update`, add convention refresh AFTER the index update:

```rust
// In process_watcher_changes(), after apply_incremental_update:
if update_count > 0 {
    crate::conventions::update_conventions_incremental(
        &mut idx.conventions,
        &modified_paths,
        &removed_paths,
        &idx,
    );
}
```

Implement `update_conventions_incremental()` in `src/conventions/mod.rs`:
- For each removed file: subtract its contributions from the per-file maps
- For each modified file: subtract old contribution, add new contribution from re-parsed symbols
- Recompute percentages and strength labels for affected categories
- Git health: NOT updated here (uses 60s TTL cache, refreshed on verify/conventions calls)

Add `src/commands/watch.rs` to modified files for this task.

- [ ] **Step 5: Write integration test**

```rust
#[test]
fn test_convention_profile_builds_for_real_repo() {
    // Create temp repo with .rs files
    // Build index → verify conventions populated
}
```

- [ ] **Step 5: Run all tests**

- [ ] **Step 6: Commit**

```bash
git add src/conventions/mod.rs src/index/mod.rs src/commands/serve.rs
git commit -m "feat: build convention profile at index time, store on CodebaseIndex"
```

---

## Stream 2: DNA Section in Auto Context

### Task 10: Implement DNA renderer

**Files:** `src/conventions/render.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_render_dna_includes_conventions() {
    // Profile with Convention-strength naming → appears in DNA
}

#[test]
fn test_render_dna_includes_trends() {
    // Profile with Trend-strength visibility → appears in DNA
}

#[test]
fn test_render_dna_excludes_mixed() {
    // Profile with Mixed-strength pattern → NOT in DNA
}

#[test]
fn test_render_dna_under_1200_tokens() {
    // Full profile renders under 1200 tokens
}

#[test]
fn test_render_dna_includes_git_health() {
    // Top churn files appear in DNA
}

#[test]
fn test_render_dna_includes_reverts() {
    // Revert history appears as anti-patterns
}

#[test]
fn test_render_dna_empty_profile() {
    // Default profile → minimal or empty DNA
}
```

- [ ] **Step 2: Implement `render_dna_section()`**

Two rendering functions:

`render_dna_section(profile)` — full DNA (~800-1000 tokens). Convention + Trend patterns, ordered by percentage desc, plus git health. Uses `TokenCounter` to verify under 1200 tokens.

`render_compact_dna(profile)` — compact DNA (~200-300 tokens). Top 3 Convention-strength patterns only. Used when budget is 2000-5000 tokens.

- [ ] **Step 3: Run tests**
- [ ] **Step 4: Commit**

```bash
git commit -m "feat: render DNA section (~1000 tokens compact markdown)"
```

### Task 11: Wire DNA into auto_context pipeline

**Files:**
- Modify: `src/auto_context/mod.rs`

- [ ] **Step 1: Add `dna` field to `AutoContextResult`**

```rust
pub struct AutoContextResult {
    pub task: String,
    pub dna: String,  // NEW
    pub budget: BudgetSummary,
    pub sections: PackedSections,
    pub filtered_out: Vec<FilteredFile>,
}
```

- [ ] **Step 2: Add step 0 to pipeline**

Before query expansion:
```rust
let dna = crate::conventions::render::render_dna_section(&index.conventions);
let dna_tokens = counter.count(&dna);

// Budget tiering: skip DNA if budget < 2000, compact if < 5000
let (effective_dna, dna_token_cost) = if opts.tokens < 2000 {
    (String::new(), 0)
} else if opts.tokens < 5000 {
    let compact = render_compact_dna(&index.conventions); // top 3 conventions
    let cost = counter.count(&compact);
    (compact, cost)
} else {
    (dna, dna_tokens)
};

let remaining_budget = opts.tokens.saturating_sub(dna_token_cost);
// ... pass remaining_budget to allocate_and_pack()
```

- [ ] **Step 3: Write tests**

```rust
#[test]
fn test_auto_context_includes_dna() {
    // Build index with conventions → auto_context → dna field populated
}

#[test]
fn test_auto_context_dna_deducted_from_budget() {
    // 50k budget, DNA ~1000 tokens → remaining budget ~49k
}

#[test]
fn test_auto_context_tiny_budget_skips_dna() {
    // Budget 1k → dna field empty
}

#[test]
fn test_auto_context_small_budget_compact_dna() {
    // Budget 3k → compact dna (~300 tokens)
}
```

- [ ] **Step 4: Run all tests**

- [ ] **Step 5: Commit**

```bash
git commit -m "feat: DNA section in auto_context (step 0, never degraded, budget-tiered)"
```

---

## Stream 3: Verify

### Task 12: Implement diff scoping via git2

**Files:** `src/conventions/verify.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_diff_uncommitted_changes() {
    // Create repo, modify file, get diff → changed lines detected
}

#[test]
fn test_diff_against_ref() {
    // Create repo, commit, modify, diff against HEAD~1 → changes detected
}

#[test]
fn test_diff_focus_filter() {
    // Changes in src/ and tests/ → focus "src/" → only src/ changes
}

#[test]
fn test_diff_no_changes() {
    // Clean working tree → empty diff
}

#[test]
fn test_diff_new_file() {
    // Untracked new file → all lines are "added"
}
```

- [ ] **Step 2: Implement `get_changed_lines()`**

```rust
pub struct ChangedFile {
    pub path: String,
    pub added_lines: Vec<usize>,  // line numbers of added/modified lines
    pub is_new: bool,
}

pub fn get_changed_lines(
    repo_path: &Path,
    git_ref: Option<&str>,
    focus: Option<&str>,
) -> Result<Vec<ChangedFile>, String> {
    // Uses git2::Repository
    // No ref → diff_index_to_workdir (uncommitted)
    // ref → diff_tree_to_tree (ref..HEAD)
    // Filter by focus prefix
}
```

- [ ] **Step 3: Run tests**
- [ ] **Step 4: Commit**

```bash
git commit -m "feat: diff scoping via git2 for convention verification"
```

### Task 13: Implement convention checking + violation generation

**Files:** `src/conventions/verify.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_verify_naming_violation_high() {
    // Convention: snake_case (99%). New function PascalCase → high violation
}

#[test]
fn test_verify_unwrap_violation_with_history() {
    // Convention: no unwrap. Revert history. New unwrap → high + history evidence
}

#[test]
fn test_verify_import_violation() {
    // Convention: absolute imports. New relative import → high violation
}

#[test]
fn test_verify_visibility_trend_medium() {
    // Trend: 74% private. New public helper → medium violation
}

#[test]
fn test_verify_preexisting_not_reported() {
    // File has old unwrap on line 10. New code on line 50. Only line 50 checked.
}

#[test]
fn test_verify_all_passed() {
    // New code follows all conventions → passed list populated
}

#[test]
fn test_verify_suggestion_naming() {
    // PascalCase function → suggestion: "Rename to snake_case_name"
}

#[test]
fn test_verify_suggestion_unwrap() {
    // unwrap() → suggestion with Result pattern
}

#[test]
fn test_verify_suggestion_null_for_architecture() {
    // Architecture violation → suggestion is null
}

#[test]
fn test_verify_evidence_has_counts() {
    // Every violation has count, total, percentage, strength
}
```

- [ ] **Step 2: Implement `verify_changes()`**

```rust
pub struct VerifyResult {
    pub files_checked: usize,
    pub lines_checked: usize,
    pub violations: Vec<Violation>,
    pub passed: Vec<String>,
    pub summary: ViolationSummary,
}

pub struct Violation {
    pub severity: String,  // "high", "medium", "low"
    pub category: String,
    pub location: String,  // "src/auth/jwt.rs:42"
    pub message: String,
    pub evidence: ViolationEvidence,
    pub suggestion: Option<String>,
}

pub fn verify_changes(
    changed_files: &[ChangedFile],
    index: &CodebaseIndex,
    repo_path: &Path,
) -> VerifyResult {
    // For each changed file:
    //   Re-parse to get fresh symbols
    //   Re-parse using LanguageRegistry (same as apply_incremental_update in watch.rs)
    //   use crate::parser::LanguageRegistry;
    //   For each NEW symbol (in added_lines range):
    //     Check naming → naming convention
    //     Check signature → error return type
    //     Check body → unwrap, imports, etc.
    //   For new files:
    //     Check dependency direction
    //     Check file naming
}
```

- [ ] **Step 3: Run tests**
- [ ] **Step 4: Commit**

```bash
git commit -m "feat: convention verification with violations, evidence, suggestions"
```

---

## Stream 4: MCP Wiring

### Task 14: Wire `cxpak_verify` MCP tool

**Files:** `src/commands/serve.rs`

- [ ] **Step 1: Add tool to tools/list (tool #12)**
- [ ] **Step 2: Implement handler**

```rust
// NOTE: In the actual serve.rs handler, `index` is behind a RwLock.
// The handler receives &CodebaseIndex via the existing read guard pattern.
// This pseudocode assumes `index` is already a &CodebaseIndex reference.
"cxpak_verify" => {
    let git_ref = args.get("ref").and_then(|r| r.as_str());
    let focus = args.get("focus").and_then(|f| f.as_str());

    let changed = crate::conventions::verify::get_changed_lines(repo_path, git_ref, focus)
        .map_err(|e| format!("Error: {e}"))?;

    if changed.is_empty() {
        return mcp_tool_result(id, &serde_json::to_string_pretty(&json!({
            "files_checked": 0, "lines_checked": 0,
            "violations": [], "passed": ["No changes detected"],
            "summary": {"high": 0, "medium": 0, "low": 0}
        })).unwrap_or_default());
    }

    let result = crate::conventions::verify::verify_changes(&changed, index, repo_path);
    mcp_tool_result(id, &serde_json::to_string_pretty(&serde_json::to_value(&result).unwrap()).unwrap_or_default())
}
```

- [ ] **Step 3: Add `POST /verify` HTTP endpoint**
- [ ] **Step 4: Write MCP round-trip test**
- [ ] **Step 5: Commit**

```bash
git commit -m "feat: add cxpak_verify MCP tool (#12)"
```

### Task 15: Wire `cxpak_conventions` MCP tool

**Files:** `src/commands/serve.rs`

- [ ] **Step 1: Add tool to tools/list (tool #13)**
- [ ] **Step 2: Implement handler**

Parse `category`, `strength`, `focus` params. When `focus` is provided, recompute by iterating per-file contributions filtered by prefix. Serialize filtered profile.

- [ ] **Step 3: Add `POST /conventions` HTTP endpoint**
- [ ] **Step 4: Write MCP round-trip test + filter tests**
- [ ] **Step 5: Update tools/list test to expect 13 tools**

NOTE: Task 14 adds tool #12 but does NOT update the tools/list test (it would be 12, not 13). Update the test HERE to 13 after BOTH tools are added. If pre-commit hooks require passing tests, temporarily skip the tools/list count assertion in Task 14's commit, or add both tools before running tests.

- [ ] **Step 6: Commit**

```bash
git commit -m "feat: add cxpak_conventions MCP tool (#13)"
```

---

## Stream 5: Integration + Documentation + QA

### Task 16: Integration tests

**Files:** Add tests

- [ ] **Step 1: Write end-to-end tests**

```rust
#[test]
fn test_auto_context_dna_then_verify() {
    // Build index → auto_context (verify DNA present) → modify file → verify (check violations)
}

#[test]
fn test_conventions_match_actual_codebase() {
    // Build index on cxpak itself → conventions profile should detect Rust patterns
}

#[test]
fn test_verify_after_auto_context_conventions_match() {
    // DNA shows "snake_case 99%" → verify flags PascalCase → same convention referenced
}

#[test]
fn test_incremental_update_conventions() {
    // Build index → add file → incremental update → conventions updated
}

#[test]
fn test_empty_repo_conventions() {
    // No files → default profile, no errors
}

#[test]
fn test_git_health_ttl_caching() {
    // Two verify calls within 60s → git health computed once
}
```

- [ ] **Step 2: Run all tests**

Run: `cargo test --verbose`

- [ ] **Step 3: Commit**

```bash
git commit -m "test: integration tests for v1.1.0 conventions + verify + DNA"
```

### Task 17: Documentation

**Files:** `README.md`, `.claude/CLAUDE.md`, `plugin/README.md`

- [ ] **Step 1: Update README.md**

Add "Repository DNA" section documenting:
- Convention extraction (8 categories)
- Pattern strength labels
- DNA in auto_context
- Verify workflow
- Conventions tool for exploration
- Update tool count to 13

- [ ] **Step 2: Update CLAUDE.md**

Add conventions module to architecture pipeline (between Intelligence and Auto Context). Document extraction sources and verify tool.

- [ ] **Step 3: Update plugin/README.md**

Update tool table to 13 tools. Add verify and conventions.

- [ ] **Step 4: Commit**

```bash
git commit -m "docs: document Repository DNA features for v1.1.0"
```

### Task 18: Version bump

- [ ] **Step 1: Bump to 1.1.0** in Cargo.toml, plugin.json, marketplace.json, ensure-cxpak

- [ ] **Step 2: Commit**

```bash
git commit -m "chore: bump version to 1.1.0"
```

### Task 19: Pre-Release QA + CI Validation

- [ ] **Step 1: Run full test suite** — `cargo test --verbose`
- [ ] **Step 2: Run clippy** — `cargo clippy --all-targets -- -D warnings`
- [ ] **Step 3: Run formatter** — `cargo fmt -- --check`
- [ ] **Step 4: Run coverage** — `cargo tarpaulin --verbose --all-features --workspace --timeout 120 --fail-under 90`

- [ ] **Step 5: Manual QA — conventions**

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_conventions","arguments":{}}}' | cargo run -- serve --mcp .
```
Verify: naming, errors, imports, dependencies, testing, visibility, functions, git_health all populated with real data from the cxpak codebase.

- [ ] **Step 6: Manual QA — DNA in auto_context**

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_auto_context","arguments":{"task":"fix a bug in the scanner"}}}' | cargo run -- serve --mcp .
```
Verify: `dna` field present with ~1000 tokens of convention summary.

- [ ] **Step 7: Manual QA — verify**

Create a test file with a PascalCase function and `.unwrap()`. Run:
```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_verify","arguments":{}}}' | cargo run -- serve --mcp .
```
Verify: violations reported for naming + unwrap with evidence and suggestions.

- [ ] **Step 8: Simulate CI**

```bash
cargo build --verbose && cargo test --verbose && cargo clippy --all-targets -- -D warnings && cargo fmt -- --check && cargo tarpaulin --verbose --all-features --workspace --timeout 120 --fail-under 90
```

- [ ] **Step 9: Tag and push**

```bash
git tag v1.1.0
git push origin main --tags
```

---

## Task Summary

| Stream | Tasks | Dependencies |
|---|---|---|
| 1. Core Types + Extraction | Tasks 1-9 | Sequential (scaffold → naming → imports → errors → deps → testing → visibility+functions → git health → orchestrator) |
| 2. DNA in Auto Context | Tasks 10-11 | Task 9 (needs ConventionProfile on index) |
| 3. Verify | Tasks 12-13 | Task 9 (needs conventions) + Task 1 (types) |
| 4. MCP Wiring | Tasks 14-15 | Tasks 11 + 13 |
| 5. Integration + QA | Tasks 16-19 | All prior |

**Parallelizable:** After Task 9, Streams 2 and 3 can run in parallel — DNA rendering and verify implementation are independent. Tasks 2-8 (individual extractors) can also be parallelized since each operates on different data.

**Critical path:** Task 1 → (Tasks 2-8 parallel) → Task 9 → (Tasks 10-11 ∥ Tasks 12-13) → Tasks 14-15 → Tasks 16-19

**Total: 19 tasks, 65 new tests, 100% branch coverage on `src/conventions/`, 95%+ on modified modules, 90%+ overall CI gate. Task 19 is the release gate.**
