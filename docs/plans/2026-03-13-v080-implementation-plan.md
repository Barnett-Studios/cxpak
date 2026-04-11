# cxpak v0.8.0 — Integrations Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make cxpak available everywhere — GitHub PRs via Actions, all MCP-compatible tools via daemon mode, and IDE workflows via persistent indexing.

**Architecture:** Three features in priority order: (1) GitHub Action in a separate repo that runs `cxpak diff` on PRs and posts context as a collapsible comment, (2) Daemon mode with file watching (`notify`), in-memory incremental index, HTTP API (`axum`), and MCP server over stdio, behind a `daemon` feature flag, (3) Codebase narrative as a stretch goal using template-based generation from index signals.

**Tech Stack:** Rust, clap, tree-sitter, notify 7, axum 0.8, tokio 1, serde_json (existing), GitHub Actions composite action

---

## Phase 1: GitHub Action (separate repo)

### Task 1: Create cxpak-action repository scaffold

**Files:**
- Create: `../cxpak-action/action.yml`
- Create: `../cxpak-action/README.md`
- Create: `../cxpak-action/.github/workflows/test.yml`

**Step 1: Create the directory and action.yml**

```bash
mkdir -p ../cxpak-action
```

```yaml
# ../cxpak-action/action.yml
name: 'cxpak Context'
description: 'Post cxpak diff context as a PR comment'
branding:
  icon: 'package'
  color: 'blue'
inputs:
  tokens:
    description: 'Token budget for the context output'
    default: '30k'
  format:
    description: 'Output format (markdown, json, xml)'
    default: 'markdown'
  focus:
    description: 'Boost files under this path prefix'
    required: false
  version:
    description: 'cxpak version to install (e.g. 0.7.0, or latest)'
    default: 'latest'
runs:
  using: 'composite'
  steps:
    - name: Install cxpak
      shell: bash
      run: |
        VERSION="${{ inputs.version }}"
        if [ "$VERSION" = "latest" ]; then
          DOWNLOAD_URL=$(curl -sL https://api.github.com/repos/lyubomir-bozhinov/cxpak/releases/latest \
            | grep browser_download_url | grep x86_64-unknown-linux-gnu | head -1 | cut -d'"' -f4)
        else
          DOWNLOAD_URL="https://github.com/lyubomir-bozhinov/cxpak/releases/download/v${VERSION}/cxpak-x86_64-unknown-linux-gnu.tar.gz"
        fi
        curl -sSL "$DOWNLOAD_URL" | tar xz
        sudo mv cxpak /usr/local/bin/
        cxpak --version

    - name: Run cxpak diff
      shell: bash
      run: |
        ARGS="diff --tokens ${{ inputs.tokens }} --format ${{ inputs.format }}"
        if [ -n "${{ inputs.focus }}" ]; then
          ARGS="$ARGS --focus ${{ inputs.focus }}"
        fi
        ARGS="$ARGS --git-ref origin/${{ github.base_ref }}"
        cxpak $ARGS > /tmp/cxpak-output.md 2>/tmp/cxpak-err.log || true
        if [ ! -s /tmp/cxpak-output.md ]; then
          echo "cxpak produced no output. stderr:" >&2
          cat /tmp/cxpak-err.log >&2
          echo "_No context changes detected._" > /tmp/cxpak-output.md
        fi

    - name: Post PR comment
      shell: bash
      env:
        GH_TOKEN: ${{ github.token }}
      run: |
        PR_NUMBER="${{ github.event.pull_request.number }}"
        REPO="${{ github.repository }}"

        # Delete previous cxpak comment if exists (idempotent update)
        COMMENT_ID=$(gh api "repos/${REPO}/issues/${PR_NUMBER}/comments" \
          --jq '.[] | select(.body | startswith("<!-- cxpak -->")) | .id' 2>/dev/null | head -1)
        if [ -n "$COMMENT_ID" ]; then
          gh api "repos/${REPO}/issues/comments/${COMMENT_ID}" -X DELETE 2>/dev/null || true
        fi

        # Build comment with collapsible details
        {
          echo "<!-- cxpak -->"
          echo "<details><summary>📦 cxpak context (click to expand)</summary>"
          echo ""
          cat /tmp/cxpak-output.md
          echo ""
          echo "</details>"
        } > /tmp/cxpak-comment.md

        gh pr comment "$PR_NUMBER" --body-file /tmp/cxpak-comment.md
```

