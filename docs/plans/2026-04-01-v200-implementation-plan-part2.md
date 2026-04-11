# v2.0.0 "The Experience" Implementation Plan — Part 2 (Tasks 16-28)
> Continuation of Part 1. See `2026-04-01-v200-implementation-plan-part1.md` for Tasks 1-15.

---

## Task 16 — Onboarding Map Module (`src/intelligence/onboarding.rs`)

**Goal:** Produce a dependency-ordered, complexity-progressive reading guide for new engineers.

**Files:**
- `src/intelligence/onboarding.rs` (new)
- `src/intelligence/mod.rs` (add `pub mod onboarding`)

**Steps:**

1. Write failing test: `compute_onboarding_map` returns phases where each phase's files have no unresolved intra-phase dependencies (all their deps appear in prior phases or are external).
2. Write failing test: files within a phase are ordered by ascending `token_count` (simpler first).
3. Write failing test: phases are ordered by descending aggregate PageRank (most important module first).
4. Implement `compute_onboarding_map` — see Tasks 17-20 for subtasks.
5. Register `pub mod onboarding` in `src/intelligence/mod.rs`.
6. All tests green.

**Commands:**
```bash
cargo test intelligence::onboarding
```

---

## Task 17 — `OnboardingMap`, `OnboardingPhase`, `OnboardingFile` Types

**Files:**
- `src/intelligence/onboarding.rs`

