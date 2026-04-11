# cxpak v0.7.0 — Performance & DX Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Eliminate O(V*E) dependents lookup, remove double disk reads, default --tokens to 50k, and add --since flag for diff.

**Architecture:** Add reverse adjacency index to DependencyGraph for O(1) dependents. Pass file content from parse phase to index phase to avoid re-reading. Make --tokens optional with 50k default. Add time-expression parser that resolves to git commit for --since.

**Tech Stack:** Rust, git2, clap, tree-sitter (existing)

---

### Task 1: Reverse Adjacency Index — Failing Tests

**Files:**
- Modify: `src/index/graph.rs:3-6` (struct definition)
- Test: `src/index/graph.rs` (existing test module)

**Step 1: Write the failing test**

Add to `src/index/graph.rs` test module:

```rust
#[test]
fn test_reverse_edges_maintained() {
    let mut graph = DependencyGraph::new();
    graph.add_edge("a.rs", "b.rs");
    graph.add_edge("c.rs", "b.rs");
    graph.add_edge("a.rs", "d.rs");
    // reverse_edges should exist and be populated
    assert!(graph.reverse_edges.get("b.rs").unwrap().contains("a.rs"));
    assert!(graph.reverse_edges.get("b.rs").unwrap().contains("c.rs"));
    assert!(graph.reverse_edges.get("d.rs").unwrap().contains("a.rs"));
    assert!(graph.reverse_edges.get("b.rs").unwrap().len() == 2);
}

#[test]
fn test_dependents_uses_reverse_index() {
    // Same as existing test_dependents but verifies O(1) path works
    let mut graph = DependencyGraph::new();
    for i in 0..100 {
        graph.add_edge(&format!("file_{i}.rs"), "common.rs");
    }
    let deps = graph.dependents("common.rs");
    assert_eq!(deps.len(), 100);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib index::graph::tests::test_reverse_edges_maintained -- --nocapture`
Expected: FAIL — `reverse_edges` field doesn't exist

**Step 3: Commit**

```bash
git add src/index/graph.rs
git commit -m "test: add failing tests for reverse adjacency index"
```

---

### Task 2: Reverse Adjacency Index — Implementation

**Files:**
- Modify: `src/index/graph.rs:3-6` (add `reverse_edges` field)
- Modify: `src/index/graph.rs:13-18` (`add_edge` — maintain reverse index)
- Modify: `src/index/graph.rs:20-26` (`dependents` — use reverse lookup)
- Modify: `src/index/graph.rs:56-61` (`reachable_from` — use reverse lookup)

**Step 1: Implement the reverse adjacency index**

Change struct:
```rust
#[derive(Debug, Default)]
pub struct DependencyGraph {
    pub edges: HashMap<String, HashSet<String>>,
    reverse_edges: HashMap<String, HashSet<String>>,
}
```

Change `add_edge`:
```rust
pub fn add_edge(&mut self, from: &str, to: &str) {
    self.edges
        .entry(from.to_string())
        .or_default()
        .insert(to.to_string());
    self.reverse_edges
        .entry(to.to_string())
        .or_default()
        .insert(from.to_string());
}
```

Change `dependents`:
```rust
pub fn dependents(&self, path: &str) -> Vec<&str> {
    self.reverse_edges
        .get(path)
        .map(|set| set.iter().map(String::as_str).collect())
        .unwrap_or_default()
}
```

Change `reachable_from` lines 56-61 (the reverse edge scan):
```rust
// Follow incoming edges (files that import `current`)
if let Some(importers) = self.reverse_edges.get(&current) {
    for importer in importers {
        if visited.insert(importer.clone()) {
            queue.push_back(importer.clone());
        }
    }
}
```

**Step 2: Run all tests**

Run: `cargo test --lib index::graph -- --nocapture`
Expected: ALL PASS (11 existing + 2 new)

**Step 3: Run full test suite**

Run: `cargo test --verbose`
Expected: ALL PASS

**Step 4: Commit**

```bash
git add src/index/graph.rs
git commit -m "perf: add reverse adjacency index for O(1) dependents lookup"
```

---

### Task 3: Eliminate Double Disk Read — Failing Test

**Files:**
- Modify: `src/index/mod.rs` (test module)

**Step 1: Write failing test**

Add to `src/index/mod.rs` test module:

```rust
#[test]
fn test_build_with_content_map() {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let fp = dir.path().join("test.rs");
    std::fs::write(&fp, "fn hello() {}").unwrap();
    let files = vec![ScannedFile {
        relative_path: "test.rs".into(),
        absolute_path: fp.clone(),
        language: Some("rust".into()),
        size_bytes: 13,
    }];
    let mut content_map = HashMap::new();
    content_map.insert("test.rs".to_string(), "fn hello() {}".to_string());
    let index = CodebaseIndex::build_with_content(files, HashMap::new(), content_map, &counter);
    assert_eq!(index.total_files, 1);
    assert_eq!(index.files[0].content, "fn hello() {}");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib index::tests::test_build_with_content_map -- --nocapture`
