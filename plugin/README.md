# cxpak — Claude Code Plugin

Structured codebase context for Claude Code, powered by [cxpak](https://github.com/lyubomir-bozhinov/cxpak).

## What It Does

- **Auto-context:** Claude automatically runs `cxpak overview` when you ask about codebase structure
- **Auto-diff:** Claude automatically runs `cxpak diff` when you ask to review changes
- **On-demand commands:** `/cxpak:overview`, `/cxpak:trace`, `/cxpak:diff`, `/cxpak:clean`

## Installation

### Prerequisites

cxpak is auto-downloaded on first use if not already installed. To install manually:

```bash
# Via Homebrew
brew tap lyubomir-bozhinov/tap
brew install cxpak

# Via cargo
cargo install cxpak
```

### Add the Plugin

```
/plugin marketplace add lyubomir-bozhinov/cxpak
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

## License

MIT
