//! Per-surface projection framework over the capability catalog.
//!
//! A projection *reshapes* a capability's single core result for a particular
//! surface; it never re-derives the result (ADR-0153 single-source invariant).
//! Concretely, [`project`] takes the core `serde_json::Value` and returns the
//! surface-shaped value, and [`recover_core`] is its inverse — extracting the
//! core back out so the conformance gate can assert bit-exact round-trip
//! equality (`tests/surface_conformance.rs`).
//!
//! Most surfaces today carry the core result verbatim (the projection is the
//! identity reshape) — exactly what the cross-channel tests already prove for
//! `health`/`risks`/`architecture` (SPA, `/v1`, MCP and LSP all serialise the
//! same core struct). The MCP surface is the one with real reshaping: its
//! results are wrapped in an intent-tool envelope carrying the selected `op`,
//! because the MCP adapter groups the catalog into **≤ 8** intent-tools.

use super::{catalog, Capability, Intent};
use serde_json::{json, Value};

/// The five projection surfaces, in deterministic order. Used by the
/// conformance harness to iterate every capability × surface cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Mcp,
    Lsp,
    Cli,
    Http,
    Visual,
}

/// All surfaces in a fixed order — the canonical iteration order for the
/// conformance matrix. Deterministic (a literal slice, never HashMap order).
pub const ALL_SURFACES: &[Surface] = &[
    Surface::Mcp,
    Surface::Lsp,
    Surface::Cli,
    Surface::Http,
    Surface::Visual,
];

/// JSON key under which the MCP envelope nests the core result.
const MCP_RESULT_KEY: &str = "result";
/// JSON key under which the MCP envelope records the selected capability op.
const MCP_OP_KEY: &str = "op";

/// Project a capability's core result onto a surface.
///
/// * MCP — wrap in `{ "op": <id>, "result": <core> }`. This mirrors how the
///   grouped intent-tool dispatches: the top-level tool selects a capability by
///   `op`, and the core result is returned under `result`. The wrap is a pure
///   reshape — the core is embedded unchanged.
/// * LSP / CLI / HTTP / Visual — the core result is carried verbatim (identity
///   reshape), matching the established cross-channel behaviour where each
///   surface serialises the same core struct.
///
/// A projection MUST depend only on `core` (never recompute), so the
/// single-source invariant holds. `cap` is accepted so future surfaces can key
/// reshaping off capability metadata without changing the signature.
pub fn project(cap: &Capability, surface: Surface, core: &Value) -> Value {
    match surface {
        Surface::Mcp => json!({
            MCP_OP_KEY: cap.id,
            MCP_RESULT_KEY: core.clone(),
        }),
        Surface::Lsp | Surface::Cli | Surface::Http | Surface::Visual => core.clone(),
    }
}

/// Inverse of [`project`]: recover the core result from a surface projection so
/// conformance can compare it to the original core.
///
/// For MCP this unwraps the envelope; for the identity surfaces it returns the
/// value unchanged.
pub fn recover_core(_cap: &Capability, surface: Surface, projected: &Value) -> Value {
    match surface {
        Surface::Mcp => projected
            .get(MCP_RESULT_KEY)
            .cloned()
            .unwrap_or(Value::Null),
        Surface::Lsp | Surface::Cli | Surface::Http | Surface::Visual => projected.clone(),
    }
}

/// A top-level MCP intent-tool: one tool fronting one or more capabilities,
/// selected by the `op` parameter. The catalog's MCP capabilities are grouped
/// into these so the total tool count stays **≤ 8**.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct McpTool {
    /// Top-level tool name (`cxpak_<intent>`).
    pub name: String,
    /// One-line tool description.
    pub description: String,
    /// The capability ids selectable via this tool's `op` parameter, in catalog
    /// order. Deterministic.
    pub ops: Vec<String>,
}