**Type definitions:**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingMap {
    /// Total source files included (excludes test files and generated files).
    pub total_files: usize,
    /// Human-readable estimate, e.g. "~4h 20m" at 200 tokens/min.
    pub estimated_reading_time: String,
    /// Ordered reading phases. Read phase 0 before phase 1, etc.
    pub phases: Vec<OnboardingPhase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingPhase {
    /// Human-readable module name derived from the directory prefix,
    /// e.g. "src/intelligence" → "Intelligence".
    pub name: String,
    /// Raw directory prefix used to group files, e.g. "src/intelligence".
    pub module: String,
    /// One-sentence rationale for why this module comes in this position,
    /// e.g. "Core utilities depended on by all other modules."
    pub rationale: String,
    /// Files in this phase, ordered simpler → more complex.
    pub files: Vec<OnboardingFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingFile {
    /// Repo-relative path, e.g. "src/intelligence/pagerank.rs".
    pub path: String,
    /// Normalised PageRank score [0.0, 1.0].
    pub pagerank: f64,
    /// Up to 5 public symbols most worth reading, sorted by symbol_importance descending.
    pub symbols_to_focus_on: Vec<String>,
    /// Token count from the index.
    pub estimated_tokens: usize,
}
```

**Steps:**

1. Write the type definitions with `#[derive(Debug, Clone, Serialize, Deserialize)]` on each struct.
2. Add a unit test confirming `OnboardingMap` round-trips through `serde_json` with all fields preserved.
3. Test green.

---

## Task 18 — Topological Sort for Reading Order

**Files:**
- `src/intelligence/onboarding.rs`

**Goal:** Order files so every file's local dependencies appear before it in the reading sequence.

**Steps:**

1. Write failing test: given a 3-file graph `A → B → C` (A imports B, B imports C), the topological order places C before B before A (dependencies first, dependents last).
2. Write failing test: a cycle (`A ↔ B`) does not panic — both files appear in the output (cycle broken by lexicographic file path as tiebreak).
3. Implement `topological_sort_files(files: &[&str], graph: &DependencyGraph) -> Vec<String>` using Kahn's algorithm (BFS, in-degree based). On cycle detection (remaining nodes with in-degree > 0 after BFS exhaustion), append remaining nodes sorted lexicographically.
4. Tests green.

**Key function signature:**
```rust
/// Returns files in dependency-first order (leaves before importers).
/// Cycles are broken by appending remaining nodes in lexicographic order.
pub fn topological_sort_files(
    files: &[&str],
    graph: &DependencyGraph,
) -> Vec<String>
```

---

## Task 19 — Phase Grouping by Module with 7±2 Constraint

**Files:**
- `src/intelligence/onboarding.rs`

**Goal:** Group topologically-sorted files into phases, one per module directory prefix, with no phase exceeding 9 files (cognitive load cap from design spec). Phases ordered by descending aggregate PageRank so the most important module comes first.

**Steps:**

1. Write failing test: 20 files from module `src/foo` produce 3 phases (9, 9, 2 files) all named `"Foo"` with distinguishing suffix `"(1/3)"`, `"(2/3)"`, `"(3/3)"`.
2. Write failing test: modules are ordered by their aggregate PageRank sum, highest first.
3. Write failing test: files from different modules are never mixed within a phase.
4. Implement `group_into_phases`:
   - Compute module prefix per file: first two path segments (matching the `module_depth: 2` default from v1.2.0 `ArchitectureMap`).
   - Group files by module, preserving topological order within each group.
   - Sort module groups by `sum(pagerank)` descending.
   - Split any group with >9 files into sub-phases of ≤9, appending `"(N/M)"` suffix.
   - Assign `rationale` heuristically: phase 0 gets `"Core module depended on by all others."`, subsequent phases get `"Builds on {prior_phase_name}."` if there's a cross-module edge, or `"Independent module."` otherwise.
5. Tests green.

**Key function signature:**
```rust
fn group_into_phases(
    sorted_files: &[String],
    pagerank: &HashMap<String, f64>,
    graph: &DependencyGraph,
) -> Vec<OnboardingPhase>
```

---

## Task 20 — Estimated Reading Time Computation

**Files:**
- `src/intelligence/onboarding.rs`

**Goal:** Compute a human-readable reading time estimate from total token count across all onboarding files.

**Steps:**

1. Write failing test: 12 000 tokens at 200 tokens/min → `"~1h 0m"`.
2. Write failing test: 500 tokens → `"~3m"` (omit hours when 0).
3. Write failing test: 0 tokens → `"~0m"`.
4. Implement `format_reading_time(total_tokens: usize) -> String` — constant reading speed of 200 tokens/min. Format: `"~{h}h {m}m"` when h ≥ 1, `"~{m}m"` otherwise.
5. Implement `compute_onboarding_map(index: &CodebaseIndex) -> OnboardingMap`:
   - Exclude test files (use `index.test_map` keys as a blocklist) and generated/vendored files (reuse noise filter blocklist from `src/auto_context/noise.rs`).
   - Run `topological_sort_files` on remaining files.
   - Run `group_into_phases`.
   - For each file, populate `symbols_to_focus_on` with top-5 public symbols by `symbol_importance` (from `src/intelligence/pagerank.rs`).
   - Compute `total_files`, `estimated_reading_time`.
6. Tests green.

**Key function signatures:**
```rust
pub fn format_reading_time(total_tokens: usize) -> String

pub fn compute_onboarding_map(index: &CodebaseIndex) -> OnboardingMap
```

---

## Task 21 — WASM Plugin SDK Types (`src/plugin/mod.rs`)

**Goal:** Define the full public plugin contract. The WASM host runtime (Task 22) is gated behind the `plugins` feature; these types are always compiled.

**Files:**
- `src/plugin/mod.rs` (new module)
- `src/lib.rs` (add `pub mod plugin`)

**Type definitions (always compiled):**

```rust
use crate::index::IndexedFile;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Capabilities ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginCapability {
    /// Plugin calls `analyze(IndexSnapshot)` → `Vec<Finding>`.
    Analyzer,
    /// Plugin calls `detect(FileSnapshot)` → `Vec<Detection>`.
    Detector,
    /// Plugin registers a named output format, e.g. `"junit"`.
    OutputFormat(String),
}

// ── Snapshot types (host → plugin, serialised across WASM boundary) ──────────

/// Read-only view of the indexed codebase passed to `analyze()`.
/// Contains only files whose paths match the plugin's declared `file_patterns`.
/// File contents are present only when the manifest declares `needs_content: true`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexSnapshot {
    pub files: Vec<FileSnapshot>,
    /// Aggregate PageRank scores for all indexed files, path → score [0.0, 1.0].
    pub pagerank: HashMap<String, f64>,
    /// Total file count of the full codebase (before pattern filtering).
    pub total_files: usize,
}

/// Read-only view of a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSnapshot {
    pub path: String,
    pub language: Option<String>,
    pub token_count: usize,
    /// `None` unless plugin manifest declares `needs_content: true`.
    pub content: Option<String>,
    /// Public symbol names extracted during parsing.
    pub public_symbols: Vec<String>,
    /// Direct import paths as extracted by the parser.
    pub imports: Vec<String>,
}

// ── Output types (plugin → host) ─────────────────────────────────────────────

/// A codebase-level finding from `analyze()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// Short identifier, e.g. `"large-module"`.
    pub kind: String,
    /// Human-readable message.
    pub message: String,
    /// Optional file path this finding is associated with.
    pub path: Option<String>,
    /// Severity: `"error"` | `"warning"` | `"info"`.
    pub severity: String,
    /// Arbitrary plugin-defined metadata.
    pub metadata: HashMap<String, serde_json::Value>,
}

/// A file-level detection from `detect()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Detection {
    pub kind: String,
    pub message: String,
    /// 1-based line number, if applicable.
    pub line: Option<u32>,
    pub severity: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

// ── Plugin trait (host-side; WASM plugins implement the same interface via WIT) ─

/// Implemented by the host-side WASM wrapper.
/// Each loaded `.wasm` binary is wrapped in a `Box<dyn CxpakPlugin>`.
pub trait CxpakPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn capabilities(&self) -> Vec<PluginCapability>;
    /// Whole-codebase analysis. Return size is capped at 1 MB (serialised JSON).
    fn analyze(&self, index: &IndexSnapshot) -> Vec<Finding>;
    /// Single-file detection. Called once per matching file.
    fn detect(&self, file: &FileSnapshot) -> Vec<Detection>;
}
```

**Steps:**

1. Create `src/plugin/mod.rs` with the types above.
2. Add `pub mod plugin` to `src/lib.rs`.
3. Write unit test: `Finding` and `Detection` round-trip through `serde_json`.
4. Write unit test: `PluginCapability::OutputFormat("junit".to_string())` serialises to `{"OutputFormat":"junit"}`.
5. Tests green. No feature flag needed for these types — they compile in all configurations.

---

## Task 22 — Plugin Loader (wasmtime Integration, Feature-Gated)

**Files:**
- `src/plugin/loader.rs` (new)
- `src/plugin/mod.rs` (add `#[cfg(feature = "plugins")] pub mod loader`)
- `Cargo.toml` (add `wasmtime` dependency and `plugins` feature)