Expected: FAIL — `build_with_content` doesn't exist

**Step 3: Commit**

```bash
git add src/index/mod.rs
git commit -m "test: add failing test for build_with_content (no double disk read)"
```

---

### Task 4: Eliminate Double Disk Read — Implementation

**Files:**
- Modify: `src/cache/parse.rs:31-36` (return type change)
- Modify: `src/cache/parse.rs:68-87` (always read content, return it)
- Modify: `src/index/mod.rs:37-41` (add `build_with_content` method)
- Modify: `src/commands/overview.rs` (pass content through)
- Modify: `src/commands/trace.rs` (pass content through)
- Modify: `src/commands/diff.rs` (pass content through)

**Step 1: Modify `parse_with_cache` return type**

Change signature to return `(HashMap<String, ParseResult>, HashMap<String, String>)` — parse results + content map.

In the parallel `.map()` closure (line 53), always read the file content:
- On cache miss: already reading at line 76-77. Keep the content.
- On cache hit: read content with `std::fs::read_to_string`.

Return `(Option<ParseResult>, CacheEntry, String)` from each parallel iteration.

After collecting, build both maps:
```rust
let mut parse_results: HashMap<String, ParseResult> = HashMap::new();
let mut content_map: HashMap<String, String> = HashMap::new();
// ...
content_map.insert(cache_entry.relative_path.clone(), content);
```

Return `(parse_results, content_map)`.

**Step 2: Add `build_with_content` to CodebaseIndex**

```rust
pub fn build_with_content(
    files: Vec<ScannedFile>,
    parse_results: HashMap<String, ParseResult>,
    content_map: HashMap<String, String>,
    counter: &TokenCounter,
) -> Self {
    // Same as build() but uses content_map instead of read_to_string
    for file in &files {
        let content = content_map
            .get(&file.relative_path)
            .cloned()
            .unwrap_or_else(|| std::fs::read_to_string(&file.absolute_path).unwrap_or_default());
        // ... rest same as build()
    }
}
```

Keep existing `build()` as-is for backward compatibility (tests use it).

**Step 3: Update callers**

In `overview.rs`, `trace.rs`, `diff.rs`: Change `parse_with_cache` call to destructure both return values, pass `content_map` to `CodebaseIndex::build_with_content`.

Example pattern:
```rust
let (parse_results, content_map) = crate::cache::parse::parse_with_cache(&files, path, &counter, verbose);
let index = CodebaseIndex::build_with_content(files, parse_results, content_map, &counter);
```

**Step 4: Run tests**

Run: `cargo test --verbose`
Expected: ALL PASS

**Step 5: Commit**

```bash
git add src/cache/parse.rs src/index/mod.rs src/commands/overview.rs src/commands/trace.rs src/commands/diff.rs
git commit -m "perf: eliminate double disk reads by passing content from parser to indexer"
```

---

### Task 5: Default --tokens to 50k — Failing Tests

**Files:**
- Modify: `src/cli/mod.rs` (test module)

**Step 1: Write failing tests**

Add to `src/cli/mod.rs` test module:

```rust
#[test]
fn test_overview_default_tokens() {
    let cli = Cli::try_parse_from(["cxpak", "overview"])
        .expect("should parse without --tokens");
    match cli.command {
        Commands::Overview { tokens, .. } => {
            assert_eq!(tokens, "50k");
        }
        _ => panic!("expected Overview"),
    }
}

#[test]
fn test_diff_default_tokens() {
    let cli = Cli::try_parse_from(["cxpak", "diff"])
        .expect("should parse without --tokens");
    match cli.command {
        Commands::Diff { tokens, .. } => {
            assert_eq!(tokens, "50k");
        }
        _ => panic!("expected Diff"),
    }
}

#[test]
fn test_trace_default_tokens() {
    let cli = Cli::try_parse_from(["cxpak", "trace", "my_symbol"])
        .expect("should parse without --tokens");
    match cli.command {
        Commands::Trace { tokens, .. } => {
            assert_eq!(tokens, "50k");
        }
        _ => panic!("expected Trace"),
    }
}

#[test]
fn test_tokens_override_still_works() {
    let cli = Cli::try_parse_from(["cxpak", "overview", "--tokens", "100k"])
        .expect("should parse with explicit --tokens");
    match cli.command {
        Commands::Overview { tokens, .. } => {
            assert_eq!(tokens, "100k");
        }
        _ => panic!("expected Overview"),
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib cli::tests::test_overview_default_tokens -- --nocapture`
Expected: FAIL — `--tokens` is required

**Step 3: Commit**

