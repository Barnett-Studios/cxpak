//! `cxpak search` — CLI surface for the deterministic iterative-retrieval
//! capability (cxpak 3.0.0 Task C1, ADR-0180).
//!
//! Builds the index for `path`, then delegates to the single core
//! [`crate::intelligence::retrieval::execute`]. The result is the same
//! byte-deterministic JSON every other surface (MCP `cxpak_context`
//! `op=retrieval`, LSP `cxpak/retrieval`, HTTP `/v1/retrieval`) returns — this
//! command only adapts CLI args into the core's `params` object and prints the
//! JSON; it never re-derives.

use crate::budget::counter::TokenCounter;
use crate::index::CodebaseIndex;
use crate::intelligence::retrieval;
use crate::scanner::Scanner;
use serde_json::{json, Map, Value};
use std::path::Path;

#[allow(clippy::too_many_arguments)]
pub fn run(
    path: &Path,
    op: &str,
    query: Option<&str>,
    symbol: Option<&str>,
    seeds: &[String],
    depth: usize,
    limit: usize,
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
    // `execute` validates that the chosen `op` has its required parameters. The
    // positional `query` doubles as the `symbol` for `references` when `--symbol`
    // is not given, so `cxpak search --op references helper` works.
    let mut params = Map::new();
    if let Some(q) = query {
        params.insert("query".to_string(), json!(q));
    }
    if let Some(s) = symbol.or(query) {
        params.insert("symbol".to_string(), json!(s));
    }
    if !seeds.is_empty() {
        params.insert("seeds".to_string(), json!(seeds));
    }
    params.insert("depth".to_string(), json!(depth));
    params.insert("limit".to_string(), json!(limit));

    let result = retrieval::execute(&index, op, &Value::Object(params))
        .map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
