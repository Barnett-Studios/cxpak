# cxpak Claude Code Plugin — Design

**Goal:** Make cxpak's codebase indexing available as auto-invoked skills and user-invoked commands inside Claude Code sessions.

---

## Plugin Structure

```
plugin/
├── commands/
│   ├── overview.md             # /cxpak:overview
│   ├── trace.md                # /cxpak:trace <symbol>
│   ├── diff.md                 # /cxpak:diff
│   └── clean.md                # /cxpak:clean
├── skills/
│   ├── codebase-context/
│   │   └── SKILL.md            # auto-triggers on codebase questions
│   └── diff-context/
│       └── SKILL.md            # auto-triggers on change review requests
├── lib/
│   └── ensure-cxpak            # bash: checks PATH, downloads if missing
├── LICENSE
└── README.md
```

No agents, no hooks. Skills handle auto-invocation. `lib/ensure-cxpak` is shared by all commands and skills.

---

## Auto-Download (`lib/ensure-cxpak`)

Bash script that resolves the cxpak binary path:

1. Check if `cxpak` is on PATH — if yes, return its path
2. Check if previously downloaded to `~/.claude/plugins/cxpak/bin/cxpak` — if yes, return that
3. If neither: detect OS + arch via `uname`, download latest release tarball from GitHub Releases, extract to `~/.claude/plugins/cxpak/bin/`, make executable, return path
4. Print progress to stderr

Platform mapping:
- Darwin + arm64 → `cxpak-aarch64-apple-darwin.tar.gz`
- Darwin + x86_64 → `cxpak-x86_64-apple-darwin.tar.gz`
- Linux + aarch64 → `cxpak-aarch64-unknown-linux-gnu.tar.gz`
- Linux + x86_64 → `cxpak-x86_64-unknown-linux-gnu.tar.gz`

No Windows support (Claude Code doesn't run on Windows natively).

---

## Skills (Auto-Invoked)

### `codebase-context`

```yaml
name: codebase-context
description: "Use when the user asks about codebase structure, architecture, what a project does, how components relate, or needs project-wide context to answer a question."
```

When triggered, Claude:
1. Asks the user for token budget (default 50k)
2. Runs `ensure-cxpak` to get the binary path
3. Runs `cxpak overview --tokens <budget> --format markdown .`
4. Injects the output into conversation context
5. Uses it to answer the user's question
6. Mentions that cxpak is providing the context

### `diff-context`

```yaml
name: diff-context
description: "Use when the user asks to review changes, understand what changed, prepare a PR description, or needs context about recent modifications."
```

When triggered, Claude:
1. Asks the user for token budget (default 50k)
2. Optionally asks for a git ref (default: HEAD / working tree)
3. Runs `cxpak diff --tokens <budget> --format markdown [--git-ref <ref>] .`
4. Injects the diff output and surrounding dependency context
5. Uses it to review, summarize, or discuss the changes
6. Mentions that cxpak is providing the context

---

## Commands (User-Invoked)

### `/cxpak:overview`

Asks for token budget (default 50k). Runs `cxpak overview --tokens <budget> --format markdown <path>`. User can pass a path as argument (default `.`). Injects output into conversation.

### `/cxpak:trace <symbol>`

Requires a symbol argument. Asks for token budget (default 50k). Runs `cxpak trace --tokens <budget> --format markdown <symbol> <path>`. Supports `--all` flag for full BFS. Injects output into conversation.

### `/cxpak:diff`

Asks for token budget (default 50k) and optional git ref. Runs `cxpak diff --tokens <budget> --format markdown [--git-ref <ref>] <path>`. Injects output into conversation.

### `/cxpak:clean`

Runs `cxpak clean <path>`. No questions asked, confirms when done.

All commands call `ensure-cxpak` first. Each command's `.md` file is a prompt that tells Claude to run the bash command and use the output.

---

## Defaults

- **Token budget:** 50k, always ask user (skills and commands)
- **Output format:** Markdown (hardcoded — native to Claude's context window)
- **Path:** Current working directory (`.`)

---

## Distribution

Plugin lives in `plugin/` directory of the cxpak repo. Versioned and released alongside the CLI. If a CLI flag changes, the plugin updates in the same commit.

Installation: users add the plugin directory to their Claude Code plugin configuration.