```bash
git add src/cli/mod.rs
git commit -m "test: add failing tests for default --tokens 50k"
```

---

### Task 6: Default --tokens to 50k — Implementation

**Files:**
- Modify: `src/cli/mod.rs:19-20` (Overview tokens field)
- Modify: `src/cli/mod.rs:43-44` (Diff tokens field)
- Modify: `src/cli/mod.rs:67-68` (Trace tokens field)

**Step 1: Add default_value to all three commands**

Change in Overview:
```rust
#[arg(long, default_value = "50k")]
tokens: String,
```

Same change in Diff and Trace.

**Step 2: Run tests**

Run: `cargo test --verbose`
Expected: ALL PASS (existing + 4 new)

**Step 3: Commit**

```bash
git add src/cli/mod.rs
git commit -m "dx: default --tokens to 50k for all commands"
```

---

### Task 7: --since Flag — Failing Tests

**Files:**
- Modify: `src/commands/diff.rs` (test module)

**Step 1: Write failing tests for time expression parsing**

Add to `src/commands/diff.rs`:

```rust
#[test]
fn test_parse_time_expression_days() {
    assert_eq!(parse_time_expression("1 day").unwrap().as_secs(), 86400);
    assert_eq!(parse_time_expression("2 days").unwrap().as_secs(), 172800);
    assert_eq!(parse_time_expression("1d").unwrap().as_secs(), 86400);
    assert_eq!(parse_time_expression("3d").unwrap().as_secs(), 259200);
}

#[test]
fn test_parse_time_expression_hours() {
    assert_eq!(parse_time_expression("1 hour").unwrap().as_secs(), 3600);
    assert_eq!(parse_time_expression("3 hours").unwrap().as_secs(), 10800);
    assert_eq!(parse_time_expression("1h").unwrap().as_secs(), 3600);
}

#[test]
fn test_parse_time_expression_weeks() {
    assert_eq!(parse_time_expression("1 week").unwrap().as_secs(), 604800);
    assert_eq!(parse_time_expression("2 weeks").unwrap().as_secs(), 1209600);
    assert_eq!(parse_time_expression("1w").unwrap().as_secs(), 604800);
}

#[test]
fn test_parse_time_expression_months() {
    assert_eq!(parse_time_expression("1 month").unwrap().as_secs(), 2592000);
    assert_eq!(parse_time_expression("2 months").unwrap().as_secs(), 5184000);
}

#[test]
fn test_parse_time_expression_yesterday() {
    assert_eq!(parse_time_expression("yesterday").unwrap().as_secs(), 86400);
}

#[test]
fn test_parse_time_expression_invalid() {
    assert!(parse_time_expression("").is_err());
    assert!(parse_time_expression("abc").is_err());
    assert!(parse_time_expression("0 days").is_err());
}
```

**Step 2: Run to verify failure**

Run: `cargo test --lib commands::diff::tests::test_parse_time_expression_days -- --nocapture`
Expected: FAIL — `parse_time_expression` doesn't exist

**Step 3: Commit**

```bash
git add src/commands/diff.rs
git commit -m "test: add failing tests for --since time expression parsing"
```

---

### Task 8: --since Flag — Implementation

**Files:**
- Modify: `src/cli/mod.rs:42-63` (add `since` field to Diff)
- Modify: `src/commands/diff.rs` (add `parse_time_expression`, `resolve_since`)
- Modify: `src/main.rs` (pass `since` to diff::run)

**Step 1: Add `since` field to CLI**

In `src/cli/mod.rs`, add to the Diff variant after `git_ref`:
```rust
/// Show changes since a time expression (e.g., "1 day", "2 hours", "1 week")
#[arg(long)]
since: Option<String>,
```

**Step 2: Implement `parse_time_expression`**

Add to `src/commands/diff.rs`:

```rust
pub fn parse_time_expression(expr: &str) -> Result<std::time::Duration, Box<dyn std::error::Error>> {
    let expr = expr.trim().to_lowercase();

    if expr == "yesterday" {
        return Ok(std::time::Duration::from_secs(86400));
    }

    // Try short form: "1d", "3h", "2w"
    if let Some(num_str) = expr.strip_suffix('d') {
        let n: u64 = num_str.trim().parse().map_err(|_| "invalid number")?;
        if n == 0 { return Err("duration must be positive".into()); }
        return Ok(std::time::Duration::from_secs(n * 86400));
    }
    if let Some(num_str) = expr.strip_suffix('h') {
        let n: u64 = num_str.trim().parse().map_err(|_| "invalid number")?;
        if n == 0 { return Err("duration must be positive".into()); }
        return Ok(std::time::Duration::from_secs(n * 3600));
    }
    if let Some(num_str) = expr.strip_suffix('w') {
        let n: u64 = num_str.trim().parse().map_err(|_| "invalid number")?;
        if n == 0 { return Err("duration must be positive".into()); }
        return Ok(std::time::Duration::from_secs(n * 604800));
    }

    // Try long form: "1 day", "2 hours", "1 week", "1 month"
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 2 {
        return Err(format!("unrecognized time expression: {expr}").into());
    }
    let n: u64 = parts[0].parse().map_err(|_| format!("invalid number: {}", parts[0]))?;
    if n == 0 {
        return Err("duration must be positive".into());
    }
    let secs_per_unit = match parts[1] {
        "day" | "days" => 86400,
        "hour" | "hours" => 3600,
        "week" | "weeks" => 604800,
        "month" | "months" => 2592000, // 30 days
        _ => return Err(format!("unknown time unit: {}", parts[1]).into()),
    };
    Ok(std::time::Duration::from_secs(n * secs_per_unit))
}
```