**Steps:**

1. Verify `wasmtime` is already in `Cargo.toml` from Task 1 (Part 1) with version `"28"`. Do NOT add a duplicate or downgrade.
2. Add feature: `plugins = ["dep:wasmtime"]` and add `"plugins"` to `[features] default`.
3. Write failing test (`#[cfg(feature = "plugins")]`): `load_plugin` on a non-existent path returns `Err` with message containing the path.
4. Write failing test: loading a WASM binary that exceeds 10 MB returns `Err("plugin too large")`.
5. Write failing test: a loaded plugin's `name()` and `version()` match the values returned by the WASM `cxpak_plugin_name` / `cxpak_plugin_version` exported functions.
6. Implement `PluginLoader` as a skeleton:

```rust
#[cfg(feature = "plugins")]
pub mod loader {
    use super::{CxpakPlugin, Detection, FileSnapshot, Finding, IndexSnapshot, PluginCapability};
    use std::path::Path;

    const MAX_PLUGIN_BYTES: u64 = 10 * 1024 * 1024; // 10 MB
    const MAX_RETURN_BYTES: usize = 1 * 1024 * 1024; // 1 MB

    pub struct PluginLoader {
        engine: wasmtime::Engine,
    }

    impl PluginLoader {
        pub fn new() -> anyhow::Result<Self> {
            let engine = wasmtime::Engine::default();
            Ok(Self { engine })
        }

        /// Load a `.wasm` plugin from disk.
        /// Validates file size, instantiates, and wraps in `WasmPlugin`.
        pub fn load(&self, path: &Path) -> anyhow::Result<Box<dyn CxpakPlugin>> {
            let meta = std::fs::metadata(path)?;
            if meta.len() > MAX_PLUGIN_BYTES {
                anyhow::bail!("plugin too large: {} bytes (max {MAX_PLUGIN_BYTES})", meta.len());
            }
            let module = wasmtime::Module::from_file(&self.engine, path)?;
            let mut store = wasmtime::Store::new(&self.engine, ());
            let instance = wasmtime::Instance::new(&mut store, &module, &[])?;
            Err(anyhow::anyhow!("WASM plugin instantiation: module loaded ({} bytes) but guest function binding is not yet implemented — this is the v2.0.0 skeleton", meta.len()))
        }
    }

    /// Host-side wrapper around a loaded WASM instance.
    struct WasmPlugin {
        name: String,
        version: String,
        capabilities: Vec<PluginCapability>,
        // store + instance hidden behind feature gate
    }

    impl CxpakPlugin for WasmPlugin {
        fn name(&self) -> &str { &self.name }
        fn version(&self) -> &str { &self.version }
        fn capabilities(&self) -> Vec<PluginCapability> { self.capabilities.clone() }
        fn analyze(&self, _index: &IndexSnapshot) -> Vec<Finding> { vec![] }
        fn detect(&self, _file: &FileSnapshot) -> Vec<Detection> { vec![] }
    }
}
```

