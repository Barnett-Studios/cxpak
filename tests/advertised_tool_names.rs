//! Every `cxpak_*` name cxpak puts in front of a caller must be one the MCP
//! server actually advertises, and every `(op: "...")` beside it must be an op
//! that tool hosts.
//!
//! cxpak#61: a `no prior snapshot` recommendation told callers to
//! `Call cxpak_auto_context` — a name `tools/list` has not offered since the
//! 3.0 intent-tool consolidation (ADR-0182). It still *routes*, via the
//! deprecated alias table in `serve.rs`, so a permissive client never noticed;
//! a client that validates tool names against the advertised set before
//! dispatching — which is what an agent harness does — was told to call
//! something it was never offered.
//!
//! The first fix corrected the two strings the ticket named. That is a
//! hand-written denominator: a third site, the `context_for_task` hint in
//! `serve.rs`, named `cxpak_pack_context` and was reached through an
//! *advertised* entry point, and two `visual`-feature error strings named
//! `cxpak_visual` / `cxpak_onboard`. None were in the ticket.
//!
//! So this test derives the site set from the source instead of listing it:
//! every `cxpak_<name>` token appearing inside a string literal under `src/`,
//! checked against `mcp_catalog_tools()` — the same function `tools/list` is
//! built from, so a future rename moves both sides together.

use std::path::{Path, PathBuf};

/// The two exclusions, both narrow and both load-bearing:
///
/// * `legacy_alias_to_op` — its 26 arms exist *to* name the pre-3.0 tool names
///   and map them onto ops. Naming a non-advertised tool is its whole job.
/// * `mod tests` — test code asserts on the deprecated alias deliberately
///   (`!rec.contains("cxpak_auto_context")` is how the fix is proved).
///
/// Each is matched by a marker that is part of the item's own declaration, so
/// renaming `legacy_alias_to_op` does not silently widen the scan — it makes
/// the 26 arms visible to it and the test fails loudly.
const EXCLUDED_ITEM_MARKERS: &[&str] = &["fn legacy_alias_to_op", "mod tests"];

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {dir:?}: {e}"));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Line indices covered by an excluded item, found by brace-matching from its
/// declaration. An unbalanced file would run to EOF, which over-excludes rather
/// than under-excludes — but the non-empty assertion below catches a scan that
/// has excluded everything.
fn excluded_lines(lines: &[&str]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !EXCLUDED_ITEM_MARKERS.iter().any(|m| line.contains(m)) {
            continue;
        }
        let mut depth: i32 = 0;
        let mut opened = false;
        let mut end = lines.len() - 1;
        for (j, l) in lines.iter().enumerate().skip(i) {
            depth += l.matches('{').count() as i32 - l.matches('}').count() as i32;
            opened |= l.contains('{');
            if opened && depth <= 0 {
                end = j;
                break;
            }
        }
        spans.push((i, end));
    }
    spans
}

/// `cxpak_<name>` occurrences inside string literals on this line, each paired
/// with the `op` it names if one follows it.
fn tools_named_in_literals(line: &str) -> Vec<(String, Option<String>)> {
    let mut found = Vec::new();
    let bytes: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != '"' {
            i += 1;
            continue;
        }
        // Walk the literal, honouring backslash escapes so an escaped quote
        // does not end it early — `(op: \"pack_context\")` is written that way.
        let start = i + 1;
        let mut j = start;
        while j < bytes.len() {
            if bytes[j] == '\\' {
                j += 2;
                continue;
            }
            if bytes[j] == '"' {
                break;
            }
            j += 1;
        }
        let literal: String = bytes[start..j.min(bytes.len())].iter().collect();
        for (pos, _) in literal.match_indices("cxpak_") {
            let name: String = literal[pos..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let rest = &literal[pos + name.len()..];
            // The op, when the literal spells it out next to the tool name.
            let op = rest.split_once("op:").and_then(|(before, after)| {
                // Only an op that belongs to *this* mention: no other tool name
                // may sit between them.
                if before.contains("cxpak_") {
                    return None;
                }
                let after = after.trim_start().trim_start_matches('\\');
                after
                    .strip_prefix('"')
                    .map(|v| v.chars().take_while(|c| *c != '"' && *c != '\\').collect())
            });
            found.push((name, op));
        }
        i = j + 1;
    }
    found
}

#[test]
fn every_tool_name_in_a_string_literal_is_advertised() {
    let advertised = cxpak::capability::adapter::mcp_catalog_tools();
    assert!(
        !advertised.is_empty(),
        "no advertised tools — the catalog is not being read, so this proves nothing"
    );

    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&src, &mut files);
    assert!(!files.is_empty(), "no sources scanned under {src:?}");

    let mut checked = 0usize;
    let mut ops_checked = 0usize;
    let mut violations = Vec::new();

    for file in &files {
        let text = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("read {file:?}: {e}"));
        let lines: Vec<&str> = text.lines().collect();
        let excluded = excluded_lines(&lines);

        for (n, line) in lines.iter().enumerate() {
            if excluded.iter().any(|(a, b)| n >= *a && n <= *b) {
                continue;
            }
            for (name, op) in tools_named_in_literals(line) {
                checked += 1;
                let Some(tool) = advertised.iter().find(|t| t.name == name) else {
                    violations.push(format!(
                        "{}:{} names {name:?}, which tools/list does not advertise",
                        file.display(),
                        n + 1
                    ));
                    continue;
                };
                if let Some(op) = op {
                    ops_checked += 1;
                    if !tool.ops.contains(&op) {
                        violations.push(format!(
                            "{}:{} names {name:?} with op {op:?}, which that tool does not host \
                             (hosts: {:?})",
                            file.display(),
                            n + 1,
                            tool.ops
                        ));
                    }
                }
            }
        }
    }

    // Without these the test is green on a scan that found nothing — the exact
    // failure the exclusion brace-matching could produce.
    assert!(
        checked > 0,
        "scanned {} files and found no cxpak_* tool name in any string literal — the scan is \
         reading nothing, or the exclusions swallowed the source",
        files.len()
    );
    assert!(
        ops_checked > 0,
        "found {checked} tool names but not one `(op: \"...\")` beside them — the op half of \
         this guard is not exercised by anything, so it proves nothing"
    );
    assert!(
        violations.is_empty(),
        "{} user-facing string(s) name a tool or op the server does not advertise:\n  {}",
        violations.len(),
        violations.join("\n  ")
    );
}