**Step 2: Create README.md**

```markdown
# cxpak-action

GitHub Action that runs [cxpak](https://github.com/lyubomir-bozhinov/cxpak) `diff` on pull requests and posts token-budgeted context as a collapsible PR comment.

## Usage

Add to `.github/workflows/cxpak.yml`:

\`\`\`yaml
name: cxpak context
on:
  pull_request:
    types: [opened, synchronize]

permissions:
  contents: read
  pull-requests: write

jobs:
  context:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - uses: lyubomir-bozhinov/cxpak-action@v1
        with:
          tokens: 30k
\`\`\`

## Inputs

| Input     | Default    | Description                        |
|-----------|------------|------------------------------------|
| `tokens`  | `30k`      | Token budget for context output    |
| `format`  | `markdown` | Output format                      |
| `focus`   | —          | Boost files under this path prefix |
| `version` | `latest`   | cxpak version to install           |

## How it works

1. Downloads the cxpak binary from GitHub Releases
2. Runs `cxpak diff --git-ref origin/<base-branch>` against the PR's base
3. Posts output as a collapsible `<details>` comment on the PR
4. On subsequent pushes, replaces the previous comment (no spam)
```

**Step 3: Initialize git repo, commit, push**

```bash
cd ../cxpak-action
git init
git add action.yml README.md
git commit -m "feat: initial cxpak GitHub Action"
```

**Step 4: Verify action works**

- Create the `lyubomir-bozhinov/cxpak-action` repo on GitHub
- Push the code
- Tag `v1` for marketplace usage
- Add `uses: lyubomir-bozhinov/cxpak-action@v1` to cxpak's own `.github/workflows/` to dogfood

**Step 5: Commit dogfood workflow in cxpak repo**

Create `.github/workflows/cxpak-context.yml` in the cxpak repo:

```yaml
name: cxpak context
on:
  pull_request:
    types: [opened, synchronize]

permissions:
  contents: read
  pull-requests: write

jobs:
  context:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - uses: lyubomir-bozhinov/cxpak-action@v1
        with:
          tokens: 30k
```

```bash
git add .github/workflows/cxpak-context.yml
git commit -m "ci: add cxpak-action dogfood workflow for PRs"
```

---

## Phase 2: Daemon Mode — Foundation

### Task 2: Add daemon dependencies and feature flag

**Files:**
- Modify: `Cargo.toml`

**Step 1: Add dependencies behind `daemon` feature flag**

Add to `Cargo.toml`:

```toml
# Under [dependencies]
notify = { version = "7", optional = true }
axum = { version = "0.8", optional = true }
tokio = { version = "1", features = ["full"], optional = true }

# Under [features]
daemon = ["dep:notify", "dep:axum", "dep:tokio"]
```

Do NOT add `daemon` to the `default` feature list — keep the CLI lean.

**Step 2: Verify it builds both ways**

```bash
cargo build                     # without daemon — must still work
cargo build --features daemon   # with daemon — new deps compile
```

**Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "feat: add daemon feature flag with notify, axum, tokio deps"
```

---

### Task 3: Incremental index — `IndexedFile` add/remove/update

The current `CodebaseIndex::build()` is batch-only. We need methods to update a single file in-place for daemon mode.

**Files:**
- Modify: `src/index/mod.rs`
- Test: `src/index/mod.rs` (inline tests)

**Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `src/index/mod.rs`:

```rust
#[test]
fn test_upsert_file_adds_new() {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let fp = dir.path().join("a.rs");
    std::fs::write(&fp, "fn a() {}").unwrap();
    let files = vec![ScannedFile {
        relative_path: "a.rs".into(),
        absolute_path: fp,
        language: Some("rust".into()),
        size_bytes: 9,
    }];
    let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
    assert_eq!(index.files.len(), 1);

    // Upsert a new file
    index.upsert_file(
        "b.rs",
        Some("rust"),
        "fn b() {}",
        None,
        &counter,
    );
    assert_eq!(index.files.len(), 2);
    assert_eq!(index.total_files, 2);
    let b = index.files.iter().find(|f| f.relative_path == "b.rs").unwrap();
    assert!(b.content.contains("fn b()"));
}

