---
id: '0184'
title: RRF fusion + contextual retrieval + optional local reranker (D2-gated semantic upgrade)
status: ACCEPTED
date: 2026-07-03
triggered_by: cxpak 3.0.0 Phase D (D1) — semantic retrieval upgrade
loop: implementation
---

# ADR-0184: RRF fusion + contextual retrieval + optional local reranker

## Context

`MultiSignalScorer::score_all` combined its seven relevance signals as a linear
weighted sum (`combined = Σ wᵢ·signalᵢ`). A weighted sum mixes signals on their
raw *magnitudes*: a signal that happens to produce large absolute values for a
query dominates one that separates the relevant files just as well but on a
smaller numeric range. The D1 task is to upgrade retrieval quality with three
pieces — Reciprocal Rank Fusion, contextual retrieval, and an optional local
reranker — **without regressing the D2 recall gate** (`recall@budget ≥ the
current-code baseline`, measured by the harness in `src/bench/recall.rs`).

Two hard constraints shape the design and belong to a human decision rather than
the code:

1. **The D2 recall gate is a user surface point.** Whether the upgrade ships
   ACTIVE is decided by the controller + user after a full-corpus A/B, not by the
   implementer. The committed `bench/baseline.json` is stale (regen deferred to
   Phase R), so the fair test is **current-code (inert) vs D1 (active)**, A/B,
   index-once — the same method C2 used (ADR-0181).
2. **Determinism + no-LLM + no-OpenSSL.** Core ranking must stay byte-stable; no
   LLM in any ranking path; no heavy/native dependency (ADR-0163).

## Options considered

- **Option A — Unweighted RRF replaces the weighted sum.** Standard
  `score = Σ 1/(k+rankᵢ)`. Pros: textbook simple. Cons: discards the hand-tuned
  signal weights (symbol_match 0.27 vs recency 0.05 become equal voters), and the
  raw score maxes at `≈ Σ 1/(k+1) ≈ 0.11`, far below the `SEED_THRESHOLD = 0.10`
  scale — seed selection would collapse to near-empty, a catastrophic recall
  regression. A stakeholder might still prefer it for fidelity to the RRF paper.
- **Option B — Weighted, scale-normalized RRF replaces the weighted sum
  (CHOSEN).** `rrfᵢ = Σⱼ wⱼ·(K+1)/(K+rankᵢⱼ)`. Keeps the tuned weights meaningful
  and normalizes each term to `(0,1]` so the fused score stays on the weighted
  sum's `[0,1]` scale — the seed threshold remains calibrated (recall safety).
- **Option C — Fuse RRF *with* the weighted sum (blend).** `α·weighted +
  (1−α)·rrf`. Pros: conservative. Cons: introduces a second tuning knob (`α`)
  with no principled value, and muddies the A/B (neither pure ranking). Rejected
  as under-motivated.

