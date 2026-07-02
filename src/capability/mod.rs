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
//! This module is a **parallel framework**, deliberately built ALONGSIDE the
//! legacy 26-tool MCP handler in `src/commands/serve.rs`. Task 0.6 does NOT
//! migrate, modify, or delete that handler — the 26→8 consolidation is a later
//! task (C3). What 0.6 locks in are the two CI gates every future capability
//! must pass:
//!
//! * **MCP ≤ 8 tools** — [`adapter::mcp_tools`] groups capabilities into at most
//!   eight intent-tools; a new capability rides as an `op` param under an
//!   existing tool, never as a new top-level MCP tool.
//! * **Surface conformance** — each capability's data, projected to a surface,
//!   round-trips equal to the core result (`adapter::project` /
//!   `adapter::recover_core`).
//!
//! The catalog is **honest**: it lists only capabilities that genuinely exist
//! and are reachable today. It does not attempt to enumerate all 26 legacy MCP
//! ops — that is C3's job.

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

/// The initial capability catalog: capabilities that genuinely exist and are
/// reachable today, each declaring its surfaces.
///
/// Anchored to Task 0.5's schema ids (`context`/`graph`/`data`/`review`) plus
/// the core intelligence capabilities already cross-channel today
/// (`health`/`risks`/`architecture` — see `tests/cross_channel_consistency.rs`,
/// which proves they are SPA + v1 + MCP + LSP consistent). It does NOT
/// enumerate all 26 legacy MCP ops; that is C3.
///
/// Built once via `OnceLock` and returned as `&'static`, so catalog order is
/// fixed and there is no per-call allocation. The slice order is the canonical
/// deterministic ordering used by every projection.
pub fn catalog() -> &'static [Capability] {
    use std::sync::OnceLock;
    static CATALOG: OnceLock<Vec<Capability>> = OnceLock::new();
    CATALOG.get_or_init(build_catalog).as_slice()
}