#[test]
fn test_upsert_file_updates_existing() {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let fp = dir.path().join("a.rs");
    std::fs::write(&fp, "fn a() {}").unwrap();
    let files = vec![ScannedFile {
        relative_path: "a.rs".into(),
        absolute_path: fp,
        language: Some("rust".into()),
        size_bytes: 9,
    }];
    let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);

    // Upsert the same file with new content
    index.upsert_file(
        "a.rs",
        Some("rust"),
        "fn a_v2() { /* updated */ }",
        None,
        &counter,
    );
    assert_eq!(index.files.len(), 1);
    assert!(index.files[0].content.contains("a_v2"));
}

#[test]
fn test_remove_file() {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let fp1 = dir.path().join("a.rs");
    let fp2 = dir.path().join("b.rs");
    std::fs::write(&fp1, "fn a() {}").unwrap();
    std::fs::write(&fp2, "fn b() {}").unwrap();
    let files = vec![
        ScannedFile {
            relative_path: "a.rs".into(),
            absolute_path: fp1,
            language: Some("rust".into()),
            size_bytes: 9,
        },
        ScannedFile {
            relative_path: "b.rs".into(),
            absolute_path: fp2,
            language: Some("rust".into()),
            size_bytes: 9,
        },
    ];
    let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
    assert_eq!(index.files.len(), 2);

    index.remove_file("a.rs");
    assert_eq!(index.files.len(), 1);
    assert_eq!(index.total_files, 1);
    assert_eq!(index.files[0].relative_path, "b.rs");
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test --lib index::tests::test_upsert -- --nocapture
```

Expected: compile error — `upsert_file` method does not exist.

**Step 3: Implement `upsert_file` and `remove_file`**

Add to `impl CodebaseIndex` in `src/index/mod.rs`:

```rust
/// Insert or update a single file in the index.
///
/// If a file with the same `relative_path` already exists, it is replaced.
/// Language stats and totals are recomputed.
pub fn upsert_file(
    &mut self,
    relative_path: &str,
    language: Option<&str>,
    content: &str,
    parse_result: Option<ParseResult>,
    counter: &TokenCounter,
) {
    // Remove old entry if it exists (adjusts stats)
    self.remove_file(relative_path);

    let token_count = counter.count_or_zero(content);
    let size_bytes = content.len() as u64;

    if let Some(lang) = language {
        let stats = self
            .language_stats
            .entry(lang.to_string())
            .or_insert(LanguageStats {
                file_count: 0,
                total_bytes: 0,
                total_tokens: 0,
            });
        stats.file_count += 1;
        stats.total_bytes += size_bytes;
        stats.total_tokens += token_count;
    }

    self.total_tokens += token_count;
    self.total_bytes += size_bytes;

    self.files.push(IndexedFile {
        relative_path: relative_path.to_string(),
        language: language.map(|s| s.to_string()),
        size_bytes,
        token_count,
        parse_result,
        content: content.to_string(),
    });

    self.total_files = self.files.len();
}

/// Remove a file from the index by relative path.
///
/// Adjusts language stats and totals. No-op if the file is not present.
pub fn remove_file(&mut self, relative_path: &str) {
    if let Some(pos) = self
        .files
        .iter()
        .position(|f| f.relative_path == relative_path)
    {
        let removed = self.files.swap_remove(pos);
        self.total_tokens = self.total_tokens.saturating_sub(removed.token_count);
        self.total_bytes = self.total_bytes.saturating_sub(removed.size_bytes);

        if let Some(lang) = &removed.language {
            if let Some(stats) = self.language_stats.get_mut(lang) {
                stats.file_count = stats.file_count.saturating_sub(1);
                stats.total_bytes = stats.total_bytes.saturating_sub(removed.size_bytes);
                stats.total_tokens = stats.total_tokens.saturating_sub(removed.token_count);
                if stats.file_count == 0 {
                    self.language_stats.remove(lang);
                }
            }
        }

        self.total_files = self.files.len();
    }
}
```

**Step 4: Run tests to verify they pass**

```bash
cargo test --lib index::tests -- --nocapture
```

Expected: all tests pass including `test_upsert_file_adds_new`, `test_upsert_file_updates_existing`, `test_remove_file`.

**Step 5: Commit**

```bash
git add src/index/mod.rs
git commit -m "feat: add upsert_file and remove_file for incremental index updates"
```

---

### Task 4: Incremental graph — add/remove edges for a single file

When a file is re-parsed, we need to remove its old graph edges and add new ones without rebuilding the entire graph.

**Files:**
- Modify: `src/index/graph.rs`
- Test: `src/index/graph.rs` (inline tests)

**Step 1: Write the failing tests**

Add to `#[cfg(test)] mod tests` in `src/index/graph.rs`:

```rust
#[test]
fn test_remove_edges_for_file() {
    let mut graph = DependencyGraph::new();
    graph.add_edge("a.rs", "b.rs");
    graph.add_edge("a.rs", "c.rs");
    graph.add_edge("d.rs", "b.rs");

    graph.remove_edges_for("a.rs");

    // a.rs edges should be gone
    assert!(graph.edges.get("a.rs").map_or(true, |s| s.is_empty()));
    // b.rs should only have d.rs as dependent now
    let b_deps = graph.dependents("b.rs");
    assert_eq!(b_deps.len(), 1);
    assert!(b_deps.contains(&"d.rs"));
    // c.rs should have no dependents
    assert!(graph.dependents("c.rs").is_empty());
}

#[test]
fn test_remove_edges_for_nonexistent() {
    let mut graph = DependencyGraph::new();
    graph.add_edge("a.rs", "b.rs");
    graph.remove_edges_for("z.rs"); // no-op
    assert_eq!(graph.edges["a.rs"].len(), 1);
}

#[test]
fn test_remove_and_readd_edges() {
    let mut graph = DependencyGraph::new();
    graph.add_edge("a.rs", "b.rs");
    graph.add_edge("a.rs", "c.rs");

    // Simulate re-parse: remove old, add new
    graph.remove_edges_for("a.rs");
    graph.add_edge("a.rs", "d.rs");

    assert_eq!(graph.edges["a.rs"].len(), 1);
    assert!(graph.edges["a.rs"].contains("d.rs"));
    assert!(graph.dependents("b.rs").is_empty());
    assert!(graph.dependents("c.rs").is_empty());
    assert_eq!(graph.dependents("d.rs"), vec!["a.rs"]);
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test --lib index::graph::tests::test_remove_edges -- --nocapture
```

Expected: compile error — `remove_edges_for` does not exist.

**Step 3: Implement `remove_edges_for`**

Add to `impl DependencyGraph` in `src/index/graph.rs`:

```rust
/// Remove all outgoing edges from `source` and clean up corresponding reverse edges.
///
/// Used during incremental re-indexing: call this before re-adding the new
/// edges from a freshly parsed file.
pub fn remove_edges_for(&mut self, source: &str) {
    if let Some(targets) = self.edges.remove(source) {
        for target in &targets {
            if let Some(rev) = self.reverse_edges.get_mut(target.as_str()) {
                rev.remove(source);
                if rev.is_empty() {
                    self.reverse_edges.remove(target.as_str());
                }
            }
        }
    }
}
```

**Step 4: Run tests to verify they pass**

```bash
cargo test --lib index::graph::tests -- --nocapture
```

Expected: all pass.

**Step 5: Commit**

```bash
git add src/index/graph.rs
git commit -m "feat: add remove_edges_for for incremental graph updates"
```

---

### Task 5: File watcher module

**Files:**
- Create: `src/daemon/watcher.rs`
- Create: `src/daemon/mod.rs`
- Modify: `src/lib.rs` — add `pub mod daemon` behind cfg

**Step 1: Write the failing test**

Create `src/daemon/watcher.rs`:

```rust
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

/// Debounced file change events from the file system.
pub enum FileChange {
    Modified(PathBuf),
    Created(PathBuf),
    Removed(PathBuf),
}

/// Watches a directory for file changes with debouncing.
pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    receiver: mpsc::Receiver<FileChange>,
}

impl FileWatcher {
    /// Start watching `root` for file changes.
    ///
    /// Changes are debounced: rapid successive events on the same file
    /// are collapsed into one.
    pub fn new(root: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, rx) = mpsc::channel();

        let sender = tx.clone();
        let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
            if let Ok(event) = res {
                for path in event.paths {
                    let change = match event.kind {
                        EventKind::Create(_) => FileChange::Created(path),
                        EventKind::Modify(_) => FileChange::Modified(path),
                        EventKind::Remove(_) => FileChange::Removed(path),
                        _ => continue,
                    };
                    let _ = sender.send(change);
                }
            }
        })?;

        watcher.watch(root, RecursiveMode::Recursive)?;

        Ok(Self {
            _watcher: watcher,
            receiver: rx,
        })
    }

    /// Receive the next file change event, blocking up to `timeout`.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<FileChange> {
        self.receiver.recv_timeout(timeout).ok()
    }

    /// Drain all pending events (non-blocking).
    pub fn drain(&self) -> Vec<FileChange> {
        let mut events = Vec::new();
        while let Ok(change) = self.receiver.try_recv() {
            events.push(change);
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_watcher_detects_file_create() {
        let dir = tempfile::TempDir::new().unwrap();
        let watcher = FileWatcher::new(dir.path()).unwrap();

        // Create a file
        let file = dir.path().join("new.rs");
        fs::write(&file, "fn new() {}").unwrap();

        // Give the watcher time to detect
        std::thread::sleep(Duration::from_millis(200));
        let events = watcher.drain();
        assert!(
            !events.is_empty(),
            "watcher should detect file creation"
        );
    }

    #[test]
    fn test_watcher_detects_file_modify() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("existing.rs");
        fs::write(&file, "fn v1() {}").unwrap();

        let watcher = FileWatcher::new(dir.path()).unwrap();

        // Modify the file
        fs::write(&file, "fn v2() {}").unwrap();

        std::thread::sleep(Duration::from_millis(200));
        let events = watcher.drain();
        assert!(
            !events.is_empty(),
            "watcher should detect file modification"
        );
    }

    #[test]
    fn test_watcher_detects_file_remove() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("doomed.rs");
        fs::write(&file, "fn doomed() {}").unwrap();

        let watcher = FileWatcher::new(dir.path()).unwrap();

        fs::remove_file(&file).unwrap();

        std::thread::sleep(Duration::from_millis(200));
        let events = watcher.drain();
        assert!(
            !events.is_empty(),
            "watcher should detect file removal"
        );
    }
}
```

Create `src/daemon/mod.rs`:

```rust
pub mod watcher;
```

**Step 2: Wire it up in lib.rs**

Add to `src/lib.rs`:

```rust
#[cfg(feature = "daemon")]
pub mod daemon;
```

**Step 3: Run tests to verify they pass**

```bash
cargo test --features daemon --lib daemon::watcher::tests -- --nocapture
```

Expected: all 3 tests pass.

**Step 4: Commit**

```bash
git add src/daemon/ src/lib.rs
git commit -m "feat: add file watcher module with notify integration"
```

---

### Task 6: `cxpak watch` command

**Files:**
- Create: `src/commands/watch.rs`
- Modify: `src/commands/mod.rs`
- Modify: `src/cli/mod.rs`
- Modify: `src/main.rs`

**Step 1: Create `src/commands/watch.rs`**

```rust
use crate::budget::counter::TokenCounter;
use crate::cache::FileCache;
use crate::cli::OutputFormat;
use crate::daemon::watcher::{FileChange, FileWatcher};
use crate::index::CodebaseIndex;
use crate::parser::LanguageRegistry;
use crate::scanner::Scanner;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

pub fn run(
    path: &Path,
    token_budget: usize,
    format: &OutputFormat,
    verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let counter = TokenCounter::new();
    let registry = LanguageRegistry::new();

    // Initial full build
    let scanner = Scanner::new(path)?;
    let files = scanner.scan()?;

    let cache_dir = path.join(".cxpak");
    let cache = FileCache::load(&cache_dir);
    let cache_map = cache.as_map();

    // Parse all files
    let mut parse_results = HashMap::new();
    let mut content_map = HashMap::new();
    for file in &files {
        let source = std::fs::read_to_string(&file.absolute_path).unwrap_or_default();
        if let Some(lang_name) = &file.language {
            if let Some(lang) = registry.get(lang_name) {
                let ts_lang = lang.ts_language();
                let mut parser = tree_sitter::Parser::new();
                parser.set_language(&ts_lang).ok();
                if let Some(tree) = parser.parse(&source, None) {
                    let result = lang.extract(&source, &tree);
                    parse_results.insert(file.relative_path.clone(), result);
                }
            }
        }
        content_map.insert(file.relative_path.clone(), source);
    }

    let mut index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);

    eprintln!(
        "cxpak: watching {} ({} files indexed, {} tokens)",
        path.display(),
        index.total_files,
        index.total_tokens
    );

    // Start watching
    let watcher = FileWatcher::new(path)?;

    loop {
        // Block until a change comes in, then drain all pending
        if let Some(first) = watcher.recv_timeout(Duration::from_secs(1)) {
            let mut changes = vec![first];
            // Small debounce: wait a bit then drain
            std::thread::sleep(Duration::from_millis(50));
            changes.extend(watcher.drain());

            let mut modified_paths = std::collections::HashSet::new();
            let mut removed_paths = std::collections::HashSet::new();

            for change in changes {
                match change {
                    FileChange::Created(p) | FileChange::Modified(p) => {
                        if let Ok(rel) = p.strip_prefix(path) {
                            modified_paths.insert(rel.to_string_lossy().to_string());
                        }
                    }
                    FileChange::Removed(p) => {
                        if let Ok(rel) = p.strip_prefix(path) {
                            removed_paths.insert(rel.to_string_lossy().to_string());
                        }
                    }
                }
            }

            let start = std::time::Instant::now();
            let mut update_count = 0;

            for rel_path in &removed_paths {
                index.remove_file(rel_path);
                update_count += 1;
            }

            for rel_path in &modified_paths {
                if removed_paths.contains(rel_path) {
                    continue;
                }
                let abs_path = path.join(rel_path);
                if let Ok(content) = std::fs::read_to_string(&abs_path) {
                    let lang_name = crate::scanner::detect_language(rel_path);
                    let parse_result = lang_name.as_deref().and_then(|ln| {
                        registry.get(ln).and_then(|lang| {
                            let ts_lang = lang.ts_language();
                            let mut parser = tree_sitter::Parser::new();
                            parser.set_language(&ts_lang).ok()?;
                            let tree = parser.parse(&content, None)?;
                            Some(lang.extract(&content, &tree))
                        })
                    });

                    index.upsert_file(
                        rel_path,
                        lang_name.as_deref(),
                        &content,
                        parse_result,
                        &counter,
                    );
                    update_count += 1;
                }
            }

            if update_count > 0 {
                eprintln!(
                    "cxpak: updated {} file(s) ({:.0?}), {} files / {} tokens total",
                    update_count,
                    start.elapsed(),
                    index.total_files,
                    index.total_tokens
                );
            }
        }
    }
}
```

**Step 2: Add CLI subcommand**

Add to `Commands` enum in `src/cli/mod.rs`:

```rust
/// Watch for file changes and keep index hot
#[cfg(feature = "daemon")]
Watch {
    #[arg(long, default_value = "50k")]
    tokens: String,
    #[arg(long, default_value = "markdown")]
    format: OutputFormat,
    #[arg(long)]
    verbose: bool,
    #[arg(default_value = ".")]
    path: PathBuf,
},
```

**Step 3: Add to `src/commands/mod.rs`**

```rust
#[cfg(feature = "daemon")]
pub mod watch;
```

**Step 4: Wire up in `src/main.rs`**

Add match arm:

```rust
#[cfg(feature = "daemon")]
Commands::Watch {
    tokens,
    format,
    verbose,
    path,
} => {
    let token_budget = match parse_token_count(tokens) {
        Ok(0) => {
            eprintln!("Error: --tokens must be greater than 0");
            std::process::exit(1);
        }
        Ok(n) => n,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };
    commands::watch::run(path, token_budget, format, *verbose)
}
```

**Step 5: Build and smoke test**

```bash
cargo build --features daemon
cargo run --features daemon -- watch --tokens 50k .
# In another terminal: touch src/lib.rs
# Expect: "cxpak: updated 1 file(s) (...), N files / N tokens total"
# Ctrl-C to stop
```

**Step 6: Commit**

```bash
git add src/commands/watch.rs src/commands/mod.rs src/cli/mod.rs src/main.rs
git commit -m "feat: add cxpak watch command with incremental re-indexing"
```

---

### Task 7: HTTP server (`cxpak serve`)

**Files:**
- Create: `src/commands/serve.rs`
- Modify: `src/commands/mod.rs`
- Modify: `src/cli/mod.rs`
- Modify: `src/main.rs`

This task creates an axum HTTP server that keeps a hot index and serves overview/trace/diff queries. This is the largest single task — it wires together the watcher, incremental index, and a query API.

**Step 1: Create `src/commands/serve.rs`**

The server holds a shared `Arc<RwLock<CodebaseIndex>>` and `Arc<RwLock<DependencyGraph>>`, updated by a background watcher thread. HTTP routes query the locked index.

Key routes:
- `GET /health` → `{"status": "ok"}`
- `GET /stats` → `{"files": N, "tokens": N, "last_update": "..."}`
- `GET /overview?tokens=50k&format=json&focus=src/auth`
- `GET /trace?target=my_fn&tokens=50k`

This is a substantial implementation. The code should use `axum::Router`, `tokio::spawn` for the watcher loop, and `Arc<RwLock<_>>` for shared state.

**Step 2: Add CLI subcommand in `src/cli/mod.rs`**

```rust
/// Start HTTP server with hot index
#[cfg(feature = "daemon")]
Serve {
    #[arg(long, default_value = "3000")]
    port: u16,
    #[arg(long, default_value = "50k")]
    tokens: String,
    #[arg(long)]
    verbose: bool,
    #[arg(default_value = ".")]
    path: PathBuf,
},
```

**Step 3: Wire up in main.rs, commands/mod.rs**

**Step 4: Test manually**

```bash
cargo run --features daemon -- serve --port 3000 .
# In another terminal:
curl http://localhost:3000/health
curl http://localhost:3000/stats
curl "http://localhost:3000/overview?tokens=10k&format=json"
```

**Step 5: Commit**

```bash
git add src/commands/serve.rs src/commands/mod.rs src/cli/mod.rs src/main.rs
git commit -m "feat: add cxpak serve command with HTTP API"
```

---

### Task 8: MCP server mode (`cxpak serve --mcp`)

**Files:**
- Modify: `src/commands/serve.rs`
- Modify: `src/cli/mod.rs`

**Step 1: Add `--mcp` flag to Serve command**

```rust
/// Run as MCP server over stdio instead of HTTP
#[arg(long)]
mcp: bool,
```

**Step 2: Implement MCP JSON-RPC over stdio**

When `--mcp` is passed, instead of starting an HTTP server:
- Read JSON-RPC requests from stdin
- Respond with tool results on stdout
- Expose three tools: `cxpak_overview`, `cxpak_trace`, `cxpak_diff`
- Follow the MCP protocol: `initialize`, `tools/list`, `tools/call`

**Step 3: Test with Claude Code config**

Add to Claude Code MCP settings:
```json
{
  "mcpServers": {
    "cxpak": {
      "command": "cxpak",
      "args": ["serve", "--mcp", "."]
    }
  }
}
```

**Step 4: Commit**

```bash
git add src/commands/serve.rs src/cli/mod.rs
git commit -m "feat: add MCP server mode (cxpak serve --mcp)"
```

---

## Phase 3: Release

### Task 9: Version bump and release

**Files:**
- Modify: `Cargo.toml` — version "0.7.0" → "0.8.0"
- Modify: `.github/workflows/release.yml` — add `--features daemon` to build

**Step 1: Update version**

```bash
sed -i '' 's/version = "0.7.0"/version = "0.8.0"/' Cargo.toml
```

**Step 2: Update release workflow to build with daemon feature**

In `.github/workflows/release.yml`, change the Build step:

```yaml
- name: Build
  run: cargo build --release --features daemon --target ${{ matrix.target }}
```

**Step 3: Run full test suite**

```bash
cargo test --verbose
cargo test --features daemon --verbose
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features daemon -- -D warnings
```

**Step 4: Commit and tag**

```bash
git add Cargo.toml Cargo.lock .github/workflows/release.yml
git commit -m "chore: bump version to 0.8.0"
git tag v0.8.0
```

**Step 5: Push to trigger release**

```bash
git push origin main --tags
```

---

## Phase 4: Stretch — Codebase Narrative

### Task 10: Template-based narrative section (optional)

**Files:**
- Create: `src/output/narrative.rs`
- Modify: `src/output/mod.rs`
- Modify: `src/commands/overview.rs`

Generate 3-5 sentences describing the project using signals from the index:
- Primary language (most files)
- Total files and tokens
- Entry point detection (main.rs, index.ts, etc.)
- Key dependencies from Cargo.toml / package.json
- Pipeline/architecture hints from module names

This is template-based — no LLM needed. Deferred to v0.8.1 if time is short.

---

## Summary

| Phase | Tasks | What ships |
|-------|-------|------------|
| 1     | 1     | GitHub Action (separate repo) |
| 2     | 2–8   | `watch` + `serve` + `serve --mcp` behind `daemon` feature |
| 3     | 9     | v0.8.0 release |
| 4     | 10    | Narrative section (stretch) |

**Critical path:** Tasks 2→3→4→5→6→7→8→9 (daemon mode is sequential).
Task 1 (GitHub Action) is fully independent and can be done in parallel.
