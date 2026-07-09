# Migrating to cxpak 3.0.0 — MCP tool consolidation (BREAKING)

cxpak 3.0.0 replaces the 26 hand-rolled MCP tools with **five intent-parameterized
tools**. This is the only breaking change in the 3.0.0 release. It affects MCP
clients only — the CLI, the HTTP `/v1/*` API, and the LSP `cxpak/*` methods are
unchanged.

## What changed

Before 3.0.0 the MCP server advertised 26 top-level tools (`cxpak_auto_context`,
`cxpak_overview`, `cxpak_health`, …). Discovering that many tools is expensive
for a model's context, and the set kept growing with every feature.

3.0.0 groups every capability under one of five **intent-tools**, selected by an
`op` argument:

| Intent-tool | Purpose | `op` values |
|---|---|---|
| `cxpak_context` | Pack token-budgeted context | `context`, `retrieval`, `search`, `overview`, `stats`, `context_for_task`, `pack_context`, `briefing` |
| `cxpak_graph` | Query the dependency graph | `graph`, `trace`, `blast_radius`, `call_graph`, `dead_code`, `api_surface`, `data_flow`, `cross_lang`, `predict` |
| `cxpak_data` | Inspect the data layer | `data` |
| `cxpak_review` | Analyze changes for review | `review`, `diff`, `verify` |
| `cxpak_insight` | Report health / risk / architecture | `health`, `risks`, `architecture`, `conventions`, `security_surface`, `drift`, `visual`, `onboard` |

Every intent-tool's `inputSchema` has a **required `op`** (a string enum of the
values above) plus the parameters each op accepts (`additionalProperties: true`).
Each intent-tool advertises `annotations.readOnlyHint: true` — every cxpak
capability is read-only.

## How to call the new tools

Old:

```json
{"method": "tools/call", "params": {"name": "cxpak_health", "arguments": {}}}
```

New — select the capability with `op`:

```json
{"method": "tools/call", "params": {"name": "cxpak_insight", "arguments": {"op": "health"}}}
```

All the old per-tool arguments are unchanged; they now sit alongside `op`.

### Deprecated aliases (transitional)

For one release, the 26 old tool **names** still route to the same capability if
you call them directly (`tools/call` with `name: "cxpak_health"` still works).
They are **not discoverable** — they no longer appear in `tools/list` — and will
be **removed in a future release**. Migrate to the intent-tool + `op` form.

## Full 26 → (intent-tool, op) mapping

