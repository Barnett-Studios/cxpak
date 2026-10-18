---
id: '0215'
title: Attribute scenario C max RSS by phase before cutting; "drop content from Prepared" is a no-op because content moves into the index
status: PROPOSED
date: 2026-10-18
triggered_by: ADR-0213 Outcome (scenario C max RSS 570 MB vs 504 MB limit) and its "Revisit if" naming Prepared.content
loop: planning
---

# ADR-0215: Attribute scenario C max RSS by phase before cutting

## Context

ADR-0213 shipped with scenario C max RSS at 569.5 MB against 504.2 MB (3.4.0 x 1.10); footprint and
scenario U meet the gate. Its fallback was to drop `content` from `Prepared` (`serve.rs:370`).
Tracing shows the file text is moved from the settled read (`serve.rs:483`) into `Prepared`
(`serve.rs:543`) and then into the public `IndexedFile.content` (`src/index/mod.rs:692`), where
tools, LSP, hooks and convention miners read it. It is never copied, so removing it from `Prepared`
frees nothing; removing it from the index is a public-API and behaviour change. The 185 MB gap
between C and U comes from the background mining/republish phase, which is not yet attributed.

## Options considered

- **Option A — drop `content` from `Prepared` as ADR-0213 worded it.** No API risk, small diff.
  Rejected on evidence: ownership moves, so peak is unchanged. Someone could prefer it for being
  the documented next step; it would need a measurement showing otherwise.
- **Option B — drop or lazily load `IndexedFile.content`.** The only change that removes the text
  itself (~the whole repo size). Rejected for 3.5.x: `IndexedFile` is `pub`, every consumer in
  section 2 of the spec reads the field; this is a semver-minor/major design (separate ADR).
- **Option C — attribute first, then cut the transient duplicates.** Add phase-tagged RSS sampling
  (build, fold, derived load, mining, republish, store/derived save). Then target what the data
  names, within no public API change: the `Prepared.fresh`/`changed` artifact duplicates, mining
  allocations, release of build transients before the mining phase starts. Pros: evidence-led, API
  safe. Cons: may not reach 504 MB; macOS max RSS is a high-water mark that frees do not lower, so
  only avoided peaks count.
- **Option D — accept 570 MB.** Footprint (the gate metric the product cares about) passes; document
  that RSS gate is unmet. Legitimate if C fails to move; costs the stated 1.10x gate on RSS.

## Decision

Option C. Attribution first (RED test pins whichever duplicate the data names), Option A is not
implemented. If measurement shows no no-API lever reaches 504 MB median, ship the best measured
reduction (or none) with an honest unmet-gate note, and open a separate ADR for Option B.

## Consequences

### Positive
- No public API change; decision follows a measured attribution rather than a hypothesis.
### Negative
- The outcome is uncertain; gate may remain unmet.
- Phase sampling needs a small, non-default instrumentation hook or an external sampler.
### Neutral
- Content stays resident in the index; steady-state memory is unchanged.

## Revisit if

- Attribution shows the C-only peak is mining allocations inherent to `build_convention_profile`
  (not duplicates): then only Option B or a streaming miner moves it.
- A consumer audit shows `IndexedFile.content` can become lazy without breaking its readers.
