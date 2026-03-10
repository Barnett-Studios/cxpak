# Homebrew, Caching, and Diff Command — Design

**Goal:** Three features for cxpak v0.4.0: easier installation via Homebrew, faster re-runs via caching, and a new `diff` command for token-budgeted change summaries.

---

## Feature 1: Homebrew Tap

### Overview
Homebrew formula in a dedicated tap repo, auto-updated on each release.

### Design
- **Tap repo**: `lyubomir-bozhinov/homebrew-tap` on GitHub
- **Formula**: downloads pre-built tarballs from GitHub Releases (same artifacts the release workflow already produces)
- **Install**: `brew tap lyubomir-bozhinov/tap && brew install cxpak`
- **Automation**: new job in `.github/workflows/release.yml` that updates the formula SHA and version in the tap repo after artifacts are uploaded

### Formula Template
```ruby
class Cxpak < Formula
  desc "Token-budgeted codebase context for LLMs"
  homepage "https://github.com/lyubomir-bozhinov/cxpak"
  version "VERSION"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/lyubomir-bozhinov/cxpak/releases/download/vVERSION/cxpak-aarch64-apple-darwin.tar.gz"
      sha256 "SHA256"
    else
      url "https://github.com/lyubomir-bozhinov/cxpak/releases/download/vVERSION/cxpak-x86_64-apple-darwin.tar.gz"
      sha256 "SHA256"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/lyubomir-bozhinov/cxpak/releases/download/vVERSION/cxpak-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "SHA256"
    else
      url "https://github.com/lyubomir-bozhinov/cxpak/releases/download/vVERSION/cxpak-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "SHA256"
    end
  end

  def install
    bin.install "cxpak"
  end

  test do
    system "#{bin}/cxpak", "--help"
  end
end
```

### Release Workflow Addition
New job `update-homebrew` that runs after all build jobs complete:
1. Download all release tarballs
2. Compute SHA256 for each
3. Clone the tap repo
4. Render the formula template with version + SHAs
5. Commit and push to tap repo

Requires a PAT with write access to the tap repo, stored as a repository secret.

---

## Feature 2: Caching

### Overview
Cache tree-sitter parse results and token counts per file to skip recomputation on unchanged files.

### Design

**Location**: `.cxpak/cache/` inside the target repo.

**Cache key**: file path + mtime + file size. On each run, compare current mtime+size against cached values. Mismatch = re-parse.

**What's cached**: per-file entry containing:
- File path (relative to repo root)
- mtime (as Unix timestamp)
- File size (bytes)
- Language detected
- Symbols (name, kind, visibility, line)
- Imports
- Exports
- Token count

**Format**: single `cache.json` file (or bincode if perf matters later). Simple to inspect, debug, and blow away.

**Stale cleanup integration**: the existing `.cxpak/` cleanup (removing `tree.md`, `modules.md`, etc.) must preserve the `cache/` subdirectory. Only output files get wiped between runs.

**Invalidation**:
- Per-file: mtime+size mismatch on each run triggers re-parse of that file
- Full wipe: `cxpak clean` command deletes entire `.cxpak/` (cache + outputs)
- No time-based expiry, no size limits
- Orphaned entries (deleted files) are harmless; optional future pruning

**`cxpak clean` command**: new subcommand, trivial — just `rm -rf .cxpak/`.

### Integration Points
- Scanner: after walking files, check cache before parsing
- Parser: if cache hit, return cached ParseResult; if miss, parse and update cache
- Index: receives ParseResults as before (transparent)
- overview.rs cleanup: change `remove_dir_all(".cxpak")` to only remove non-cache files
- .gitignore: `.cxpak/` already ignored (no change needed)

---

## Feature 3: Diff Command

### Overview
`cxpak diff` shows what changed in a git repo, then packs relevant surrounding context (callers, types, imports) within a token budget. Like `git diff` + `trace` combined.

### CLI
```
cxpak diff --tokens 50k [ref] [path]
```
- `ref`: git ref to diff against (default: HEAD)
- `path`: repo path (default: `.`)
- `--format`: markdown/json/xml (same as overview/trace)
- `--out`: write to file instead of stdout
- `--verbose`: progress on stderr
- `--all`: full BFS (not just 1-hop context)

### Pipeline
1. **Git diff**: use `git2` to diff `ref` against working tree. Collect list of changed files + hunks.
2. **Parse changed files**: run through existing Scanner → Parser pipeline (with cache).
3. **Identify changed symbols**: map diff hunks to symbol definitions/usages.
4. **Context gathering**: trace-style BFS from changed files through DependencyGraph. 1-hop default, full BFS with `--all`.
5. **Budget allocation**: diff-first — include full diff output, fill remaining budget with context files, prioritized by dependency distance.
6. **Output**: render via existing markdown/json/xml output system.

### Output Structure (Markdown)
```markdown
## Changes (vs HEAD)

### src/parser/mod.rs
\```diff
- old line
+ new line
\```

### src/scanner/mod.rs
\```diff
...
\```

## Context

### src/index/mod.rs
(Full file or relevant symbols — callers/types used by changed code)

### src/output/markdown.rs
(Related file pulled in by dependency graph)
```

### Budget Strategy
- Phase 1: render all diff hunks. If this exceeds budget, truncate least-important hunks (by file, preserving most-changed files first).
- Phase 2: fill remaining budget with context, ordered by BFS distance from changed files. Closest dependencies first.

### Edge Cases
- No changes: print "No changes" and exit 0
- Untracked files: include if they're in git-tracked directories (same as `git diff --no-index` behavior), or skip. Lean toward skipping — untracked files aren't in the dependency graph anyway.
- Binary files: skip with note
- Massive diffs exceeding budget: truncate with omission markers, same pattern as overview

---

## Shared Concerns

### Version
All three features ship as v0.4.0.

### Testing
- Homebrew: manual test of formula install (CI can't easily test brew)
- Caching: integration tests comparing first-run vs cached-run output (should be identical), tests for invalidation on file change
- Diff: integration tests with temp git repos, staged changes, multiple refs
