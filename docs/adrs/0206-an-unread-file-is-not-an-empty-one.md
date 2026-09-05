---
id: '0206'
title: An unread file leaves the index rather than entering it empty, and a size cap decides before the read
status: ACCEPTED
date: 2026-09-03
triggered_by: issue #40 (read_to_string().unwrap_or_default() at four index sites)
---

# An unread file is not an empty one

## Context

Four sites on the index path read content as
`std::fs::read_to_string(..).unwrap_or_default()` — `index/mod.rs` in `build`, in
`build_with_content`'s content-map miss, and in `incremental_rebuild`, plus `commands/serve.rs`
before parsing. That default collapsed four different states into one empty string:

- a permission or IO error,
- content that is not UTF-8 (`read_to_string` returns `InvalidData`),
- a file too large to hold in memory — there was no cap at all,
- a file that is genuinely empty.

The first three then entered the index carrying their **real** `size_bytes` beside a **zero**
token count, an inconsistency the index already held both halves of.

The cost was not one degraded run. `serve` hashed that empty content into the stat-index under the
file's real `(mtime_ns, size_bytes)` key. A later successful read with unchanged stat hit the
index, reused `sha256("")`, left the content fingerprint unchanged, and the derived cache went on
serving analysis built over a file cxpak had never read. The emptiness was sticky.

There is a comment in `serve.rs` reasoning carefully about failed **metadata** reads and why
`(0, 0)` is safe for them. Failed **content** reads had no equivalent argument. The asymmetry was
the gap, not a decision.

## Decision

**An unreadable file is not in the index.** `read_indexable` returns `Option<String>` and never an
empty string for a file it could not read; every caller receiving `None` skips the file entirely.
It contributes to neither `total_files`, `total_bytes`, nor the language stats, and because
`serve`'s `fp_files` is built from `index.files`, it never reaches the fingerprint either — which
is what cuts the poisoning chain at one place rather than four.

**A genuinely empty file is still indexed.** It was read successfully and it is empty. Skipping it
would re-merge the two states this change exists to separate, so it is pinned by a test rather
than left to reading.

**The cap is decided from `size_bytes`, before the read.** `CXPAK_MAX_FILE_BYTES`, default
**5 MiB** — comfortably above any hand-written source file, far below the checked-in multi-GB
generated `.sql`/`.json` this exists to stop. Applying it to bytes already read would defeat the
purpose: the memory is spent by then.

**Zero and unparseable values take the default rather than disabling the cap.** A limit that
silently becomes infinite is the failure being guarded against, not a way to opt out of it. A user
who wants a bigger cap sets a bigger number, and the skip warning names the variable so they know
it exists.

**The reason is recorded on stderr**, in the existing `cxpak: warning: ...` idiom already used for
cache-save failures and unindexable paths. No new surface, and the message carries the OS error
rather than a category of our own, so a permission problem and a non-UTF-8 file read differently.

## Consequences

### Positive
- An unreadable file can no longer be mistaken for an empty one, at any of the four sites.
- The fingerprint cannot be poisoned with `sha256("")`, because the file never reaches `fp_files`.
- A multi-GB checked-in file is skipped on its stat rather than loaded to discover it is large.

### Negative
- `total_files` and `total_bytes` now exclude unreadable files, so a repo with such files reports
  a smaller index than before. That is the honest number, but it is a change in a user-visible
  figure.
- The cap is a hardcoded default. 5 MiB is a judgement, not a measurement; a repo with legitimate
  larger source files must set the variable, and will learn that from the warning rather than
  from documentation they already read.

### Neutral
- `commands/hook.rs` keeps `unwrap_or_default()` at three config/attrs reads, where an absent file
  legitimately means empty. Its fourth such read is inside `#[cfg(test)]`.

## Revisit if
- Users need unreadable files represented in the index rather than absent from it — a distinct
  `IndexedFile` state rather than an omission.
- The 5 MiB default starts costing real source on real repositories.