For the A/B control: a **runtime `RelevanceMode`** (Option B') was chosen over a
compile-time const flip (C2's original approach, which needs two builds and is
reaping-prone) so the harness scores both modes from a SINGLE index build.

## Decision

**RRF (weighted, scale-normalized) replaces the linear combine in `Active`
mode.** For each signal `j`, rank the files by `signalⱼ` descending (path-ascending
deterministic tiebreak, 1-based rank), then

```text
rrfᵢ = Σⱼ  weightⱼ · (K + 1) / (K + rankᵢⱼ)          K = RRF_K = 60
```

`K = 60` is the standard constant (Cormack, Clarke & Büttcher, SIGIR 2009). The
`(K+1)` numerator normalizes each reciprocal-rank term to `(0,1]` (a rank-1 file
contributes exactly `weightⱼ`), so with `Σ weights = 1` the fused score lands on
the SAME `[0,1]` scale as the weighted sum — keeping `SEED_THRESHOLD` calibrated.
A weight-0 signal is skipped (it cannot affect the fusion, and skipping it keeps
the embeddings-off path free of path-order tie noise from the constant neutral
embedding scores). The C2 boost-only `identifier_rank` factor and the `[0,1]`
clamp are applied to the fused base unchanged. `rrf.rs::fuse` is the pure core.

**Contextual retrieval** (`index::build_context_header`, Anthropic "contextual
retrieval"): at index time, in `Active` mode, `build_embedding_index` prepends a
deterministic graph-context header to each symbol's embedded text:

```text
// file: src/api/mod.rs | depends on: config.rs, middleware.rs | used by: server.rs
```

Dependency/dependent basenames come from the prebuilt `DependencyGraph`
(`dependencies` is a `BTreeSet`, `dependents` set-backed), de-duplicated, sorted,
and capped at `CONTEXT_HEADER_MAX_NEIGHBORS = 8`. No LLM; the same index yields
byte-identical headers. In `Inert` mode the bare signature is embedded (pre-D1,
byte-identical). Note: the D2 harness has no persisted embedding index (the
embedding signal is neutral there), so contextual retrieval did not move the
D2 A/B. At the time of this ADR `build_embedding_index` had no production caller,
so contextual retrieval (piece 2/3) shipped reachable-in-principle but unexercised
and unmeasured. **UPDATE (R-E1, ADR-0186):** `build_embedding_index` is now wired
as an opt-in background build, so contextual retrieval is reachable in production
whenever embeddings are configured (`.cxpak.json`) AND mode is `Active`. Its recall
impact remains unmeasured here (opt-in and correctness-gated per R-E1; not
re-measured at the R-D1 flip). It is shipped and unit-tested for determinism.

**Optional local reranker** (`relevance::reranker`, behind the NON-default
`reranker` Cargo feature): a deterministic, no-LLM, no-new-dependency lexical
cross-encoder that re-orders the top-N (`DEFAULT_TOP_N = 20`) fused candidates by
jointly featurizing the query against each file (query↔symbol-name/path token
overlap, weighted by symbol importance). It re-orders the top-N *among
themselves only* — never drops a candidate, never promotes from outside top-N,
and reassigns the same multiset of top-N scores — so it cannot regress the
ranking *set* above any prefix ≥ N. (It does not guarantee recall under a smaller
*budget* cut inside the top-N: a token budget admitting only the first k < N
files can drop a file the reranker demoted within the top-N.) OFF by default;
only fires in `Active` mode; **excluded from the determinism fixture** (the
fixture builds with default features + Inert). A
model-backed transformer cross-encoder would live behind this SAME flag but needs
a heavy native runtime — deferred per ADR-0163 / the D1 stop-and-ask rule.

**A/B control:** `RelevanceMode { Inert, Active }` — a runtime field on
`MultiSignalScorer` (`.with_mode()`) and the product-level `auto_context_with_mode`.
`Inert` reproduces the pre-D1 ranking byte-for-byte; `Active` enables the upgrade.
Both are reachable from ONE index build, so the D2 harness measures both modes
index-once. **`DEFAULT_RELEVANCE_MODE = Inert`** shipped at D1: the machinery is
built, tested, and measurable, but flipping the default to `Active` was gated on
the full-corpus recall A/B (controller + user), exactly as C2 shipped inert.
**UPDATE (R-D1, ADR-0187):** the full-corpus A/B cleared the gate (+164% recall)
and the default is now `Active`; `Inert` remains available as a byte-stable
control via `.with_mode()` / `auto_context_with_mode`.

## D2 recall A/B (measured)

Pinned subset (ripgrep#3420, flask#5928, express#7234), `cxpak (auto_context)`
gate metric, inert vs active, index-once:

| mode   | recall@8k | recall@32k | MRR    |
|--------|-----------|------------|--------|
| INERT  | 0.5000    | 0.4667     | 0.4104 |
| ACTIVE | 0.6111    | 0.6444     | 0.7333 |

Active ≥ inert on **both** budgets (+0.111 @8k, +0.178 @32k) and MRR (+0.323).
The inert row reproduces the current-code numbers exactly (confirming the A/B is
fair and the pipeline deterministic). Per-entry: ripgrep unchanged (perfect both);
flask @32k 0.4→0.6; express @8k/@32k 0.0→0.333, MRR 0.031→1.000. On the pinned
subset the D2 rule (`active ≥ baseline on both budgets → ship active`) is met —
but the ship/tune/defer decision is deferred to the controller's full-corpus
validation + the user gate; `DEFAULT_RELEVANCE_MODE` ships `Inert` until then, and
`bench/baseline.json` is untouched.

## Consequences

### Positive
- RRF fuses signals on equal (rank) footing; clear recall + MRR gains on the subset.
- A/B control ships in the product — no throwaway measurement code; future re-A/Bs are one flag.
- Contextual headers give the embedding signal relational meaning, deterministically.
- Reranker architecture is in place behind a flag with zero new deps / no determinism impact.

### Negative
- `score_all` now materializes all per-file signal vectors before combining (RRF needs the whole population for ranks) — a second pass over the file set. Inert is unchanged arithmetically but pays the same materialization.
- The shipped default is still Inert, so users get the upgrade only after the gate flips it (intentional, gate-respecting).

### Neutral
- Single-file `RelevanceScorer::score` stays inert (RRF needs global ranks); documented, mirroring C2's neutral single-file identifier factor.
- Contextual retrieval is inert in the current D2 harness (no persisted embedding index); its gain lands at the Phase-R re-baseline.

## Revisit if
- The full-corpus D2 A/B confirms/refutes the subset gain → flip `DEFAULT_RELEVANCE_MODE` (and regen `bench/baseline.json`) or tune/defer.
- `score_all` becomes hot at scale (the signal-materialization pass) → stream ranks instead of collecting.
- A model-backed cross-encoder is wanted → wire it behind the existing `reranker` feature (stop-and-ask for the native dep first, ADR-0163).
- Contextual-retrieval recall is measured with embeddings active and shifts the baseline.