| Old MCP tool | New tool | `op` | Parameter changes |
|---|---|---|---|
| `cxpak_auto_context` | `cxpak_context` | `context` | add `op`; params unchanged (`task`, `tokens`, `focus`, `mode`, `cost_model`, …) |
| `cxpak_context_diff` | `cxpak_review` | `review` | add `op`; params unchanged (`since`, `focus`) |
| `cxpak_overview` | `cxpak_context` | `overview` | add `op`; params unchanged (`tokens`, `focus`) |
| `cxpak_trace` | `cxpak_graph` | `trace` | add `op`; params unchanged (`target`, `tokens`, `focus`) |
| `cxpak_diff` | `cxpak_review` | `diff` | add `op`; params unchanged (`git_ref`, `tokens`, `focus`, `review`) |
| `cxpak_stats` | `cxpak_context` | `stats` | add `op`; params unchanged (`focus`) |
| `cxpak_context_for_task` | `cxpak_context` | `context_for_task` | add `op`; params unchanged (`task`, `limit`, `focus`) |
| `cxpak_pack_context` | `cxpak_context` | `pack_context` | add `op`; params unchanged (`files`, `tokens`, …) |
| `cxpak_search` | `cxpak_context` | `search` | add `op`; params unchanged (`pattern`, `limit`, `focus`, `context_lines`). Legacy regex search, preserved verbatim; distinct from the newer `retrieval` op |
| `cxpak_blast_radius` | `cxpak_graph` | `blast_radius` | add `op`; params unchanged (`files`, `depth`, `focus`) |
| `cxpak_api_surface` | `cxpak_graph` | `api_surface` | add `op`; params unchanged (`focus`, `include`, `tokens`) |
| `cxpak_verify` | `cxpak_review` | `verify` | add `op`; params unchanged (`ref`, `focus`) |
| `cxpak_conventions` | `cxpak_insight` | `conventions` | add `op`; params unchanged (`category`, `strength`, `focus`) |
| `cxpak_health` | `cxpak_insight` | `health` | add `op`; params unchanged (`focus`) |
| `cxpak_risks` | `cxpak_insight` | `risks` | add `op`; params unchanged (`limit`, `focus`) |
| `cxpak_briefing` | `cxpak_context` | `briefing` | add `op`; params unchanged (`task`, `tokens`, `focus`, `cost_model`) |
| `cxpak_call_graph` | `cxpak_graph` | `call_graph` | add `op`; params unchanged (`target`, `depth`, `focus`, `workspace`) |
| `cxpak_dead_code` | `cxpak_graph` | `dead_code` | add `op`; params unchanged (`focus`, `limit`, `workspace`) |
| `cxpak_architecture` | `cxpak_insight` | `architecture` | add `op`; params unchanged (`focus`, `workspace`) |
| `cxpak_predict` | `cxpak_graph` | `predict` | add `op`; params unchanged (`files`, `depth`, `focus`) |
| `cxpak_drift` | `cxpak_insight` | `drift` | add `op`; params unchanged (`save_baseline`, `focus`) |
| `cxpak_security_surface` | `cxpak_insight` | `security_surface` | add `op`; params unchanged (`focus`) |
| `cxpak_data_flow` | `cxpak_graph` | `data_flow` | add `op`; params unchanged (`symbol`, `sink`, `depth`, `focus`) |
| `cxpak_cross_lang` | `cxpak_graph` | `cross_lang` | add `op`; params unchanged (`file`, `focus`) |
| `cxpak_visual` | `cxpak_insight` | `visual` | add `op`; params unchanged (`type`, `format`, `focus`, `symbol`, `files`) |
| `cxpak_onboard` | `cxpak_insight` | `onboard` | add `op`; params unchanged (`focus`, `format`) |

### New / re-keyed ops with a sub-selector

Two ops carry a nested operation. They accept a **renamed** sub-selector so it
does not collide with the top-level `op` discriminator:

| `op` | Sub-selector param | Values | Notes |
|---|---|---|---|
| `graph` (under `cxpak_graph`) | `graph_op` | `node`, `neighbors`, `path`, `subgraph` | The typed graph-query capability (ADR-0176). `neighbors`/`path`/`subgraph` output carries per-edge `edge_type` + `confidence` (`inferred`) — ADR-0175. |
| `retrieval` (under `cxpak_context`) | `retrieval_op` | `search`, `references`, `expand` | The iterative retrieval capability (ADR-0180). Distinct from the legacy `search` op. |

The `data` op (under `cxpak_data`) returns the indexed data layer (`SchemaIndex`:
`tables`, `views`, `orm_models`, `migrations`) — newly surfaced on MCP in 3.0.0.

## Notable non-breaking behavior changes in 3.0.0

Two changes affect *output*, not the API — no client code changes are required,
but you will notice different (better) results:

- **Retrieval defaults to Active RRF ranking.** Relevance scoring now fuses its
  signals with Reciprocal Rank Fusion instead of the prior weighted sum
  (ADR-0187, ADR-0188). It is deterministic and measured a large recall gain, so
  re-running cxpak returns different, better-ranked context for the same task.
  The prior weighted-sum path is retained only as an internal `Inert` A/B control.
- **Embeddings are opt-in.** The semantic similarity signal is now activated only
  when `.cxpak.json` declares an `"embeddings"` section (ADR-0186). Previously the
  signal existed in the code but was never built. With no config, cxpak uses its
  6 deterministic signals and downloads no model.

## Not affected

- **CLI** — all subcommands unchanged.
- **HTTP `/v1/*`** — all routes unchanged.
- **LSP `cxpak/*`** — all custom methods unchanged.

These surfaces already project the same capability cores; only the MCP tool
*surface* was reshaped. See ADR-0182 for the rationale.
