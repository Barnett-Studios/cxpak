---
id: '0180'
title: LSP-first iterative retrieval (search → references → expand) over cxpak's own index
status: ACCEPTED
date: 2026-07-02
triggered_by: cxpak 3.0.0 Phase C, Task C1
loop: implementation
---

# ADR-0180: LSP-first iterative retrieval (search → references → expand) over cxpak's own index

## Context

Phase C opens the retrieval loop: an agent needs to `search` the codebase, pick a
hit, find its `references`, and `expand` to related nodes — repeatedly — to gather
context. cxpak already indexes symbols, content, an inverted `symbol → files`
cross-reference map, and a typed dependency graph. The requirement is a
deterministic, reproducible retrieval loop over **cxpak's OWN index** (never an
external language server), exposed IDENTICALLY on all four live surfaces (LSP,
MCP, HTTP, CLI), with a `readOnly` annotation on the LSP side.

Two decisions belong to a human rather than the code: (1) how retrieval is placed
in the capability catalog without breaching the ≤8 MCP-tool budget, and (2) how
the "same inputs → byte-identical outputs across runs and surfaces" contract is
guaranteed given the index is backed by `HashMap`/`HashSet`.

## Options considered

- **Option A — one `retrieval` capability, three internal ops, one core `execute`
  (chosen):** mirror B1's `graph` capability exactly — a single
  `intelligence::retrieval::execute(index, op, params)` core with `op ∈
  search|references|expand`, riding as one `op` under the `Context` intent-tool.
  Pro: one core, zero re-derivation, the catalog adapter already projects it to
  all four surfaces, tool count unchanged. Con: the three ops share one capability
  id, so per-op schemas would need sub-typing later.
- **Option B — three separate capabilities (`search`/`references`/`expand`):**
  each its own catalog entry under `Context`. Pro: per-op metadata/schema is
  natural. Con: three entries where B1 established the one-capability/many-ops
  precedent; more surface wiring; still one intent-tool but noisier `op` list.
- **Option C — a new top-level `cxpak_retrieval` MCP tool:** most discoverable.
  Con: grows the MCP tool count toward the ≤8 ceiling for no structural reason —
  exactly what the catalog architecture (Task 0.6) exists to prevent.

## Decision

Option A. Add a single deterministic core `intelligence::retrieval` with three
pure primitives over `&CodebaseIndex`/`&DependencyGraph`:

- `search(query, limit)` — reuses symbol iteration (`find_symbol`-style) +
  `find_content_matches`; ranks by a match tier (`exact > prefix > substring >
  content`) with a bounded PageRank boost.
- `references(symbol)` — reuses `pagerank::build_symbol_cross_refs` with the SAME
  `normalize_identifier` key `symbol_importance` uses.
- `expand(seeds, depth)` — delegates verbatim to the B1 `graph_query::subgraph`.

`execute` is the single dispatch entry point every surface calls. The catalog
gains one `retrieval` capability under `Intent::Context`, projected to MCP (as
`op=retrieval` under `cxpak_context`), LSP (`cxpak/retrieval`), CLI (`cxpak
search`), and HTTP (`POST /v1/retrieval`). The MCP tool count is unchanged (still
≤8).

**Deterministic total order per op (hard contract):** all output is
byte-deterministic; no `HashMap`/`HashSet` iteration order leaks into any output.
- `search`: iterate `index.files` (a `Vec`), sort hits by `(score desc via
  f64::total_cmp, path, symbol, start_line, match_kind)` — every tiebreak key is
  total, so equal scores never reorder — then truncate to `limit`.
- `references`: collect the matching file set into a `Vec`, then `sort` + `dedup`.
- `expand`: inherits `graph_query::subgraph`'s determinism (BTree-backed graph,
  sorted nodes + induced edges).

**readOnly annotation:** a `read_only: bool` field on `Capability` (every cxpak
capability is read-only by construction), plus a `READ_ONLY_METHODS` registry and
`method_is_read_only()` predicate in `lsp::methods` covering `workspace/symbol`,
`cxpak/search`, and `cxpak/retrieval`.

**C3 reconciliation note:** the legacy regex MCP `cxpak_search`, LSP `cxpak/search`
+ `workspace/symbol`, and HTTP `/search` handlers are left untouched — they have
different (substring/regex) semantics, so routing them through the new core would
be behaviour-breaking. C3 (the 26→8 consolidation) reconciles the duplicates;
C1 only ADDS the catalog retrieval capability and its four shims.

## Consequences

### Positive
- One core, four surfaces, no re-derivation (ADR-0153 invariant holds); the
  loop is byte-reproducible across runs and surfaces.
- MCP tool budget untouched — retrieval rides under `cxpak_context`.
- Maximum reuse: `expand` is literally `graph_query::subgraph`; `references` is
  the existing cross-ref map; `search` reuses the existing symbol/content scans.

### Negative
- Two search paths coexist until C3 (new catalog retrieval vs legacy regex
  `search`), a temporary duplication the budget/conformance gates tolerate.
- One capability id for three ops defers per-op JSON schemas.

### Neutral
- `read_only` is `true` for every current capability; the field earns its keep
  once a mutating capability (none today) would set it `false`.

## Revisit if

- A retrieval op needs a pinned per-op output schema (promote to Option B, or add
  sub-schemas keyed by op).
- C3 removes the legacy `search` duplicates and the new core becomes the only
  search path (drop the coexistence note).
- Cross-surface byte parity is ever required for the PageRank-boosted `score`
  across platforms (the `spa_determinism` FMA caveat would then apply here too).
