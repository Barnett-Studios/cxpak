//! Capability catalog — the single source of truth for "what cxpak can do",
//! independent of how each capability is surfaced (cxpak 3.0.0 Phase 0 capstone).
//!
//! # The one-core → five-projections architecture
//!
//! A [`Capability`] is a unit of analysis (e.g. `context`, `graph`, `health`)
//! that produces ONE core result. The five surfaces — MCP, LSP, CLI, HTTP,
//! Visual — are *projections* of that single result, never re-derivations
//! (ADR-0153 single-source invariant; the conformance gate in
//! `tests/surface_conformance.rs` enforces it).
//!
//! Originally (Task 0.6) this was a **parallel framework** built ALONGSIDE the
//! legacy 26-tool MCP handler in `src/commands/serve.rs`. **Task C3 (ADR-0182)
//! completed the migration**: the live `serve.rs` `tools/list` is now the
//! [`adapter::mcp_tools`] projection of this catalog, and every one of the 26
//! former MCP tools is reachable as an `op` under one of the five
//! `cxpak_<intent>` tools. Two CI gates every capability must pass:
//!
//! * **MCP ≤ 8 tools** — [`adapter::mcp_tools`] groups capabilities into at most
//!   eight intent-tools; a new capability rides as an `op` param under an
//!   existing tool, never as a new top-level MCP tool.
//! * **Surface conformance** — each capability's data, projected to a surface,
//!   round-trips equal to the core result (`adapter::project` /
//!   `adapter::recover_core`).
//!
//! The catalog is **honest**: each capability lists only the surfaces that
//! genuinely return its core result today (see the per-entry comments in
//! `build_catalog`).

pub mod adapter;

/// The five projection surfaces a capability can be exposed on.
///
/// Each flag is an independent declaration: a capability is on a surface iff the
/// corresponding field is `true`. Kept as named bools (rather than a bitset) so
/// the catalog reads as a self-documenting matrix and so `serde` round-trips it
/// without a custom impl.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SurfaceSet {
    /// Exposed via the MCP server (`cxpak serve --mcp`).
    pub mcp: bool,
    /// Exposed via the LSP server (`cxpak lsp`, `cxpak/*` custom methods).
    pub lsp: bool,
    /// Exposed as a CLI subcommand / flag.
    pub cli: bool,
    /// Exposed over the HTTP intelligence API (`/v1/*`).
    pub http: bool,
    /// Exposed in a visual dashboard / diagram.
    pub visual: bool,
}

impl SurfaceSet {
    /// Number of surfaces this capability is exposed on.
    pub fn count(&self) -> usize {
        [self.mcp, self.lsp, self.cli, self.http, self.visual]
            .iter()
            .filter(|b| **b)
            .count()
    }
}

/// A unit of analysis cxpak performs, declared independently of its surfaces.
///
/// `output_schema` ties to Task 0.5's versioned schema registry
/// ([`crate::commands::schema::capability_schema`]) for the four schema-backed
/// capabilities (`context`/`graph`/`data`/`review`); capabilities without a
/// registered schema yet carry `output_schema: None` (honest — a schema is
/// added when the contract is pinned, not speculatively).
#[derive(Debug, Clone)]
pub struct Capability {
    /// Stable identifier, also the MCP `op` selector value. Must be unique
    /// across the catalog.
    pub id: &'static str,
    /// Human-facing one-line description (used by the MCP tool schema).
    pub summary: &'static str,
    /// Intent group this capability belongs to — the MCP adapter groups
    /// capabilities sharing an `intent` into a single top-level tool, which is
    /// what keeps the tool count ≤ 8. See [`adapter::mcp_tools`].
    pub intent: Intent,
    /// Names of the inputs this capability accepts (descriptive only — the
    /// catalog does not type-check arguments; surfaces validate their own).
    pub inputs: &'static [&'static str],
    /// Whether Task 0.5's schema registry has a versioned output schema for
    /// this capability id. When `true`, [`Capability::output_schema`] resolves
    /// it from the single source of truth.
    pub has_schema: bool,
    /// Whether this capability is strictly read-only — it only reads and
    /// analyses the index and never mutates the codebase or any external state.
    /// Every cxpak capability is read-only by construction (cxpak is an analysis
    /// tool, not a code mutator); this annotation lets surfaces that advertise
    /// capability metadata mark them safe (e.g. the LSP retrieval methods carry
    /// a matching read-only annotation — [`crate::lsp::methods::method_is_read_only`]).
    /// Introduced with the C1 retrieval capability (ADR-0180).
    pub read_only: bool,
    /// Which of the five surfaces expose this capability today.
    pub projections: SurfaceSet,
}

