---
id: '0175'
title: Surface EdgeConfidence in outputs and an LSP diagnostic
status: ACCEPTED
date: 2026-06-30
triggered_by: cxpak 3.0.0 Phase A, Task A3
loop: implementation
---

# ADR-0175: Surface EdgeConfidence in outputs and an LSP diagnostic

## Context

Task 0.4 added `EdgeConfidence { Extracted, Inferred }`
to every `TypedEdge`: structural edges (imports, FKs, ORM fields, the column→table
anchor) are `Extracted`; heuristic edges (embedded-SQL regex, cross-language bridge
detection, `SELECT *` / heuristic column refs) are `Inferred`. Until now the field
existed only in the data model — it was carried in the dependency graph and the cache
schema but never shown to a user.

cxpak's positioning (the descriptive-honesty principle of ADR-0097 — report what IS,
never overclaim) is "every edge proven, never inferred — and when it IS inferred,
say so." Honoring that means the `Inferred` edges must be **visibly distinguishable**
wherever edges are rendered, so a reader never mistakes a regex guess for a structurally
proven dependency. This is a presentation/honesty decision, not something the code can
resolve on its own: the tag syntax, whether to also tag `Extracted`, and the LSP
severity are all judgement calls about how loud the signal should be.

## Options considered

- **Option A — Mark only `Inferred`, as a textual suffix on the existing edge label,
  plus an LSP `INFORMATION` diagnostic.** Edge renderings already print
  `(via: <edge_type>)` (trace / auto_context) or `(<edge_type>)` (overview) for
  non-import edges; append `, inferred` to that label only when the edge is `Inferred`.
  `Extracted` edges render exactly as before. Pros: zero output change for the common
  (structural) case, so existing goldens and consumers are undisturbed; the `inferred`
  token is greppable; the rendering is driven by `edge.confidence`, not by edge type, so
  the per-edge `ColumnReference` split (some Extracted, some Inferred) is honored. Cons:
  in JSON/XML the tag is text inside an already-rendered section string rather than a
  structured field.
- **Option B — Tag both `Extracted` and `Inferred` explicitly** (e.g. every edge gains
  `, extracted` / `, inferred`). Pros: maximally explicit, symmetrical. Cons: doubles
  the noise on the overwhelmingly common structural edges for no new information (absence
  of `inferred` already means extracted); changes every edge-bearing golden.
- **Option C — Add a structured `confidence` field/attribute to a per-edge JSON/XML
  representation** in overview/trace output. Pros: machine-consumable. Cons: overview
  and trace do **not** serialize edges structurally — they render Markdown sections that
  the JSON/XML emitters embed verbatim as strings (`OutputSections` is `String`-typed).
  There is no per-edge JSON node to attach an attribute to without inventing a new
  structured edge format, which is out of scope for this task. The genuinely structured
  edge surface — `TypedEdge`'s own `serde` serialization (cache, graph dumps) — already
  carries `confidence` as a real enum field as of Task 0.4, so no string hack is
  introduced there.

## Decision

Option A. Mark only `Inferred` edges, with a `, inferred` suffix appended to the
existing per-edge label across all three human-readable edge renderings — overview
`render_dependency_graph` (`(embedded_sql, inferred)`), trace
`render_dependency_subgraph` (`(via: embedded_sql, inferred)`), and the auto_context
dependency annotation (`parent (via: embedded_sql, inferred)`). Because overview/trace
JSON and XML embed these Markdown sections verbatim, the marker flows into all three
formats consistently as rendered text; the structured `TypedEdge` serde surface keeps
carrying `confidence` as a typed field (unchanged from Task 0.4), so no "string hack" is
added to structured output.

The marker is driven by `edge.confidence.is_inferred()`, never by edge type, so a
`ColumnReference` edge stamped `Extracted` (ORM field / column→table anchor) renders
untagged while a heuristic one renders tagged.

Import edges are always `Extracted` by construction and render with no label, so they
are never tagged.

A single canonical `EdgeType::label()` is introduced in `core_graph::graph` as the one
source of truth for edge-type spelling (overview, trace, auto_context, and the LSP
diagnostic previously inlined four identical `match` arms that could drift).

For the LSP, `diagnostics_for_file` emits one `INFORMATION`-severity diagnostic
(`source: cxpak`, code `cxpak.inferred_edge`, **no** `UNNECESSARY` tag) per `Inferred`
outgoing dependency edge from the active file, anchored at the file head (edges carry no
line numbers). `INFORMATION` (not `WARNING`/`HINT`) was chosen because an inferred edge
is correct information about provenance, not a problem to fix and not a deemphasized
hint. The dead-code `WARNING` path is preserved unchanged and now runs independently of
the inferred-edge path (a file with no parse result still surfaces inferred edges).

## Consequences

### Positive
- Readers can tell heuristic dependencies from structural ones at a glance, in every
  output format and in-editor, fulfilling the "say so when inferred" contract.
- Common structural edges are byte-unchanged, so the determinism golden
  (`spa_determinism`) stays identical and existing consumers see no churn.
- `EdgeType::label()` removes four divergent copies of the edge-type spelling.

### Negative
- In JSON/XML the `inferred` marker lives inside the embedded section string, not as a
  discrete field — a machine consumer must parse the rendered text (the structured
  `TypedEdge` serde surface remains the field-typed path).
- The auto_context target tuple grew a fifth element (`EdgeConfidence`), threaded
  through the pack pipeline.

### Neutral
- Only outgoing inferred edges produce an LSP diagnostic; incoming inferred edges (whose
  heuristic lives in another file) are not surfaced when viewing the target file.

## Revisit if

- A consumer needs per-edge confidence as a discrete JSON/XML field in overview/trace
  output — that would require a structured edge format (Option C) and a new ADR.
- The LSP `INFORMATION` diagnostics prove too noisy in editors for large inferred-edge
  fan-out, suggesting a HINT severity or a client-side toggle.
- A future edge type blurs the Extracted/Inferred line such that a binary tag is no
  longer honest (e.g. a graded confidence score), invalidating the two-value model.
