# ADR-0207: Repo text is untrusted at the point it becomes output

- **Status:** ACCEPTED
- **Date:** 2026-09-03
- **Issue:** #43

## Context

cxpak reads a repository and hands the result to a reading agent. Everything it
emits — a commit message, a file body, a symbol body, a diff hunk, a path — is
text somebody else wrote, and the renderer is where that text stops being data
and starts being structure.

Two of the three renderers already knew this and one did not:

| renderer | before |
|---|---|
| `output/json.rs` | safe by construction — `serde_json` escapes control characters |
| `output/xml.rs` | `escape_xml` filtered `0x0..=0x8`, `0xB..=0xC`, `0xE..=0x1F` |
| `output/markdown.rs` | nothing |

Markdown is the default. So the renderer with no control-character policy was
the one almost every consumer actually reads, and the disagreement between two
renderers over the same `OutputSections` was invisible because no test compared
them.

Separately, `code_fence_for` — a function whose entire purpose is to emit a
backtick run longer than any run inside the content — was used for exactly one
section, `directory_tree`. The six sites that pack a **file body**, a **symbol
body** or a **diff hunk** into markdown each wrote `` ``` `` by hand. The
ordinary consequence is not an attack: `README.md` is a key file, most READMEs
contain a fenced example, and every one of them closed cxpak's block early and
had its remainder read as markdown.

## Decision

**Repo-derived text is sanitised where it becomes output, not where it is read.**
Two mechanisms, one place each.

### 1. `output::strip_control_chars` — one policy, all renderers

Drops every control character except `\n` and `\t`. `escape_xml` now calls it
instead of carrying its own list, so the two renderers cannot drift again.

`\r` and DEL are dropped where `escape_xml` used to keep them: a lone `\r`
overwrites a line the reader has already seen, which is the same class of harm
as `ESC`. A `\r\n` line ending loses its `\r` and keeps its `\n`.

**This is confinement, not censoring.** `\u{1b}[31m` becomes `[31m` — the text
survives and stops being a terminal instruction. Removing content would trade a
legibility bug for a fidelity bug.

### 2. `output::fenced` — the fence is sized by the content it must close

`code_fence_for` moves to `output/mod.rs` and all six packing sites go through
`fenced(content, info)`. In `render_key_files` the fence is computed from the
**whole** file and reused for whatever survives truncation: dropping lines can
only remove backtick runs, so a fence that closes the full body closes every
prefix of it, and the header/footer token accounting stays exact.

### 3. Rejected: fencing whole sections

#43 asks for "fencing on packed content", and for the section bodies that is the
wrong shape. `metadata`, `module_map`, `git_context` and the rest are **cxpak's
own generated markdown** — `###` sub-headers, list items, tables. Fencing them
would turn cxpak's structure into a wall of literal text and destroy the thing
the format is for.

The vector in those sections is not the section, it is the **interpolation
point**: `render_git_context` formats a commit message into a single-line list
item, so a message containing `\n\n## ` breaks out of the item and forges a
heading. That is fixed at the six interpolation points with `one_line`, which
flattens newlines and drops backticks for the two fields that sit inside a code
span. Fencing is used where the payload really is an opaque body, and nowhere
else.

## Consequences

- One control-character policy, named once, shared by both text renderers.
- A README with a code block renders correctly in Key Files for the first time.
- Sanitisation is **not** a security boundary for a reading agent's judgement —
  it removes the mechanisms that let repo text impersonate cxpak's own
  structure. A model that follows instructions found in quoted file content is
  not addressed here, and cannot be addressed by a renderer.
- `#43` also cites ADR-0044. That citation does not hold: 0044 is about the
  content map and double reads, not about untransformed content reaching output.
  Recorded here so the next reader does not go looking.

## Alternatives considered

- **Escape markdown metacharacters** (`#`, `` ` ``, `|`, `*`). Rejected: the
  section bodies *are* markdown, so escaping them destroys the output, and for
  the fenced bodies a correctly-sized fence already covers it.
- **Strip control characters at index time.** Rejected: the index is also the
  input to search, symbol extraction and the LSP surface, where the file's real
  bytes are what a consumer wants. Output is the boundary; the index is not.