/// Build the MCP projection of the catalog: capabilities declaring the MCP
/// surface, grouped by [`Intent`] into ≤ 8 top-level intent-tools.
///
/// Ordering is deterministic: intents iterate in [`Intent::ALL`] order, and
/// within each intent the ops follow catalog order. No HashMap iteration is
/// involved, so the output is byte-stable across builds (pinned by
/// `tests/mcp_tool_budget.rs`).
///
/// The ≤8 ceiling is a structural consequence of grouping by intent: there are
/// only [`Intent::ALL`]`.len()` intents, and that is `<= 8` by construction
/// (asserted in tests). New capabilities join an existing intent's `op` list
/// rather than adding a tool.
pub fn mcp_tools(caps: &[Capability]) -> Vec<McpTool> {
    let mut tools = Vec::new();
    for &intent in Intent::ALL {
        let ops: Vec<String> = caps
            .iter()
            .filter(|c| c.projections.mcp && c.intent == intent)
            .map(|c| c.id.to_string())
            .collect();
        // Only emit a tool for intents that actually front an MCP capability —
        // an intent with no MCP-exposed capability would be a dead tool.
        if ops.is_empty() {
            continue;
        }
        tools.push(McpTool {
            name: intent.tool_name().to_string(),
            description: describe_intent(intent, &ops),
            ops,
        });
    }
    tools
}

/// Human-facing description for an intent-tool, listing its selectable ops.
fn describe_intent(intent: Intent, ops: &[String]) -> String {
    let verb = match intent {
        Intent::Context => "Pack token-budgeted context",
        Intent::Graph => "Query the dependency graph",
        Intent::Data => "Inspect the data layer",
        Intent::Review => "Analyze changes for review",
        Intent::Insight => "Report health, risk, and architecture insights",
    };
    format!(
        "{verb}. Select a capability via `op` ∈ {{{}}}.",
        ops.join(", ")
    )
}

/// Convenience: the MCP tools for the live catalog.
pub fn mcp_catalog_tools() -> Vec<McpTool> {
    mcp_tools(catalog())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::SurfaceSet;

    fn cap_for(id: &str) -> &'static Capability {
        catalog().iter().find(|c| c.id == id).expect("present")
    }

    #[test]
    fn intent_count_is_within_budget() {
        // The structural reason the ≤8 gate holds: there are at most 8 intents.
        assert!(
            Intent::ALL.len() <= 8,
            "intent count {} exceeds the ≤8 MCP tool ceiling",
            Intent::ALL.len()
        );
    }

    #[test]
    fn mcp_envelope_round_trips() {
        let cap = cap_for("health");
        let core = json!({"composite": 7.5, "cycles": 9.0});
        let projected = project(cap, Surface::Mcp, &core);
        assert_eq!(projected[MCP_OP_KEY], json!("health"));
        assert_eq!(recover_core(cap, Surface::Mcp, &projected), core);
    }

    #[test]
    fn identity_surfaces_carry_core_verbatim() {
        let cap = cap_for("graph");
        let core = json!({"edges": {"a": ["b"]}});
        for s in [Surface::Lsp, Surface::Cli, Surface::Http, Surface::Visual] {
            let projected = project(cap, s, &core);
            assert_eq!(projected, core, "{s:?} must carry core verbatim");
            assert_eq!(recover_core(cap, s, &projected), core);
        }
    }

    #[test]
    fn grouping_keeps_tools_within_budget_and_ordered() {
        let tools = mcp_tools(catalog());
        assert!(tools.len() <= 8);
        // Tools follow Intent::ALL order.
        let expected: Vec<&str> = Intent::ALL
            .iter()
            .filter(|i| {
                catalog()
                    .iter()
                    .any(|c| c.projections.mcp && c.intent == **i)
            })
            .map(|i| i.tool_name())
            .collect();
        let got: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn intent_with_no_mcp_capability_emits_no_tool() {
        // A synthetic catalog where Context's only capability is MCP-off must
        // not emit a cxpak_context tool.
        let caps = vec![Capability {
            id: "context",
            summary: "x",
            intent: Intent::Context,
            inputs: &[],
            has_schema: false,
            projections: SurfaceSet {
                mcp: false,
                lsp: false,
                cli: true,
                http: false,
                visual: false,
            },
        }];
        let tools = mcp_tools(&caps);
        assert!(tools.iter().all(|t| t.name != "cxpak_context"));
        assert!(tools.is_empty());
    }
}