fn build_catalog() -> Vec<Capability> {
    // Surface declarations below are honest snapshots of where each capability
    // is reachable today (cross-referenced against serve.rs MCP tools, the
    // `/v1/*` routes, the `cxpak/*` LSP methods, the CLI subcommands, and the
    // SPA dashboard). They are NOT aspirational.
    vec![
        Capability {
            id: "context",
            summary: "Token-budgeted context bundle for a task (auto_context).",
            read_only: true,
            intent: Intent::Context,
            inputs: &["task", "budget", "files"],
            has_schema: true,
            // Reachable today ONLY via the MCP `cxpak_auto_context` tool. There
            // is no `cxpak context` CLI subcommand, no `/v1/context` route, no
            // `cxpak/context` LSP method, and the SPA does not render the
            // context bundle — so every other bit is honestly false.
            projections: SurfaceSet {
                mcp: true,
                lsp: false,
                cli: false,
                http: false,
                visual: false,
            },
        },
        Capability {
            id: "retrieval",
            summary: "Iterative retrieval over cxpak's own index: search, references, expand.",
            read_only: true,
            // Retrieval rides under the Context intent (its natural home — it
            // packs the raw material auto_context assembles). The single core
            // `intelligence::retrieval::execute` (op ∈ search|references|expand)
            // is projected to all four surfaces below — no re-derivation
            // (ADR-0180; mirrors B1's `graph` capability):
            //   * CLI  — `cxpak search <query> [--op ...]` (commands::search).
            //   * HTTP — `POST /v1/retrieval` (serve.rs `v1_retrieval_handler`).
            //   * LSP  — `cxpak/retrieval` custom method (lsp::methods), carrying
            //     a read-only annotation (`method_is_read_only`).
            //   * MCP  — selectable as `op=retrieval` under the `cxpak_context`
            //     intent-tool emitted by the catalog adapter, keeping the budget
            //     ≤8. The live serve.rs MCP server migrates onto the adapter in
            //     C3 (26→8), out of scope here; the legacy regex `cxpak_search`
            //     / `cxpak/search` / `/search` stay untouched for C3 to
            //     reconcile.
            // No schema yet (the retrieval contract is pinned by ADR-0180 + the
            // determinism gate, not a 0.5 JSON schema); `visual` stays false.
            intent: Intent::Context,
            inputs: &["op", "query", "symbol", "seeds", "depth", "limit"],
            has_schema: false,
            projections: SurfaceSet {
                mcp: true,
                lsp: true,
                cli: true,
                http: true,
                visual: false,
            },
        },
        Capability {
            id: "graph",
            summary: "Query the typed dependency graph: node, neighbors, path, subgraph.",
            read_only: true,
            intent: Intent::Graph,
            inputs: &["op", "id", "from", "to", "direction", "seeds", "depth"],
            has_schema: true,
            // B1 (graph-query) surfaces the typed graph as a retrievable result
            // through the single core `intelligence::graph_query::execute`
            // (node/neighbors/path/subgraph). All four surfaces below project
            // that one core result — no re-derivation:
            //   * CLI  — `cxpak graph <op> ...` (commands::graph).
            //   * HTTP — `POST /v1/graph` (serve.rs `v1_graph_handler`).
            //   * LSP  — `cxpak/graph` custom method (lsp::methods).
            //   * MCP  — the `cxpak_graph` intent-tool, emitted by the catalog
            //     adapter (`adapter::mcp_tools`), keeping the budget ≤8. The
            //     live `serve.rs` MCP server is migrated onto the adapter in C3
            //     (the 26→8 consolidation), which is out of scope here.
            // `visual` stays false: the architecture/flow views draw the graph,
            // they do not return the graph-query JSON contract.
            projections: SurfaceSet {
                mcp: true,
                lsp: true,
                cli: true,
                http: true,
                visual: false,
            },
        },
        Capability {
            id: "data",
            summary: "Data-layer index: tables, views, ORM models, migrations.",
            read_only: true,
            intent: Intent::Data,
            inputs: &["focus"],
            has_schema: true,
            // The `SchemaIndex` is indexed internally (consumed for query
            // expansion / schema-aware edges) but is NOT yet surfaced as a
            // retrievable result: there is no `/v1/data` route (`/v1/data_flow`
            // is the distinct cross-language data-flow capability), no MCP tool
            // emits the SchemaIndex, and `cxpak schema data` prints the schema
            // CONTRACT, not the indexed tables. Kept to anchor its 0.5 schema;
            // wired to a surface in A2/C3.
            projections: SurfaceSet {
                mcp: false,
                lsp: false,
                cli: false,
                http: false,
                visual: false,
            },
        },
        Capability {
            id: "review",
            summary: "Review-aware diff delta (changed files, symbols, edges).",
            read_only: true,
            intent: Intent::Review,
            inputs: &["base", "head"],
            has_schema: true,
            // The `ContextDelta` (Task 0.5 `review` schema) is returned today
            // ONLY by the MCP `cxpak_context_diff` tool (= `compute_diff`). The
            // CLI `cxpak diff --review` emits a markdown change-impact bundle
            // (not ContextDelta JSON), `cxpak/diff` (LSP) returns raw git
            // changes (not ContextDelta), there is no `/v1/` ContextDelta route,
            // and the SPA inits its diff data to null. So MCP only.
            projections: SurfaceSet {
                mcp: true,
                lsp: false,
                cli: false,
                http: false,
                visual: false,
            },
        },
        Capability {
            id: "health",
            summary: "Composite codebase health score across six dimensions.",
            read_only: true,
            intent: Intent::Insight,
            inputs: &[],
            has_schema: false,
            projections: SurfaceSet {
                mcp: true,
                lsp: true,
                cli: false,
                http: true,
                visual: true,
            },
        },
        Capability {
            id: "risks",
            summary: "Per-file risk ranking (churn × blast radius × coverage).",
            read_only: true,
            intent: Intent::Insight,
            inputs: &[],
            has_schema: false,
            projections: SurfaceSet {
                mcp: true,
                lsp: false,
                cli: false,
                http: true,
                visual: true,
            },
        },
        Capability {
            id: "architecture",
            summary: "Module map with circular-dependency detection.",
            read_only: true,
            intent: Intent::Insight,
            inputs: &["module_depth"],
            has_schema: false,
            projections: SurfaceSet {
                mcp: true,
                lsp: false,
                cli: false,
                http: true,
                visual: true,
            },
        },
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