**Step 3: Implement `resolve_since`**

Add to `src/commands/diff.rs`:

```rust
pub fn resolve_since(repo_path: &Path, since: &str) -> Result<String, Box<dyn std::error::Error>> {
    let duration = parse_time_expression(since)?;
    let cutoff = std::time::SystemTime::now() - duration;
    let cutoff_epoch = cutoff
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;

    let repo = git2::Repository::open(repo_path)?;
    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;
    revwalk.set_sorting(git2::Sort::TIME)?;

    for oid in revwalk {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        if commit.time().seconds() <= cutoff_epoch {
            return Ok(oid.to_string());
        }
    }

    Err("no commits found before the given time".into())
}
```

**Step 4: Update `run()` signature and main.rs**

Add `since: Option<&str>` parameter to `diff::run()`. At the top of `run()`, resolve it:

```rust
let effective_git_ref = match since {
    Some(s) => Some(resolve_since(path, s)?),
    None => git_ref.map(|s| s.to_string()),
};
let git_ref_str = effective_git_ref.as_deref();
// Use git_ref_str instead of git_ref throughout
```

In `src/main.rs`, pass `since` from CLI to `diff::run()`.

In `src/cli/mod.rs`, add `since` to the Diff command.

**Step 5: Run tests**

Run: `cargo test --verbose`
Expected: ALL PASS

**Step 6: Add integration test for --since**

Add to `tests/cli_test.rs`:

```rust
#[test]
fn test_diff_since_flag() {
    // Create repo with commit, wait, add another commit, use --since "1 hour"
    let dir = tempfile::TempDir::new().unwrap();
    // ... set up repo with commits ...
    Command::cargo_bin("cxpak").unwrap()
        .args(["diff", "--since", "1 hour"])
        .current_dir(dir.path())
        .assert()
        .success();
}
```

**Step 7: Commit**

```bash
git add src/cli/mod.rs src/commands/diff.rs src/main.rs tests/cli_test.rs
git commit -m "feat: add --since flag for diff command with time expression parsing"
```

---

### Task 9: Version Bump & Final Verification

**Files:**
- Modify: `Cargo.toml:3` (version)
- Modify: `Cargo.lock` (auto-updated by cargo)
- Modify: `plugin/.claude-plugin/plugin.json` (version)
- Modify: `.claude-plugin/marketplace.json` (version)

**Step 1: Bump version to 0.7.0**

In `Cargo.toml`: `version = "0.7.0"`
In `plugin/.claude-plugin/plugin.json`: update version field
In `.claude-plugin/marketplace.json`: update version field

**Step 2: Run full test suite**

Run: `cargo test --verbose`
Expected: ALL PASS

**Step 3: Run clippy and fmt**

Run: `cargo fmt -- --check && cargo clippy --all-targets -- -D warnings`
Expected: No warnings, no errors

**Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock plugin/.claude-plugin/plugin.json .claude-plugin/marketplace.json
git commit -m "release: bump version to 0.7.0"
```

---

## Summary

| Task | What | Files |
|------|------|-------|
| 1 | Reverse index — tests | `src/index/graph.rs` |
| 2 | Reverse index — impl | `src/index/graph.rs` |
| 3 | No double read — test | `src/index/mod.rs` |
| 4 | No double read — impl | `src/cache/parse.rs`, `src/index/mod.rs`, 3 commands |
| 5 | Default tokens — tests | `src/cli/mod.rs` |
| 6 | Default tokens — impl | `src/cli/mod.rs` |
| 7 | --since flag — tests | `src/commands/diff.rs` |
| 8 | --since flag — impl | `src/cli/mod.rs`, `src/commands/diff.rs`, `src/main.rs`, `tests/cli_test.rs` |
| 9 | Version bump | `Cargo.toml`, plugin files |

**Note:** Graph caching (Problem 3 from design doc) is deferred to v0.7.1 per the design doc's recommendation — the graph build is already fast and the first two perf changes are sufficient.
