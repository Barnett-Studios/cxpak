//! Budget gate: the capability catalog's MCP projection must never exceed
//! **8 top-level MCP tools** (the ≤8 guardrail, cxpak 3.0.0 Phase 0).
//!
//! This counts the NEW capability catalog → MCP adapter (intent-tools grouped
//! from `catalog()`), NOT the legacy 26-tool array hand-declared in
//! `src/commands/serve.rs`. The 26→8 consolidation of the legacy handler is a
//! later task (C3); this gate locks the ceiling in *now* so every capability
//! added to the catalog later rides as an op/param under an existing
//! intent-tool rather than as a new top-level MCP tool.

use cxpak::capability::adapter::mcp_tools;
use cxpak::capability::catalog;

#[test]
fn mcp_projection_yields_at_most_eight_tools() {
    let tools = mcp_tools(catalog());
    assert!(
        tools.len() <= 8,
        "MCP adapter over catalog() must yield ≤ 8 top-level intent-tools \
         (the ≤8 guardrail); got {}: {:?}",
        tools.len(),
        tools.iter().map(|t| &t.name).collect::<Vec<_>>()
    );
}

#[test]
fn mcp_tool_names_are_unique_and_deterministic() {
    // Determinism: two builds of the projection must produce byte-identical
    // tool lists (no HashMap iteration in the output path).
    let a = mcp_tools(catalog());
    let b = mcp_tools(catalog());
    let names_a: Vec<&str> = a.iter().map(|t| t.name.as_str()).collect();
    let names_b: Vec<&str> = b.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names_a, names_b, "MCP tool ordering must be deterministic");

    let mut sorted = names_a.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        names_a.len(),
        "MCP intent-tool names must be unique; got {names_a:?}"
    );
}

#[test]
fn every_mcp_capability_is_reachable_under_some_tool() {
    // Every capability declaring the MCP surface must be selectable via exactly
    // one intent-tool's `op` parameter — otherwise the grouping silently drops
    // a capability and the ≤8 count is meaningless.
    let tools = mcp_tools(catalog());
    for cap in catalog().iter().filter(|c| c.projections.mcp) {
        let hosting: Vec<&str> = tools
            .iter()
            .filter(|t| t.ops.iter().any(|op| op == cap.id))
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(
            hosting.len(),
            1,
            "MCP capability `{}` must be hosted by exactly one intent-tool, \
             found in {hosting:?}",
            cap.id
        );
    }
}
