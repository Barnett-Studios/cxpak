---
id: '0186'
title: Opt-in background embedding enrichment on the MCP readiness seam
status: ACCEPTED
date: 2026-07-04
triggered_by: Task R-E1 (cxpak 3.0.0 Phase 0) — activate the dead embeddings plumbing
loop: implementation
---

# ADR-0186: Opt-in background embedding enrichment on the MCP readiness seam

## Context

cxpak ships a documented similarity signal (#7) driven by `build_embedding_index`
(`src/index/mod.rs`), but that function had **zero production callers**:
`CodebaseIndex.embedding_index` was always constructed `None`, so
`has_embedding_index()` always returned false and the scorer always used the
6-signal weight vector. The feature was fully implemented but never activated.

Activating it naively is a trap. `EmbeddingConfig::from_repo_root` /
`from_json` **fall back to `local_default()`** (a local MiniLM config) whenever
`.cxpak.json` is missing or has no `"embeddings"` key. So an unconditional
`build_embedding_index` call would download the ~30 MB MiniLM model for *every*
repo and change ranking for users who never opted in — violating the hard gate
"default no-config path unchanged / golden byte-identical"
(`spa_output_matches_golden_fixture`).

The build itself is slow (model download / remote API round-trips) and must not
block the MCP `initialize` handshake — R0 (ADR-0185) already moved the base
index build onto a background `std::thread` behind an `IndexReadiness` cell and
deliberately left the enum extensible for exactly this phase. This is a human
decision because it trades three things a reasonable maintainer could weigh
differently: *when* embeddings build, *how* they attach to an already-shared
immutable index, and *what happens on failure*.

## Options considered

- **Opt-in gate — `_if_configured` returning `Option` (chosen):** add
  `EmbeddingConfig::from_repo_root_if_configured` / `from_json_if_configured`
  that return `Some` **only** when `.cxpak.json` exists and declares an
  `"embeddings"` section, `None` otherwise (no local-default fallback). Serve
  builds embeddings only on `Some`. Pro: the default path never downloads a
  model, golden byte-identical; existing callers of `from_repo_root` keep their
  fallback. Con: two config entry points with different absent-key semantics.

- **Reuse `from_repo_root` + a separate boolean "enabled" flag:** rejected — the
  fallback-to-local behavior is the whole hazard; a parallel flag duplicates the
  "is it configured?" question the JSON already answers and invites the two to
  drift.

- **Attach mechanism (a) — `ReadyEnriched(Arc<CodebaseIndex>)` enum variant
  (chosen):** phase 2 clones the ready base into a local, sets
  `embedding_index: Some(..)`, and swaps `ReadyEnriched(Arc::new(enriched))`
  under a brief write lock; `snapshot_ready_index` treats `ReadyEnriched` exactly
  like `Ready`. Pro: same snapshot-then-swap discipline R0 already proved;
  `Ready(Arc<CodebaseIndex>)` stays immutable so in-flight readers are never torn;
  the enrich is one O(1) `Arc` swap, no lock held across the build. Con: a full
  `CodebaseIndex` clone (off-lock).

- **Attach mechanism (b) — interior mutability on `embedding_index`
  (`RwLock`/`OnceLock` on the field):** rejected — it threads a second lock
  through every reader of a hot field for a one-time write, muddies the
  "index is an immutable snapshot" invariant, and buys nothing over an O(1)
  whole-index `Arc` swap that the readiness cell already supports.

- **Fatal vs. non-fatal embedding failure:** rejected fatal. An embedding
  failure (missing API key, no network, model-download failure, or a panic in
  the provider/loader) must leave the base `Ready` and serving on 6 signals.

## Decision

Add the opt-in `_if_configured` config gate and a **two-phase background
publish** riding R0's readiness seam:

1. **Phase 1 (R0, unchanged):** build the base index, publish `Ready(base)`
   ASAP; tool calls serve immediately on the 6-signal path.
2. **Phase 2 (new, opt-in):** in the *same* background thread, after publishing
   `Ready`, if `from_repo_root_if_configured(repo)` is `Some`, build the
   embedding index under `catch_unwind` and swap in
   `ReadyEnriched(Arc<CodebaseIndex>)` (base + `embedding_index: Some(..)`). The
   enriched index is built into a local first; only the O(1) `Arc` swap holds
   the write lock, with the same poison-recovery R0 uses. `snapshot_ready_index`
   returns the index for `Ready | ReadyEnriched` identically.

`build_embedding_index` is passed the current `DEFAULT_RELEVANCE_MODE`
(`Inert` when this ADR was written; flipped to `Active` in ADR-0187, read
dynamically) — **not** a hard-coded value. Scope is **MCP serve
only**; CLI keeps constructing `embedding_index: None`. All new serve wiring is
`#[cfg(feature = "embeddings")]`; with the feature off, behavior is exactly
today's. The whole path is opt-in and excluded from the determinism fixture.

## Consequences

### Positive
- Default no-config path is byte-identical: `None` gate → no build, no model
  download, no network, 6-signal, golden unchanged.
- Model download / remote calls happen off the handshake and off the base-ready
  path; the base serves 6-signal the instant it is ready.
- A racing `tools/call` sees either the pre-swap `Ready` (6-signal) or the
  post-swap `ReadyEnriched` (7-signal) — never a torn or panicking state.
- Embedding failure is non-fatal: any error/`None`/panic leaves the base
  `Ready`; the server never wedges, never reports `Failed` for an embedding
  problem, never hangs.
- No new dependencies (candle + remote providers already exist behind the
  default `embeddings` feature; remote uses reqwest/rustls — no OpenSSL).

### Negative
- Phase 2 clones the whole `CodebaseIndex` once to attach the field (off-lock;
  acceptable for a one-time enrich).
- Two config constructors with different absent-key semantics
  (`from_repo_root` local-defaults; `from_repo_root_if_configured` returns
  `None`) — documented on both.

### Neutral
- Embeddings are not persisted across serve starts. Because the mode passed is
  `DEFAULT_RELEVANCE_MODE` at build time, when the later R-D1 task flips the
  default to `Active` a fresh serve simply rebuilds embeddings in `Active` mode
  (contextual headers) — there is no stale-embedding problem to migrate.
- HTTP `serve` is out of scope (it builds up-front); left as-is.

## Revisit if

- The `IndexReadiness` cell gains a periodic-rebuild / long-lived-session model
  (today one serve keeps its first enriched index) — re-enriching on rebuild
  would need the same swap discipline applied on each cycle.
- Embeddings become persisted across serve starts — then the mode a persisted
  index was built in must be recorded and reconciled against the current mode
  (the "no stale" argument above no longer holds for free).
- The MiniLM model or a remote provider becomes cheap/instant enough that an
  up-front (non-background) build is acceptable, collapsing the two phases.
- A measured recall study shows the opt-in 7-signal path materially beats the
  6-signal default for the general (non-configured) case, motivating a default
  flip (which would move embeddings into the gated determinism fixture).
