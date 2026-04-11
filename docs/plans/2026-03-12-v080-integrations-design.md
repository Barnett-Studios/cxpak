# cxpak v0.8.0 — Integrations

**Goal:** Make cxpak available everywhere — GitHub PRs via Actions, all MCP-compatible tools via daemon mode, and IDE workflows via persistent indexing.

---

## Feature 1: GitHub Action

### Problem

cxpak's `diff` command produces exactly the kind of context a reviewer needs: changed code with dependency context, token-budgeted. But you have to run it manually. The highest-leverage integration is automatic PR comments.

### Solution: `cxpak-action` GitHub Action

Published to the GitHub Marketplace. Runs `cxpak diff` on every PR and posts the output as a comment.

```yaml
# .github/workflows/cxpak.yml
name: cxpak context
on:
  pull_request:
    types: [opened, synchronize]

jobs:
  context:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0  # Need full history for diff
      - uses: lyubomir-bozhinov/cxpak-action@v1
        with:
          tokens: 30k          # Budget for PR comment (smaller for readability)
          format: markdown     # PR comments are markdown
          focus: ""            # Optional: auto-detect from changed files
          since: ""            # Optional: override diff range
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

### Implementation

The action is a separate repository: `lyubomir-bozhinov/cxpak-action`.

**action.yml:**
```yaml
name: 'cxpak Context'
description: 'Post cxpak diff context as a PR comment'
inputs:
  tokens:
    description: 'Token budget'
    default: '30k'
  format:
    description: 'Output format'
    default: 'markdown'
  focus:
    description: 'Focus path prefix'
    required: false
  version:
    description: 'cxpak version to install'
    default: 'latest'
runs:
  using: 'composite'
  steps:
    - name: Install cxpak
      shell: bash
      run: |
        VERSION="${{ inputs.version }}"
        if [ "$VERSION" = "latest" ]; then
          curl -sSL https://github.com/lyubomir-bozhinov/cxpak/releases/latest/download/cxpak-x86_64-unknown-linux-gnu.tar.gz | tar xz
        else
          curl -sSL "https://github.com/lyubomir-bozhinov/cxpak/releases/download/v${VERSION}/cxpak-x86_64-unknown-linux-gnu.tar.gz" | tar xz
        fi
        mv cxpak /usr/local/bin/

    - name: Run cxpak diff
      shell: bash
      run: |
        ARGS="--tokens ${{ inputs.tokens }} --format ${{ inputs.format }}"
        if [ -n "${{ inputs.focus }}" ]; then
          ARGS="$ARGS --focus ${{ inputs.focus }}"
        fi
        ARGS="$ARGS --git-ref origin/${{ github.base_ref }}"
        cxpak diff $ARGS > /tmp/cxpak-output.md

    - name: Post PR comment
      shell: bash
      env:
        GH_TOKEN: ${{ env.GITHUB_TOKEN }}
      run: |
        # Delete previous cxpak comment if exists
        COMMENT_ID=$(gh api repos/${{ github.repository }}/issues/${{ github.event.pull_request.number }}/comments \
          --jq '.[] | select(.body | startswith("<!-- cxpak -->")) | .id' | head -1)
        if [ -n "$COMMENT_ID" ]; then
          gh api repos/${{ github.repository }}/issues/comments/$COMMENT_ID -X DELETE
        fi
        # Post new comment
        {
          echo "<!-- cxpak -->"
          echo "<details><summary>📦 cxpak context (click to expand)</summary>"
          echo ""
          cat /tmp/cxpak-output.md
          echo ""
          echo "</details>"
        } > /tmp/cxpak-comment.md
        gh pr comment ${{ github.event.pull_request.number }} --body-file /tmp/cxpak-comment.md
