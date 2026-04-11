# cxpak v0.7.0 — Performance & DX

**Goal:** Eliminate performance bottlenecks (O(V*E) dependents, double disk reads, no graph caching) and improve developer experience (default token budget, `--since` flag for diff).

---

## Workstream 1: Performance

### Problem 1: O(V*E) dependents lookup

`src/index/graph.rs` line 20-26:

```rust
pub fn dependents(&self, path: &str) -> Vec<&str> {
    self.edges
        .iter()
        .filter(|(_, deps)| deps.contains(path))
        .map(|(k, _)| k.as_str())
        .collect()
}
```

Every call scans all edges. Called per-file in `rank_files()` and in trace/diff graph walks. For a 500-file repo with 2000 edges, ranking alone does 500 * 2000 = 1M comparisons.

### Solution: Reverse adjacency index

Add a `reverse_edges: HashMap<String, HashSet<String>>` to `DependencyGraph`. Maintain it in `add_edge()`. `dependents()` becomes O(1) lookup.

```rust
#[derive(Debug, Default)]
pub struct DependencyGraph {
    pub edges: HashMap<String, HashSet<String>>,        // forward: A imports B
    reverse_edges: HashMap<String, HashSet<String>>,    // reverse: B is imported by A
}

impl DependencyGraph {
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

    pub fn dependents(&self, path: &str) -> Vec<&str> {
        self.reverse_edges
            .get(path)
            .map(|set| set.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }
}
```

`reachable_from()` also benefits — the reverse edge traversal (lines 57-60) currently scans all edges. Replace with `reverse_edges.get()`.

```rust
// Replace:
for (importer, deps) in &self.edges {
    if deps.contains(&current) && visited.insert(importer.clone()) {
        queue.push_back(importer.clone());
    }
}

// With:
if let Some(importers) = self.reverse_edges.get(&current) {
    for importer in importers {
        if visited.insert(importer.clone()) {
            queue.push_back(importer.clone());
        }
    }
}
```

**Tests:** All existing graph.rs tests should pass unchanged (API is identical). Add benchmarks for large graphs (1000 nodes, 5000 edges) to verify O(1) vs O(V*E) improvement.

---

### Problem 2: Double disk read

`src/cache/parse.rs` reads file contents during parsing (line 77: `std::fs::read_to_string`). Then `src/index/mod.rs` reads every file again during indexing (line 48: `std::fs::read_to_string`).

For a 500-file repo, that's 1000 file reads instead of 500.

### Solution: Pass content through the pipeline

Modify `parse_with_cache` to return file content alongside parse results. The index builder then uses the already-read content instead of re-reading from disk.

**Option A (minimal change):** Return `HashMap<String, (ParseResult, String)>` — parse result + content. Adjust `CodebaseIndex::build()` to accept content.

**Option B (cleaner):** Introduce an intermediate struct:

```rust
pub struct ParsedFile {
    pub relative_path: String,
    pub content: String,
    pub parse_result: Option<ParseResult>,
}
```

`parse_with_cache` returns `Vec<ParsedFile>`. `CodebaseIndex::build` takes `Vec<ScannedFile>` + `Vec<ParsedFile>` (or just `Vec<ParsedFile>` with ScannedFile data folded in).

**Recommended:** Option A for now — smaller diff, same perf benefit. Option B is a larger refactor better suited for v0.8.0 if needed.

