//! `cxpak graph` — CLI surface for the deterministic graph-query capability
//! (cxpak 3.0.0 Task B1).
//!
//! Builds the dependency graph for `path`, then delegates to the single core
//! [`crate::intelligence::graph_query::execute`]. The result is the same
//! byte-deterministic JSON every other surface (MCP `cxpak_graph`, LSP
//! `cxpak/graph`, HTTP `/v1/graph`) returns — this command only adapts CLI args
//! into the core's `params` object and prints the JSON; it never re-derives.

use crate::budget::counter::TokenCounter;
use crate::index::CodebaseIndex;
use crate::intelligence::graph_query;
use crate::scanner::Scanner;
use serde_json::{json, Map, Value};
use std::path::Path;

#[allow(clippy::too_many_arguments)]
pub fn run(
    path: &Path,
    op: &str,
    id: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    direction: &str,
    seeds: &[String],
    depth: usize,
    workspace: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let counter = TokenCounter::new();
    let scanner = Scanner::new(path)?;
    let files = scanner.scan_workspace(workspace)?;
    if files.is_empty() {
        return Err("no source files found".into());
    }
    let (parse_results, content_map) =
        crate::cache::parse::parse_with_cache(&files, path, &counter, false);
    let index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);

    // Build the core's params object from whichever CLI args were supplied;
    // `execute` validates that the chosen `op` has its required parameters.
    let mut params = Map::new();
    if let Some(v) = id {
        params.insert("id".to_string(), json!(v));
    }
    if let Some(v) = from {
        params.insert("from".to_string(), json!(v));
    }
    if let Some(v) = to {
        params.insert("to".to_string(), json!(v));
    }
    params.insert("direction".to_string(), json!(direction));
    if !seeds.is_empty() {
        params.insert("seeds".to_string(), json!(seeds));
    }
    params.insert("depth".to_string(), json!(depth));

    let result = graph_query::execute(&index.graph, op, &Value::Object(params))
        .map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