7. Tests for path error and size limit green. The `load()` function returns `Err` with a descriptive message for the unfinished guest binding — no `todo!()` or `unimplemented!()` macros (CLAUDE.md compliance).

**Commands:**
```bash
cargo build --features plugins
cargo test --features plugins plugin::loader
```

---

## Task 23 — Plugin Manifest (`.cxpak/plugins.json` Schema, File Pattern Scoping, Content Stripping)

**Files:**
- `src/plugin/manifest.rs` (new)
- `src/plugin/mod.rs` (add `pub mod manifest`)

**Type definitions:**

```rust
use serde::{Deserialize, Serialize};

/// Root of `.cxpak/plugins.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginsManifest {
    pub plugins: Vec<PluginEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginEntry {
    /// Display name, must match `CxpakPlugin::name()`.
    pub name: String,
    /// Path to `.wasm` file, relative to the repo root.
    pub path: String,
    /// SHA-256 hex digest of the `.wasm` binary. Verified on load.
    pub checksum: String,
    /// Glob patterns scoping which files are visible to this plugin.
    /// e.g. `["**/*.py", "**/*.pyi"]`. Empty = no files visible.
    pub file_patterns: Vec<String>,
    /// When `true`, `FileSnapshot::content` is populated. Triggers a
    /// warning to the user on first load.
    #[serde(default)]
    pub needs_content: bool,
}
```

**Steps:**

1. Write failing test: `PluginsManifest` deserialises from JSON with a missing `needs_content` field (defaults to `false`).
2. Write failing test: `load_manifest` on a non-existent path returns `Ok(PluginsManifest { plugins: vec![] })` (graceful absent).
3. Write failing test: `build_index_snapshot` with `file_patterns: ["**/*.py"]` includes only `.py` files in `IndexSnapshot::files`.
4. Write failing test: when `needs_content: false`, `FileSnapshot::content` is `None`; when `true`, content is `Some(...)`.
5. Implement `load_manifest(repo_root: &Path) -> anyhow::Result<PluginsManifest>`.
6. Implement `build_index_snapshot(index: &CodebaseIndex, entry: &PluginEntry) -> IndexSnapshot` applying pattern filtering via the `glob` crate (already available transitively; add explicit dep `glob = "0.3"` only if not already present).
7. Implement `verify_checksum(path: &Path, expected: &str) -> anyhow::Result<()>` using `sha2` crate (add `sha2 = { version = "0.10", optional = true }` behind `plugins` feature).
8. Tests green.

**Commands:**
```bash
cargo test plugin::manifest
```

---

## Task 24 — Plugin Security (Size-Limited Returns, Content Access Warnings)

**Files:**
- `src/plugin/security.rs` (new)
- `src/plugin/mod.rs` (add `pub mod security`)

**Steps:**

1. Write failing test: `enforce_return_limit` on a `Vec<Finding>` whose total JSON serialisation exceeds 1 MB returns `Err("plugin return exceeded 1 MB limit")`.
2. Write failing test: within the limit, `enforce_return_limit` returns `Ok` with the original findings unchanged.
3. Write failing test: `warn_if_needs_content` returns a non-empty warning string when `needs_content: true` and returns `None` when `false`.
4. Implement:

```rust
use super::{Finding, Detection};
use crate::plugin::manifest::PluginEntry;

const MAX_RETURN_BYTES: usize = 1 * 1024 * 1024;

/// Reject oversized Finding payloads before they reach the host.
pub fn enforce_return_limit(findings: Vec<Finding>) -> anyhow::Result<Vec<Finding>> {
    let serialised = serde_json::to_vec(&findings)?;
    if serialised.len() > MAX_RETURN_BYTES {
        anyhow::bail!(
            "plugin return exceeded 1 MB limit ({} bytes)",
            serialised.len()
        );
    }
    Ok(findings)
}

/// Same guard for Detection payloads.
pub fn enforce_detection_limit(detections: Vec<Detection>) -> anyhow::Result<Vec<Detection>> {
    let serialised = serde_json::to_vec(&detections)?;
    if serialised.len() > MAX_RETURN_BYTES {
        anyhow::bail!(
            "plugin return exceeded 1 MB limit ({} bytes)",
            serialised.len()
        );
    }
    Ok(detections)
}

/// Returns a warning message when a plugin requests raw file content.
/// This is displayed to the user on first load so they can audit the plugin.
pub fn warn_if_needs_content(entry: &PluginEntry) -> Option<String> {
    if entry.needs_content {
        Some(format!(
            "Plugin '{}' requests raw file content. \
             Ensure you trust this plugin before proceeding. \
             Path: {}",
            entry.name, entry.path
        ))
    } else {
        None
    }
}
```

5. Tests green.

---

## Task 25 — Integration Tests for Visual Output (Valid HTML, Valid SVG, Valid Mermaid)

**Goal:** Regression guard ensuring `cxpak visual` produces well-formed output for all three text-based formats without requiring a browser or parser dependency.

**Files:**
- `tests/visual_output.rs` (new integration test file)

**Steps:**

1. Write test `visual_html_is_well_formed`: run `cxpak visual --type architecture --format html` on the fixture repo; assert output contains `<!DOCTYPE html>`, `<html`, `</html>`, and a closing `</body>` tag; assert output does NOT contain `undefined` or `NaN`.
2. Write test `visual_svg_has_required_structure`: run with `--format svg`; assert output contains `<svg`, `</svg>`, `xmlns="http://www.w3.org/2000/svg"`, at least one `<rect` or `<circle` element, and at least one `<text` element.
3. Write test `visual_mermaid_has_graph_directive`: run with `--format mermaid --type architecture`; assert output starts with `graph` or `flowchart` keyword; assert every `-->` arrow has non-empty left and right node identifiers (regex: `\w+ --> \w+`).
4. Write test `visual_output_is_deterministic`: run the architecture HTML command twice on the same fixture repo; assert both outputs are byte-for-byte identical (layout is pre-computed in Rust, not randomised).
5. Write test `visual_json_is_valid`: run with `--format json`; assert `serde_json::from_str::<serde_json::Value>` succeeds on the output.
6. All tests use `assert_cmd::Command::cargo_bin("cxpak")` and the fixture repo at `tests/fixtures/sample_repo` (already used by existing integration tests in Part 1).
7. Tests are marked `#[cfg(feature = "visual")]` to avoid failures when the feature is disabled.

**Commands:**
```bash
cargo test --features visual --test visual_output
```

---

## Task 26 — Integration Tests for Onboarding (Deterministic Order, Correct Phases)

**Files:**
- `tests/onboarding.rs` (new integration test file)

**Steps:**

1. Write test `onboarding_order_is_deterministic`: call `compute_onboarding_map` twice on the same `CodebaseIndex` built from the fixture repo; assert both results are identical (same phase names, same file order in each phase).
2. Write test `onboarding_phases_respect_dependency_order`: for each phase in the result, assert that no file in phase `i` imports a file that first appears in phase `j > i`. Verify by checking `index.graph.edges` for each file path.
3. Write test `onboarding_phase_size_constraint`: assert every phase contains between 1 and 9 files inclusive.
4. Write test `onboarding_excludes_test_files`: build the index for the fixture repo; assert no path from `index.test_map.keys()` appears in any `OnboardingFile::path`.
5. Write test `onboarding_symbols_sorted_by_importance`: for each `OnboardingFile`, assert `symbols_to_focus_on.len() <= 5`; assert the list is non-empty for files that have public symbols.
6. Write test `onboarding_reading_time_format`: assert `estimated_reading_time` matches regex `^~\d+(h \d+m|\d*m)$`.
7. Build `CodebaseIndex` directly in tests (no CLI subprocess needed) using `CodebaseIndex::build_with_content`.

**Commands:**
```bash
cargo test --test onboarding
```

---

## Task 27 — Version Bump to 2.0.0 Across All 4 Files

**Files (all 4 must be updated atomically):**
- `Cargo.toml` — `version = "2.0.0"`
- `plugin/.claude-plugin/plugin.json` — `"version": "2.0.0"`
- `.claude-plugin/marketplace.json` — `"version": "2.0.0"`
- _(regenerate `Cargo.lock` via `cargo check`)_

**Steps:**

1. Write test (unit, in `src/lib.rs`) that `env!("CARGO_PKG_VERSION") == "2.0.0"` — this fails until the bump is applied, acting as a canary.
2. Update `Cargo.toml`: `version = "1.1.0"` → `version = "2.0.0"`.
3. Update `plugin/.claude-plugin/plugin.json`: set `"version": "2.0.0"`.
4. Update `.claude-plugin/marketplace.json`: set `"version": "2.0.0"`.
5. Run `cargo check` to regenerate `Cargo.lock`.
6. Verify the canary test passes.
7. Commit all four files plus `Cargo.lock` together.

**Commands:**
```bash
cargo check
cargo test test_version
```

**Note:** Per `CLAUDE.md`: "When bumping version, update all four files listed under Claude Code Plugin above, then run `cargo check` to regenerate `Cargo.lock` and commit it BEFORE tagging."

---

## Task 28 — Full Test Suite and Coverage Gate

**Goal:** Maintain ≥90% line coverage after all v2.0.0 additions. No test regressions.

**Files:**
- `scripts/check-coverage.sh` (update coverage thresholds if needed)
- All new `*_test` modules and `tests/*.rs` files from Tasks 16-26

**Steps:**

1. Run the full test suite — no failures permitted:
   ```bash
   cargo test --all-features --verbose 2>&1 | tee /tmp/test-output.txt
   grep -E "FAILED|error\[" /tmp/test-output.txt && exit 1 || true
   ```

2. Run `cargo fmt -- --check` — zero formatting violations.

3. Run `cargo clippy --all-targets --all-features -- -D warnings` — zero warnings.

4. Run tarpaulin coverage:
   ```bash
   cargo tarpaulin --all-features --out Json --output-dir . \
     --exclude-files "src/parser/languages/*" \
     --timeout 300
   ```
   Assert overall line coverage ≥ 90%.

5. Coverage by new module — each must meet its minimum individually:

   | Module | Minimum Coverage |
   |--------|-----------------|
   | `src/intelligence/onboarding.rs` | 90% |
   | `src/plugin/mod.rs` | 95% (types only — trivially testable) |
   | `src/plugin/manifest.rs` | 90% |
   | `src/plugin/security.rs` | 100% (3 small functions) |
   | `src/plugin/loader.rs` | 70% (skeleton; load() returns Err for unfinished binding) |
   | `tests/visual_output.rs` | N/A (integration tests) |
   | `tests/onboarding.rs` | N/A (integration tests) |

6. Snapshot test for `compute_onboarding_map` on the fixture repo: serialise result to JSON, compare against committed snapshot at `tests/snapshots/onboarding_map.json`. If snapshot is absent, write it (first-run bootstrap). If present, fail on any diff. Update snapshot intentionally with `UPDATE_SNAPSHOTS=1 cargo test`.

7. Snapshot test for `visual --format mermaid --type architecture` on the fixture repo: compare against `tests/snapshots/architecture.mermaid`.

8. All tests green. Coverage gate passes. No `TODO`, `FIXME`, or `unimplemented!()` in new code (clippy `todo` lint enforced).

**Commands:**
```bash
cargo test --all-features --verbose
cargo fmt -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo tarpaulin --all-features --out Json --output-dir . --timeout 300
```

---

## Summary: Tasks 16-28 Dependency Order

```
Task 17 (types)
    └── Task 16 (module scaffold)
            ├── Task 18 (topo sort)
            ├── Task 19 (phase grouping)
            └── Task 20 (reading time + entry point)

Task 21 (plugin types)
    ├── Task 22 (loader skeleton)
    ├── Task 23 (manifest)
    └── Task 24 (security)

Task 25 (visual integration tests)  ← depends on Part 1 visual tasks
Task 26 (onboarding integration tests)  ← depends on Task 16-20
Task 27 (version bump)  ← depends on all implementation tasks complete
Task 28 (coverage gate)  ← depends on Task 27
```

**Execution order:** 17 → 18 → 19 → 20 → 16 (register module) → 21 → 22 → 23 → 24 → 25 → 26 → 27 → 28.

Tasks 21-24 are independent of Tasks 16-20 and can be developed in parallel.