```

### Adoption strategy

- Collapsible `<details>` tag so it doesn't dominate the PR
- `<!-- cxpak -->` marker for idempotent comment updates (edit on push, don't spam)
- Configurable token budget (30k default for PR readability)
- Include in cxpak README with one-click setup instructions

### Tests

- Test the action in the cxpak repo itself (dogfood)
- Verify comment posting, comment updating on push, and comment deletion on close

---

## Feature 2: Daemon Mode (`cxpak watch` / `cxpak serve`)

### Problem

Every `cxpak` invocation starts cold: scan files, parse with tree-sitter, build index, build graph. For a 1000-file repo, this takes 2-5 seconds even with caching. For interactive use (IDE integration, MCP queries), this latency is unacceptable.

### Solution: Long-running daemon with file watching

Two subcommands:
- `cxpak watch` — watch for file changes, keep index hot, output to stdout on change
- `cxpak serve` — same as watch, but expose an HTTP/MCP API

### Architecture

```
┌────────────────────────────────────────────┐
│                cxpak daemon                │
│                                            │
│  ┌──────────┐  ┌─────────┐  ┌───────────┐ │
│  │ Watcher  │→ │ Parser  │→ │  Index    │ │
│  │ (notify) │  │(increm.)│  │ (in-mem)  │ │
│  └──────────┘  └─────────┘  └─────┬─────┘ │
│                                   │       │
│  ┌──────────────────────────────┐ │       │
│  │       Query Engine           │←┘       │
│  │  overview / trace / diff     │         │
│  └──────────┬───────────────────┘         │
│             │                             │
│  ┌──────────┴───────────────────┐         │
│  │       Transport Layer        │         │
│  │  HTTP API / MCP / stdout     │         │
│  └──────────────────────────────┘         │
└────────────────────────────────────────────┘
```

### Incremental updates

When a file changes:
1. `notify` crate detects the change
2. Re-parse only the changed file (tree-sitter is fast for single files)
3. Update the in-memory index entry
4. Rebuild only the affected graph edges (remove old edges from this file, add new ones)
5. Recompute rankings (fast — just recalculate the changed file's scores)

This turns a 2-5 second cold start into a 10-50ms incremental update.

### Dependencies

```toml
notify = "7"           # File system watcher (cross-platform)
axum = "0.8"           # HTTP server (for serve mode)
tokio = { version = "1", features = ["full"] }  # Async runtime
```

### `cxpak watch`

```
$ cxpak watch --tokens 50k
cxpak: watching . (500 files indexed)
cxpak: src/main.rs changed, re-indexing...
cxpak: index updated (12ms)
```

Output modes:
- `--format json` — emit JSON events to stdout on each change (for piping to other tools)
- `--format markdown` — re-emit full overview on each change (for terminal preview)
- Default: just log what changed, keep index hot for `serve` mode

### `cxpak serve`

```
$ cxpak serve --port 3000
cxpak: serving on http://localhost:3000 (500 files indexed)
```

HTTP API:
```
GET  /overview?tokens=50k&format=json&focus=src/auth
GET  /trace?target=my_function&tokens=50k
GET  /diff?tokens=50k&since=1d
GET  /health
GET  /stats   → { files: 500, tokens: 125000, last_update: "..." }
```

Response times: <50ms for any query (index is in memory).

### MCP Server Mode

`cxpak serve --mcp` exposes the same queries as MCP tools:

```json
{
    "tools": [
        {
            "name": "cxpak_overview",
            "description": "Get structured codebase context within a token budget",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tokens": { "type": "string", "default": "50k" },
                    "focus": { "type": "string" },
                    "format": { "type": "string", "enum": ["markdown", "json", "xml"] }
                }
            }
        },
        {
            "name": "cxpak_trace",
            "description": "Find a symbol and pack relevant code paths",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": { "type": "string" },
                    "tokens": { "type": "string", "default": "50k" }
                },
                "required": ["target"]
            }
        },
        {
            "name": "cxpak_diff",
            "description": "Show changes with dependency context",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tokens": { "type": "string", "default": "30k" },
                    "since": { "type": "string" }
                }
            }
        }
    ]
}
```

Transport: stdio (standard MCP transport). The daemon reads JSON-RPC from stdin, writes responses to stdout.

### IDE Integration Path

With `cxpak serve --mcp` running:
- **Claude Code**: Add to `mcp_servers` in settings — immediate access to all cxpak tools
- **Cursor**: Same MCP configuration
- **VS Code**: MCP extension or custom extension that talks to the HTTP API
- **Neovim**: Lua plugin calling the HTTP API
- **Any MCP client**: Zero-config — just point to `cxpak serve --mcp`

### Implementation order

1. **In-memory index** — refactor `CodebaseIndex` to support incremental updates
2. **File watcher** — `notify` integration, debouncing, change detection
3. **`cxpak watch`** — CLI subcommand, incremental re-index
4. **HTTP server** — `axum` routes for overview/trace/diff/health
5. **`cxpak serve`** — CLI subcommand wrapping watch + HTTP
6. **MCP transport** — JSON-RPC over stdio, MCP tool definitions
7. **`cxpak serve --mcp`** — CLI flag for MCP mode

### Tests

- Unit: incremental index update (add file, modify file, delete file)
- Unit: file watcher debouncing (rapid changes collapse to one update)
- Integration: HTTP API responses match CLI output
- Integration: MCP tool responses are valid JSON-RPC
- End-to-end: start daemon, modify file, query, verify updated results

---

## Feature 3: Codebase Narrative (stretch goal)

### Problem

The current overview output is structured data (tree, modules, signatures). It's great for LLMs but misses the "what is this project?" question that every new contributor asks.

### Solution

Add a `narrative` section to overview output — 3-5 sentences of natural language describing the project, generated from the structured data (not from an LLM).

Template-based generation using signals from the index:
- Primary language and framework detection
- Entry point identification
- Dependency count and key dependencies
- Project size and complexity tier

Example output:
```
This is a Rust CLI application (~15k tokens, 45 files) that uses tree-sitter
for code parsing across 12 programming languages. The entry point is
`src/main.rs` which dispatches to four commands (overview, trace, diff, clean).
Key dependencies include clap (CLI), git2 (git operations), and rayon
(parallelism). The codebase follows a pipeline architecture: Scanner → Parser
→ Index → Budget → Output.
```

This is a stretch goal — lower priority than the action and daemon. Can be deferred to v0.8.1.

---

## Release

- Version bump: 0.7.0 → 0.8.0
- New dependencies: `notify`, `axum`, `tokio` (behind a `daemon` feature flag to keep the CLI lean)
- Separate repo for GitHub Action
- Tag: `v0.8.0`
- CI: include daemon feature in release builds