Changes:
- `src/cache/parse.rs`: On cache miss, store the read content. On cache hit, re-read the file (cache doesn't store content, just parse results). Return `HashMap<String, (ParseResult, String)>`.
- `src/index/mod.rs`: `CodebaseIndex::build()` accepts content map. Skip `read_to_string` when content is provided.
- All callers updated: overview.rs, trace.rs, diff.rs.

**Cache hit note:** On a cache hit, we still need the file content for indexing (token counting, content search). We could either: (a) always read content in parse_with_cache (even on cache hit), or (b) store content in the cache. Option (a) is simpler and still halves the reads for cache misses. Option (b) bloats the cache significantly. Go with (a).

Actually, the real win is simpler than this: on cache miss, `parse_with_cache` already reads the file. Just return that content. On cache hit, we still need one read. Net savings: eliminate all cache-miss double reads (which are the expensive path anyway).

**Tests:** Integration tests verifying overview/trace/diff produce identical output before and after.

---

### Problem 3: Dependency graph never cached

The dependency graph is rebuilt from scratch on every run. For overview, it's built in overview.rs. For trace and diff, it's built via `trace::build_dependency_graph()`. The graph construction iterates all files, all imports, and does string matching for each — not expensive per se, but it's work that could be cached alongside parse results.

### Solution: Cache the graph as part of the file cache

The graph is deterministic given the parse results. If all files have cache hits (no mtime changes), the graph is unchanged.

Add a `graph_cache.json` alongside `cache.json` in `.cxpak/cache/`:

```json
{
    "edges": {
        "src/main.rs": ["src/lib.rs", "src/config.rs"],
        "src/lib.rs": ["src/util.rs"]
    },
    "cache_hash": "sha256-of-sorted-cache-entry-mtimes"
}
```

The `cache_hash` is derived from the cache entries (sorted path + mtime pairs). If the hash matches, the graph is valid. If any file changed, the hash differs and we rebuild.

Changes:
- `src/cache/mod.rs`: Add `GraphCache` struct with load/save.
- `src/commands/trace.rs`: Check graph cache before `build_dependency_graph()`.
- `src/commands/overview.rs` and `src/commands/diff.rs`: Same.

**Complexity note:** This is the lowest-impact of the three perf changes. The graph build is already fast (linear in files * imports). Only worth doing if benchmarks show it matters. Consider making this optional or deferring to v0.7.1 if the first two changes are sufficient.

**Tests:** Verify graph cache hit produces identical graph. Verify cache invalidation on file change.

---

## Workstream 2: DX Improvements

### Problem 1: `--tokens` is required

Every command requires `--tokens`. For a first-time user:

```
$ cxpak overview
error: the following required arguments were not provided: --tokens <TOKENS>
```

This is unnecessary friction. A sensible default eliminates the most common complaint.

### Solution: Default to 50k tokens

Change `--tokens` from required to optional with a default:

```rust
// Before
#[arg(long)]
tokens: String,

// After
#[arg(long, default_value = "50k")]
tokens: String,
```

Apply to all three commands: overview, diff, trace.

50k is a good default because:
- Fits comfortably in Claude's 200k context with room for conversation
- Large enough to be useful for most repos
- Small enough to not overwhelm smaller models
- The plugin already uses 50k as its default

**Tests:** Verify `cxpak overview` works without `--tokens`. Verify `--tokens 100k` still overrides.

---

### Problem 2: `--since` flag for diff

Currently, diff supports `--git-ref` for comparing against a specific commit. But the common use case is "what changed in the last N days/hours?" which requires looking up the right commit hash.

### Solution: Add `--since` flag

```rust
/// Show changes since a time expression (e.g., "1 day", "2 hours", "1 week")
#[arg(long)]
since: Option<String>,
```

Implementation: use `git2` to walk commits and find the first commit before the given time. Convert the time expression to a duration, subtract from now, find the nearest commit.

Time expressions to support:
- `"1 day"`, `"2 days"`, `"1d"`, `"2d"`
- `"1 hour"`, `"3 hours"`, `"1h"`, `"3h"`
- `"1 week"`, `"2 weeks"`, `"1w"`, `"2w"`
- `"1 month"`, `"2 months"`
- `"yesterday"`

When `--since` is provided, it overrides `--git-ref`. Under the hood, it resolves to a commit hash and passes it to `extract_changes` as if `--git-ref` were used.

```rust
fn resolve_since(repo_path: &Path, since: &str) -> Result<String, Box<dyn std::error::Error>> {
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

Changes:
- `src/cli/mod.rs`: Add `since: Option<String>` to Diff command.
- `src/commands/diff.rs`: Add `resolve_since()` and `parse_time_expression()`. Resolve before calling `extract_changes`.

**Tests:**
- Unit test `parse_time_expression` for all supported formats
- Integration test: create repo with commits at known times, verify `--since "1 day"` captures recent changes
- Test `--since` and `--git-ref` mutual exclusivity (error if both provided, or `--since` takes precedence)

---

## Release

- Version bump: 0.6.1 → 0.7.0
- Tag: `v0.7.0`
- Update CLAUDE.md architecture section if pipeline changes
- CI: same workflow