impl Capability {
    /// Resolve the versioned output schema from Task 0.5's registry, or `None`
    /// if this capability has no pinned schema yet. Never duplicates the schema
    /// JSON — always reads it from `commands::schema` (single source of truth).
    pub fn output_schema(&self) -> Option<serde_json::Value> {
        if !self.has_schema {
            return None;
        }
        crate::commands::schema::capability_schema(self.id).ok()
    }
}

/// Intent groups — the top-level MCP tools. The catalog is grouped by intent so
/// that adding capabilities grows an intent's `op` list, not the tool count.
///
/// There are intentionally **fewer than 8** intents so the budget gate has
/// headroom for B1 (graph-query) and the C3 migration to land without breaching
/// the ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum Intent {
    /// Pack token-budgeted context for a task / file set.
    Context,
    /// Structural graph & dependency analysis.
    Graph,
    /// Data-layer (schema/ORM/migration) analysis.
    Data,
    /// Review-aware diff / change analysis.
    Review,
    /// Health, risk, and architecture reporting.
    Insight,
}

impl Intent {
    /// Stable, deterministic ordering of all intents (also the MCP tool order).
    pub const ALL: &'static [Intent] = &[
        Intent::Context,
        Intent::Graph,
        Intent::Data,
        Intent::Review,
        Intent::Insight,
    ];

    /// The MCP top-level tool name for this intent (`cxpak_<intent>`).
    pub fn tool_name(&self) -> &'static str {
        match self {
            Intent::Context => "cxpak_context",
            Intent::Graph => "cxpak_graph",
            Intent::Data => "cxpak_data",
            Intent::Review => "cxpak_review",
            Intent::Insight => "cxpak_insight",
        }
    }
}

/// The capability catalog: every capability cxpak exposes, each declaring its
/// surfaces. Post-C3 (ADR-0182) this enumerates all 26 former MCP tools as
/// `op`-selectable capabilities plus the schema-anchored `context`/`graph`/
/// `data`/`review` ids from Task 0.5.
///
/// Built once via `OnceLock` and returned as `&'static`, so catalog order is
/// fixed and there is no per-call allocation. The slice order is the canonical
/// deterministic ordering used by every projection.
pub fn catalog() -> &'static [Capability] {
    use std::sync::OnceLock;
    static CATALOG: OnceLock<Vec<Capability>> = OnceLock::new();
    CATALOG.get_or_init(build_catalog).as_slice()
}

/// Convenience constructor for a read-only [`Capability`] — every cxpak
/// capability is read-only by construction, so the flag is not a per-call
/// parameter here (the `every_capability_is_read_only` invariant test pins it).
fn cap(
    id: &'static str,
    summary: &'static str,
    intent: Intent,
    inputs: &'static [&'static str],
    has_schema: bool,
    projections: SurfaceSet,
) -> Capability {
    Capability {
        id,
        summary,
        intent,
        inputs,
        has_schema,
        read_only: true,
        projections,
    }
}

/// Compact [`SurfaceSet`] literal: `(mcp, lsp, cli, http, visual)`.
const fn s(mcp: bool, lsp: bool, cli: bool, http: bool, visual: bool) -> SurfaceSet {
    SurfaceSet {
        mcp,
        lsp,
        cli,
        http,
        visual,
    }
}

