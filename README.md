# cxpak

> Spends CPU cycles so you don't spend tokens. The LLM gets a briefing packet instead of a flashlight in a dark room.

A Rust CLI that indexes codebases using tree-sitter and produces token-budgeted context bundles for LLMs.

## Installation

```bash
cargo install cxpak
```

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

## Supported Languages

Rust, TypeScript, JavaScript, Python, Java, Go, C, C++, Ruby, C#, Swift, Kotlin

Tree-sitter grammars are compiled in. All 12 languages are enabled by default. Language features can be toggled:

```bash
# Only Rust and Python support
cargo install cxpak --no-default-features --features lang-rust,lang-python
```

## License

MIT
