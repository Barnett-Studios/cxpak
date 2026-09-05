# cxpak — Claude Code Plugin

Structured codebase context for Claude Code, powered by [cxpak](https://github.com/Barnett-Studios/cxpak).

## What It Does

- **Auto-context:** Claude automatically runs `cxpak overview` when you ask about codebase structure
- **Auto-diff:** Claude automatically runs `cxpak diff` when you ask to review changes
- **On-demand commands:** `/cxpak:overview`, `/cxpak:trace`, `/cxpak:diff`, `/cxpak:clean`

## Installation

### Prerequisites

cxpak is installed automatically via Homebrew or cargo if not on PATH. To install manually:

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

## MCP Tools (5)

When used as an MCP server (`cxpak serve --mcp`), cxpak advertises **five intent-tools**. Each
one selects what it does through a required `op` parameter, so the tool you call is the
*intent* and the `op` is the *capability*:

| Tool | `op` values |
|------|-------------|
| `cxpak_context` | `context`, `retrieval`, `search`, `overview`, `stats`, `context_for_task`, `pack_context`, `briefing` |
| `cxpak_graph` | `graph`, `trace`, `blast_radius`, `call_graph`, `dead_code`, `api_surface`, `data_flow`, `cross_lang`, `predict` |
| `cxpak_data` | `data` |
| `cxpak_review` | `review`, `diff`, `verify` |
| `cxpak_insight` | `health`, `risks`, `architecture`, `conventions`, `security_surface`, `drift`, `visual`, `onboard` |

Start with `cxpak_context` (`op: "context"`) — it returns token-budgeted structural context for a
one-line task description.

Per-op parameters are listed in [`docs/MIGRATION-3.0.md`](../docs/MIGRATION-3.0.md), and
`tools/list` is the authority at runtime.

This section deliberately makes **no blanket claim about parameters**. The one it replaced said
*"all tools support a `focus` path prefix parameter"*, and that was not true of the surface it
described or of this one — measured on `main`, 23 of 29 ops read `focus` and six do not. A
sentence that is right for most ops is wrong for the caller holding one of the others.

> The pre-3.0 per-tool names still route as deprecated aliases, but `tools/list` does not
> advertise them — a client that builds its callable set from `tools/list`, which is what an
> agent harness does, will not find them. Use the five above.

## License

MIT