fn build_catalog() -> Vec<Capability> {
    // C3 (ADR-0182) consolidated the 26 hand-rolled `serve.rs` MCP tools into
    // this catalog: every legacy MCP capability now rides as an `op` under one
    // of the five `cxpak_<intent>` tools emitted by `adapter::mcp_tools`, and
    // the live `serve.rs` `tools/list` is that adapter projection (≤8). Each
    // capability id below is exactly the MCP `op` selector value.
    //
    // Surface declarations are honest snapshots of where each capability is
    // reachable today, cross-referenced against the live MCP dispatch
    // (`serve.rs` `dispatch_capability_op`), the `/v1/*` routes, the `cxpak/*`
    // LSP methods (`lsp::methods`), and the CLI subcommands. `http` means an
    // `/v1/*` route returns the capability's core; `visual` means a dashboard
    // renders it. They are NOT aspirational.
    vec![
        // ---- Intent::Context — pack token-budgeted context -----------------
        // `context` = auto_context, MCP only (no `/v1/context`, no CLI/LSP).
        cap(
            "context",
            "Token-budgeted context bundle for a task (auto_context).",
            Intent::Context,
            &["task", "tokens", "focus", "mode", "cost_model"],
            true,
            s(true, false, false, false, false),
        ),
        // `retrieval` (C1, ADR-0180) — one core over four surfaces: CLI
        // `cxpak search`, HTTP `/v1/retrieval`, LSP `cxpak/retrieval`, MCP op.
        cap(
            "retrieval",
            "Iterative retrieval over cxpak's own index: search, references, expand.",
            Intent::Context,
            &["retrieval_op", "query", "symbol", "seeds", "depth", "limit"],
            false,
            // CLI `cxpak search`, HTTP `/v1/retrieval`, LSP `cxpak/retrieval`, MCP.
            s(true, true, true, true, false),
        ),
        // `search` — the legacy regex content search (`find_content_matches`);
        // preserved verbatim from the removed `cxpak_search` tool. Also on LSP
        // (`cxpak/search`). Distinct from `retrieval` (the newer iterative core).
        cap(
            "search",
            "Regex content search over indexed files with surrounding context.",
            Intent::Context,
            &["pattern", "limit", "focus", "context_lines"],
            false,
            s(true, true, false, false, false),
        ),
        // `overview` — repo/language summary; MCP + LSP (`cxpak/overview`).
        cap(
            "overview",
            "Structured overview of the codebase (files, tokens, languages).",
            Intent::Context,
            &["tokens", "focus"],
            false,
            s(true, true, false, false, false),
        ),
        // `stats` — index statistics; MCP only.
        cap(
            "stats",
            "Index statistics: file count, tokens, per-language breakdown.",
            Intent::Context,
            &["focus"],
            false,
            s(true, false, false, false, false),
        ),
        // `context_for_task` — relevance ranking of files for a task; MCP only.
        cap(
            "context_for_task",
            "Score and rank codebase files by relevance to a task description.",
            Intent::Context,
            &["task", "limit", "focus"],
            false,
            s(true, false, false, false, false),
        ),
        // `pack_context` — pack explicit files into a budgeted bundle; MCP only.
        cap(
            "pack_context",
            "Pack selected files into a token-budgeted bundle with dependencies.",
            Intent::Context,
            &[
                "files",
                "tokens",
                "include_dependencies",
                "include_tests",
                "focus",
            ],
            false,
            s(true, false, false, false, false),
        ),
        // `briefing` — compact orientation manifest; MCP + HTTP (`/v1/briefing`).
        cap(
            "briefing",
            "Compact briefing: file manifest, health, risks, architecture (no code).",
            Intent::Context,
            &["task", "tokens", "focus", "cost_model"],
            false,
            s(true, false, false, true, false),
        ),
        // ---- Intent::Graph — query the dependency graph --------------------
        // `graph` (B1, ADR-0176; `nodes` + `unknown_seeds` added by ADR-0202)
        // — nodes/node/neighbors/path/subgraph over one core; CLI `cxpak
        // graph`, HTTP `/v1/graph`, LSP `cxpak/graph`, MCP op. Its
        // neighbors/path/subgraph output carries per-edge `edge_type` +
        // `confidence` (`inferred`) — the A3 (ADR-0175) edge-confidence surface.
        cap(
            "graph",
            "Query the typed dependency graph: nodes (enumerate all ids), \
             node, neighbors, path, subgraph (unknown seeds reported in \
             `unknown_seeds`, never echoed as nodes).",
            Intent::Graph,
            &[
                "graph_op",
                "id",
                "from",
                "to",
                "direction",
                "seeds",
                "depth",
            ],
            true,
            s(true, true, true, true, false),
        ),
        // `trace` — locate a symbol and report match counts; MCP + LSP.
        cap(
            "trace",
            "Trace a symbol through the codebase dependency graph.",
            Intent::Graph,
            &["target", "tokens", "focus"],
            false,
            s(true, true, false, false, false),
        ),
        // `blast_radius` — impact of changing files; MCP + LSP (`cxpak/blastRadius`).
        cap(
            "blast_radius",
            "Impact of changing files: dependents, tests, schema, with risk scores.",
            Intent::Graph,
            &["files", "depth", "focus"],
            false,
            s(true, true, false, false, false),
        ),
        // `call_graph` — cross-file call edges; MCP + LSP + HTTP (`/v1/call_graph`).
        cap(
            "call_graph",
            "Cross-file call graph with per-edge confidence (exact vs approximate).",
            Intent::Graph,
            &["target", "depth", "focus", "workspace"],
            false,
            s(true, true, false, true, false),
        ),
        // `dead_code` — unreferenced symbols; MCP + LSP + HTTP (`/v1/dead_code`).
        cap(
            "dead_code",
            "Dead symbols (zero callers, non-entry, untested) by liveness score.",
            Intent::Graph,
            &["focus", "limit", "workspace"],
            false,
            s(true, true, false, true, false),
        ),
        // `api_surface` — public symbols/routes/services; MCP + LSP (`cxpak/apiSurface`).
        cap(
            "api_surface",
            "Public API surface: symbols, HTTP routes, gRPC services, GraphQL types.",
            Intent::Graph,
            &["focus", "include", "tokens"],
            false,
            s(true, true, false, false, false),
        ),
        // `data_flow` — source→sink value flow; MCP + LSP + HTTP (`/v1/data_flow`).
        cap(
            "data_flow",
            "Trace how a value flows from source to sink through static call paths.",
            Intent::Graph,
            &["symbol", "sink", "depth", "focus"],
            false,
            s(true, true, false, true, false),
        ),
        // `cross_lang` — cross-language boundaries; MCP + HTTP (`/v1/cross_lang`).
        cap(
            "cross_lang",
            "Cross-language boundaries: HTTP, FFI, gRPC, GraphQL, shared DB, exec.",
            Intent::Graph,
            &["file", "focus"],
            false,
            s(true, false, false, true, false),
        ),
        // `predict` — change-impact prediction; MCP + LSP + HTTP (`/v1/predict`).
        cap(
            "predict",
            "Predict change impact (structural, historical, call-based) with tests.",
            Intent::Graph,
            &["files", "depth", "focus"],
            false,
            s(true, true, false, true, false),
        ),
        // ---- Intent::Data — inspect the data layer -------------------------
        // `data` (B1 M2 / C3) — the `SchemaIndex` (tables/views/ORM/migrations)
        // is now surfaced as a retrievable result via the MCP `cxpak_data` op
        // (`dispatch_capability_op` "data"). Its 0.5 schema is anchored. Other
        // surfaces do not yet return the SchemaIndex, so they stay false.
        cap(
            "data",
            "Data-layer index: tables, views, ORM models, migrations.",
            Intent::Data,
            &["focus"],
            true,
            s(true, false, false, false, false),
        ),
        // ---- Intent::Review — analyze changes for review -------------------
        // `review` = context_diff (`ContextDelta`, 0.5 review schema); MCP only.
        cap(
            "review",
            "Review-aware diff delta (changed files, symbols, edges).",
            Intent::Review,
            &["since", "focus"],
            true,
            s(true, false, false, false, false),
        ),
        // `diff` — changes with dependency context (+ optional review bundle);
        // MCP + LSP (`cxpak/diff`).
        cap(
            "diff",
            "Show changes with dependency context; optional change-impact review.",
            Intent::Review,
            &["git_ref", "tokens", "focus", "review"],
            false,
            s(true, true, false, false, false),
        ),
        // `verify` — convention deviations on changed lines; MCP only.
        cap(
            "verify",
            "Verify code changes against observed conventions (changed lines only).",
            Intent::Review,
            &["ref", "focus"],
            false,
            s(true, false, false, false, false),
        ),
        // ---- Intent::Insight — health, risk, architecture ------------------
        cap(
            "health",
            "Composite codebase health score across six dimensions.",
            Intent::Insight,
            &["focus"],
            false,
            s(true, true, false, true, true),
        ),
        cap(
            "risks",
            "Per-file risk ranking (churn × blast radius × coverage).",
            Intent::Insight,
            &["limit", "focus"],
            false,
            s(true, false, false, true, true),
        ),
        cap(
            "architecture",
            "Module map with circular-dependency detection.",
            Intent::Insight,
            &["focus", "workspace"],
            false,
            s(true, false, false, true, true),
        ),
        // `conventions` — full convention profile; MCP + LSP + HTTP.
        cap(
            "conventions",
            "Full convention profile: detected patterns with counts and strength.",
            Intent::Insight,
            &["category", "strength", "focus"],
            false,
            s(true, true, false, true, false),
        ),
        // `security_surface` — endpoints/secrets/injection; MCP + LSP + HTTP.
        cap(
            "security_surface",
            "Security surface: unprotected endpoints, secrets, injection, exposure.",
            Intent::Insight,
            &["focus"],
            false,
            s(true, true, false, true, false),
        ),
        // `drift` — architecture drift vs baseline/snapshots; MCP + LSP + HTTP.
        cap(
            "drift",
            "Architecture drift vs baseline and historical snapshots.",
            Intent::Insight,
            &["save_baseline", "focus"],
            false,
            s(true, true, false, true, false),
        ),
        // `visual` — interactive diagrams; MCP + CLI (`cxpak visual`) + visual.
        cap(
            "visual",
            "Interactive visual diagram (dashboard, architecture, risk, flow, ...).",
            Intent::Insight,
            &["type", "format", "focus", "symbol", "files"],
            false,
            s(true, false, true, false, true),
        ),
        // `onboard` — guided reading map; MCP + CLI (`cxpak onboard`).
        cap(
            "onboard",
            "Guided onboarding map: phased reading plan with reading time.",
            Intent::Insight,
            &["focus", "format"],
            false,
            s(true, false, true, false, false),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_stable_and_unique() {
        let c = catalog();
        assert!(!c.is_empty());
        // Same backing slice every call (OnceLock), so pointers are stable.
        assert!(std::ptr::eq(c.as_ptr(), catalog().as_ptr()));
        // Ids are unique.
        let mut ids: Vec<&str> = c.iter().map(|cap| cap.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "capability ids must be unique");
    }

    #[test]
    fn schema_backed_capabilities_resolve_a_schema() {
        for cap in catalog() {
            let schema = cap.output_schema();
            if cap.has_schema {
                assert!(
                    schema.is_some(),
                    "capability `{}` claims a schema but registry returned none",
                    cap.id
                );
                // It must be the SAME document Task 0.5 serves (single source).
                assert_eq!(
                    schema.unwrap(),
                    crate::commands::schema::capability_schema(cap.id).unwrap()
                );
            } else {
                assert!(
                    schema.is_none(),
                    "capability `{}` has no schema flag but registry returned one",
                    cap.id
                );
            }
        }
    }

    #[test]
    fn surface_set_count_is_correct() {
        let s = SurfaceSet {
            mcp: true,
            lsp: false,
            cli: true,
            http: true,
            visual: false,
        };
        assert_eq!(s.count(), 3);
        // A capability may legitimately be on ZERO surfaces today if it exists
        // only to anchor its Task 0.5 schema until a surface is wired (e.g.
        // `graph`/`data`). Such a capability MUST be schema-backed — an
        // unsurfaced capability with no schema would be entirely unreachable
        // and has no business in the catalog.
        for cap in catalog() {
            if cap.projections.count() == 0 {
                assert!(
                    cap.has_schema,
                    "{} is on no surface and has no schema — it is unreachable; \
                     drop it from the catalog or wire a surface",
                    cap.id
                );
            }
        }
    }

    #[test]
    fn every_capability_is_read_only() {
        // Invariant: cxpak is a read-only analysis tool — no capability mutates
        // the codebase. The `read_only` annotation (added with C1, ADR-0180)
        // must therefore be `true` for every catalog entry.
        for cap in catalog() {
            assert!(
                cap.read_only,
                "capability `{}` is not marked read_only; cxpak capabilities \
                 must be read-only",
                cap.id
            );
        }
    }

    #[test]
    fn retrieval_capability_rides_context_on_four_surfaces() {
        // C1: retrieval is a Context op reachable on MCP/LSP/CLI/HTTP (not
        // visual), schema-less, and read-only.
        let r = catalog()
            .iter()
            .find(|c| c.id == "retrieval")
            .expect("retrieval capability present");
        assert_eq!(r.intent, Intent::Context);
        assert!(r.read_only);
        assert!(!r.has_schema);
        assert!(r.projections.mcp);
        assert!(r.projections.lsp);
        assert!(r.projections.cli);
        assert!(r.projections.http);
        assert!(!r.projections.visual);
        assert_eq!(r.projections.count(), 4);
    }

    #[test]
    fn retrieval_shares_context_intent_tool_without_growing_count() {
        // Adding retrieval must NOT add a top-level MCP tool: it rides under
        // the existing `cxpak_context` intent-tool as an `op`.
        let tools = super::adapter::mcp_tools(catalog());
        assert!(tools.len() <= 8);
        let context_tool = tools
            .iter()
            .find(|t| t.name == "cxpak_context")
            .expect("cxpak_context intent-tool present");
        assert!(
            context_tool.ops.iter().any(|op| op == "retrieval"),
            "retrieval must be selectable as an op under cxpak_context; ops={:?}",
            context_tool.ops
        );
    }

    #[test]
    fn intent_all_is_complete_and_ordered() {
        // Every catalog intent appears in Intent::ALL.
        for cap in catalog() {
            assert!(Intent::ALL.contains(&cap.intent));
        }
        // ALL is strictly ascending (deterministic, no duplicates).
        for w in Intent::ALL.windows(2) {
            assert!(w[0] < w[1], "Intent::ALL must be strictly ordered");
        }
    }
}
