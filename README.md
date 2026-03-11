# cxpak

![Rust](https://img.shields.io/badge/Rust-1.70+-orange.svg)
![CI](https://github.com/lyubomir-bozhinov/cxpak/actions/workflows/ci.yml/badge.svg)
![Crates.io](https://img.shields.io/crates/v/cxpak)
![Downloads](https://img.shields.io/crates/d/cxpak)
![Homebrew](https://img.shields.io/badge/Homebrew-tap-blue.svg)
![License](https://img.shields.io/badge/License-MIT-green.svg)

> Spends CPU cycles so you don't spend tokens. The LLM gets a briefing packet instead of a flashlight in a dark room.

A Rust CLI that indexes codebases using tree-sitter and produces token-budgeted context bundles for LLMs.

## Installation

```bash
# Via Homebrew (macOS/Linux)
brew tap lyubomir-bozhinov/tap
brew install cxpak

# Via cargo
cargo install cxpak
```

## Claude Code Plugin

cxpak ships as a Claude Code plugin — skills auto-trigger when you ask about codebase structure or changes, and slash commands give you direct control.

**Install the plugin:**

```
/plugin marketplace add lyubomir-bozhinov/cxpak
/plugin install cxpak
```

**Skills (auto-invoked):**

| Skill | Triggers when you... |
|-------|---------------------|
| `codebase-context` | Ask about project structure, architecture, how components relate |
| `diff-context` | Ask to review changes, prepare a PR description, understand what changed |

**Commands (user-invoked):**

| Command | Description |
|---------|-------------|
| `/cxpak:overview` | Generate a structured repo summary |
| `/cxpak:trace <symbol>` | Trace a symbol through the dependency graph |
| `/cxpak:diff` | Show changes with dependency context |
| `/cxpak:clean` | Remove `.cxpak/` cache and output files |

The plugin auto-downloads the cxpak binary if it's not already installed.

## Usage

```bash
# Structured repo summary within a token budget
cxpak overview --tokens 50k .

# Write output to a file
cxpak overview --tokens 50k --out context.md .

# Trace from a function/error, pack relevant code paths
cxpak trace --tokens 50k "handle_request" .

# Trace with full dependency graph traversal
cxpak trace --tokens 50k --all "MyError" /path/to/repo

# Different output formats
cxpak overview --tokens 50k --format json .
cxpak overview --tokens 50k --format xml .

# Show changes with dependency context (vs working tree)
cxpak diff --tokens 50k .

# Diff against a specific ref
cxpak diff --tokens 50k --git-ref main .

# Full dependency graph context
cxpak diff --tokens 50k --all .

# Clean cache and output files
cxpak clean .
```

## What You Get

The `overview` command produces a structured briefing with these sections:

- **Project Metadata** — file counts, languages, estimated tokens
- **Directory Tree** — full file listing
- **Module / Component Map** — files with their public symbols
- **Dependency Graph** — import relationships between files
- **Key Files** — full content of README, config files, manifests
- **Function / Type Signatures** — every public symbol's signature
- **Git Context** — recent commits, file churn, contributors

Each section has a budget allocation. When content exceeds its budget, it's truncated with the most important items preserved first.

## Pack Mode

When a repo exceeds the token budget, cxpak automatically switches to **pack mode**:

- The overview stays within budget (one file, fits in one LLM prompt)
- A `.cxpak/` directory is created with **full untruncated** detail files
- Truncated sections in the overview get pointers to their detail files

```
repo/
  .cxpak/
    tree.md          # complete directory tree
    modules.md       # every file, every symbol
    dependencies.md  # full import graph
    signatures.md    # every public signature
    key-files.md     # full key file contents
    git.md           # full git history
```

Detail file extensions match `--format`: `.md` for markdown, `.json` for json, `.xml` for xml.

The overview tells the LLM what exists. The detail files let it drill in on demand. `.cxpak/` is automatically added to `.gitignore`.

If the repo fits within budget, you get a single file with everything — no `.cxpak/` directory needed.

## Caching

cxpak caches parse results in `.cxpak/cache/` to speed up re-runs. The cache is keyed on file modification time and size — when a file changes, it's automatically re-parsed.

To clear the cache and all output files:

```bash
cxpak clean .
```

## Supported Languages

Rust, TypeScript, JavaScript, Python, Java, Go, C, C++, Ruby, C#, Swift, Kotlin

Tree-sitter grammars are compiled in. All 12 languages are enabled by default. Language features can be toggled:

```bash
# Only Rust and Python support
cargo install cxpak --no-default-features --features lang-rust,lang-python
```

## License

MIT

---

## About Me

I build things — products, teams, systems… occasionally IKEA furniture (with mixed results). I've led engineering in everything from scrappy startups to big enterprises, but what I really love is turning ideas into real, useful tech.

I'm into mentoring, scaling systems, and solving problems that make people say "well, that's impossible."

Thanks for stopping by!
