---
id: '0209'
title: The LSP server anchors on an absolute workspace root, and the CLI path wins over rootUri
status: ACCEPTED
date: 2026-09-05
triggered_by: cxpak#75 and cxpak#76 — two defects with one cause, found by the family QA rotation
loop: implementation
---

# ADR-0209: The LSP server anchors on an absolute workspace root

## Context

`cxpak lsp` takes `[PATH] [default: .]` and stored that path as given. Two reported
defects follow from the single fact that the root could be relative:

- **#75** — `Url::from_file_path` returns `Err` for a non-absolute path, so every
  `workspace/symbol` location fell back to `file:///unknown`. `location.uri` is the
  only field a client uses to open a symbol, so "Go to Symbol in Workspace" listed
  every correct name and could open none of them. The `containerName` on the same
  object carried the right file the whole time.
- **#76** — `uri_to_rel_path` does `abs.strip_prefix(repo_root)`, which cannot match
  an absolute `file://` URI against `.`, so the primary resolution strategy *never
  succeeded* at the default path and every lookup fell through to a
  separator-bounded suffix match. That match aligns to *a* directory boundary, not
  to *this workspace's* root, so a server rooted at `/repo` answered for
  `/other/src/main.rs` with `/repo`'s analysis — another project's token count, a
  suppressed real warning, and a dead-code diagnostic on a symbol called two lines
  below its own definition.

Both requests succeeded and both responses were well-formed. Only the subject was
wrong, which is why neither had a failing test.

The ordering between them is not free: deleting the suffix fallback while the root
could still be `.` would have broken codeLens, diagnostics and hover outright,
because at that root the fallback was the only strategy that ever matched.

## Decision

**1. Anchor on an absolute, canonicalised root at startup.** `run_stdio`
canonicalises before it indexes, and a root that cannot be resolved is an error the
caller sees rather than a placeholder to serve wrong answers from. Canonicalising
also collapses symlinks, which is load-bearing on macOS where a client's
`/var/folders/…` and the server's `/private/var/folders/…` name one directory.

**2. A well-formed `file://` URI resolves against the root and nothing else.** If it
does not live under the root, the answer is `None`. The suffix match survives only
for uri-ish input that is not a URL at all — a bare `src/main.rs` from a non-LSP
caller, which carries no root to resolve against — and keeps its separator bound
there.

**3. Never substitute a placeholder URI.** A symbol whose location cannot be built is
omitted. `unwrap_or_else(|_| "file:///unknown")` converted a construction failure
into a well-formed lie; an omitted symbol is a visible gap, a placeholder is not.
This is the same principle already stated a hundred lines away in this file on
`cxpak/blastRadius` — *"Silent empty results for typo'd paths make a caller's 'zero
dependents' response indistinguishable from 'unknown file'"* — applied where it was
not.

**4. The client's `rootUri` is READ but does not win.** It is compared against the
indexed root and a disagreement is reported to the client as a warning; the CLI path
stays authoritative.

## Why the CLI path wins, and not `rootUri`

This is the one genuinely open question in the two tickets, and #75 suggests
honouring `rootUri` first. We do not, because of *when* the index is built.

`build_index` runs in `run_stdio`, before the LSP handshake exists. By the time
`initialize` delivers a `rootUri`, the server is already holding an index of the CLI
path. Re-anchoring the URI space on a different root without re-indexing would
publish one tree's analysis under another tree's URIs — which is **exactly the #76
defect, reintroduced from the other end**, and a worse form of it, because it would
be systematic rather than incidental.

The alternatives were:

- *Re-index at `rootUri` inside `initialize`.* Correct, and a much larger change: it
  moves indexing behind the handshake, makes startup cost depend on the client, and
  needs an answer for a client that sends no `rootUri` at all. Deferred, not
  rejected.
- *Silently ignore `rootUri`.* What the code did. It is what made #75 read as
  "ignores the client's rootUri", and it leaves an editor spawned from the wrong
  directory with no way to tell why its answers look foreign.
- *Refuse to start on a mismatch.* Too strict: the mismatch is often benign (a
  symlinked path, a client naming a subdirectory) and refusing would break working
  setups to prevent a case the warning already surfaces.

Reporting the disagreement keeps the failure legible without pretending to a
capability the server does not have. A multi-root workspace is served as its first
folder with the same warning; serving it properly needs the deferred re-index.

## Consequences

- An editor at the default path gets openable symbols and correct per-file analysis
  for the first time.
- **A file outside the root now returns nothing where it used to return a confident
  wrong answer.** That is the intent, and it is a behaviour change for any caller
  that was relying — knowingly or not — on the cross-root match.
- A relative path that cannot be canonicalised now fails at startup instead of
  producing a server that answers everything wrongly.
- The deferred re-index remains the only way to honour `rootUri` properly.
