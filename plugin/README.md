# cxpak — Claude Code Plugin

Structured codebase context for Claude Code, powered by [cxpak](https://github.com/Barnett-Studios/cxpak).

## What It Does

- **Auto-context:** Claude automatically runs `cxpak overview` when you ask about codebase structure
- **Auto-diff:** Claude automatically runs `cxpak diff` when you ask to review changes
- **On-demand commands:** `/cxpak:overview`, `/cxpak:trace`, `/cxpak:diff`, `/cxpak:clean`

## Installation

### Prerequisites

cxpak is auto-downloaded on first use if not already installed. To install manually:

```bash
# Via Homebrew
brew tap Barnett-Studios/tap
brew install cxpak

# Via cargo
cargo install cxpak
```

### Add the Plugin

```
/plugin marketplace add Barnett-Studios/cxpak
/plugin install cxpak
```

## Skills (Auto-Invoked)

| Skill | Triggers When |
|-------|---------------|
| `codebase-context` | You ask about project structure, architecture, or how components relate |
| `diff-context` | You ask to review changes, prepare a PR description, or understand modifications |

## Commands (User-Invoked)

| Command | Description |
|---------|-------------|
| `/cxpak:overview` | Structured codebase summary |
| `/cxpak:trace <symbol>` | Trace a symbol through the dependency graph |
| `/cxpak:diff` | Changes with surrounding dependency context |
| `/cxpak:clean` | Clear cache and output files |

All commands ask for a token budget (default: 50k).

## MCP Tools (11)

When used as an MCP server (`cxpak serve --mcp`), all tools support a `focus` path prefix parameter for scoped results:

| Tool | Description |
|------|-------------|
| `cxpak_auto_context` | One-call optimal context for any task |
| `cxpak_overview` | Structured repo summary |
| `cxpak_trace` | Trace a symbol through dependencies |
| `cxpak_stats` | Language stats and token counts |
| `cxpak_diff` | Show changes with dependency context |
| `cxpak_context_for_task` | Score and rank files by relevance to a task |
| `cxpak_pack_context` | Pack selected files into a token-budgeted bundle |
| `cxpak_search` | Regex search with context lines |
| `cxpak_blast_radius` | Analyze change impact with risk scores |
| `cxpak_api_surface` | Extract public API surface |
| `cxpak_context_diff` | Show what changed since last auto_context call |

## License

MIT
