---
id: '0074'
title: allocate_with_degradation operates on &[(&IndexedFile, FileRole, f64)] references, not owned files
status: ACCEPTED
date: 2026-03-20
triggered_by: IndexedFile did not implement Clone and must not be moved out of the index
loop: implementation
---

# ADR-0074: allocate_with_degradation operates on &[(&IndexedFile, FileRole, f64)] references, not owned files

## Context
The v0.11.0 budget allocation entry point was designed to borrow files rather than
own them. The shipped signature is:

```rust
pub fn allocate_with_degradation(
    files: &[(&IndexedFile, FileRole, f64)],
    budget: usize,
    pagerank: Option<&HashMap<String, f64>>,
) -> Vec<AllocatedFile>
```

(`AllocatedFile` is a named struct with `path: String`, `level: DetailLevel`,
`symbols: Vec<DegradedSymbol>`. The original implementation plan specified a raw
tuple `Vec<(String, DetailLevel, Vec<DegradedSymbol>)>`, which was later refactored
into this struct. The `pagerank` parameter was added after the original plan to
feed the priority formula `score*0.6 + cp*0.2 + pr*0.2`.)

It takes references because, at decision time (2026-03-20), `IndexedFile` was
`#[derive(Debug)]` only — not `Clone` — and must not be moved out of the index.
Note this premise is now stale: `IndexedFile` gained `#[derive(Debug, Clone)]` on
2026-04-16 (commit 368508d). The by-reference signature was nonetheless kept,
because the index remains authoritative and cloning is unnecessary.

The function lives in `src/context_quality/degradation.rs`, so `src/budget/mod.rs`
needed no changes.

## Options considered
- **Option A — borrow files by reference:** pass `(&IndexedFile, role, score)`
  tuples and return owned path/level/symbols. Pros: avoids cloning a non-`Clone`
  `IndexedFile`; the index stays authoritative. Cons: the caller must marshal
  references into the tuple shape. Someone could prefer it to keep the index as the
  single owner of file data. (Chosen.)
- **Option B — take owned or cloned `IndexedFile` values:** move or clone files
  into the allocator. Pros: self-contained input, no borrow lifetimes to thread.
  Cons: `IndexedFile` was not `Clone` at the time, so this would require moving data
  out of the index. Someone could prefer it once `IndexedFile` is `Clone` and a
  simpler ownership story is wanted.

## Decision
Define `allocate_with_degradation(files: &[(&IndexedFile, FileRole, f64)], budget:
usize, pagerank: Option<&HashMap<String, f64>>) -> Vec<AllocatedFile>`, using
references because `IndexedFile` was not `Clone` and must not be moved out of the
index. Place it in `src/context_quality/degradation.rs` so `src/budget/mod.rs` is
untouched.

## Consequences
### Positive
- No cloning of (then) non-`Clone` index files.
- `src/budget/mod.rs` required no changes.

### Negative
- Callers must assemble `(&IndexedFile, role, score)` tuples.

### Neutral
- The function returns owned outputs as a `Vec<AllocatedFile>` struct (path, level,
  degraded symbols) — refactored from the originally planned raw tuple.

## Revisit if
- The `IndexedFile`-is-not-`Clone` premise has already fired: `Clone` was added on
  2026-04-16, yet the by-reference signature was kept. Revisit only if a simpler
  owned-input signature is now preferred over keeping the index authoritative.
- Allocation needs to own files for caching.
