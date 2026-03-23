use crate::index::CodebaseIndex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

// ---------------------------------------------------------------------------
// Public data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ContextSnapshot {
    pub file_hashes: HashMap<String, u64>,
    pub symbol_set: HashMap<String, Vec<String>>,
    pub edge_set: HashSet<(String, String, String)>,
}

#[derive(Debug, Serialize)]
pub struct ContextDelta {
    pub modified_files: Vec<FileChange>,
    pub new_files: Vec<String>,
    pub deleted_files: Vec<String>,
    pub new_symbols: Vec<SymbolChange>,
    pub removed_symbols: Vec<SymbolChange>,
    pub graph_changes: Vec<GraphChange>,
    pub recommendation: String,
}

#[derive(Debug, Serialize)]
pub struct FileChange {
    pub path: String,
    pub change: String,
    pub tokens_delta: i64,
}

#[derive(Debug, Serialize)]
pub struct SymbolChange {
    pub path: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Serialize)]
pub struct GraphChange {
    pub change_type: String,
    pub from: String,
    pub to: String,
    pub edge_type: String,
}

// ---------------------------------------------------------------------------
// Hashing helper
// ---------------------------------------------------------------------------

fn hash_content(content: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Build a snapshot of the current index state for later diffing.
pub fn create_snapshot(index: &CodebaseIndex) -> ContextSnapshot {
    let mut file_hashes = HashMap::new();
    let mut symbol_set: HashMap<String, Vec<String>> = HashMap::new();

    for file in &index.files {
        file_hashes.insert(file.relative_path.clone(), hash_content(&file.content));

        let names: Vec<String> = file
            .parse_result
            .as_ref()
            .map(|pr| pr.symbols.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default();
        symbol_set.insert(file.relative_path.clone(), names);
    }

    // Collect all directed edges as (from, to, edge_type_string) tuples.
    let mut edge_set: HashSet<(String, String, String)> = HashSet::new();
    for (from, targets) in &index.graph.edges {
        for typed_edge in targets {
            edge_set.insert((
                from.clone(),
                typed_edge.target.clone(),
                format!("{:?}", typed_edge.edge_type),
            ));
        }
    }

    ContextSnapshot {
        file_hashes,
        symbol_set,
        edge_set,
    }
}

/// Compare a previously taken snapshot against the current index and return
/// a delta describing what changed, plus a human-readable recommendation.
pub fn compute_diff(snapshot: &ContextSnapshot, index: &CodebaseIndex) -> ContextDelta {
    // Build a lookup from path -> token_count for the current index.
    let current_token_counts: HashMap<&str, usize> = index
        .files
        .iter()
        .map(|f| (f.relative_path.as_str(), f.token_count))
        .collect();

    // --- File-level changes ---
    let current_hashes: HashMap<String, u64> = index
        .files
        .iter()
        .map(|f| (f.relative_path.clone(), hash_content(&f.content)))
        .collect();

    let snapshot_paths: HashSet<&String> = snapshot.file_hashes.keys().collect();
    let current_paths: HashSet<&String> = current_hashes.keys().collect();

    let mut modified_files: Vec<FileChange> = Vec::new();
    for path in snapshot_paths.intersection(&current_paths) {
        let old_hash = snapshot.file_hashes[*path];
        let new_hash = current_hashes[*path];
        if old_hash != new_hash {
            // Compute token delta.  We don't have the old token count in the
            // snapshot, so we report the current token count as the magnitude
            // (delta from an unknown baseline) represented as the current value.
            let current_tokens = current_token_counts
                .get(path.as_str())
                .copied()
                .unwrap_or(0) as i64;
            modified_files.push(FileChange {
                path: (*path).clone(),
                change: "modified".to_string(),
                tokens_delta: current_tokens,
            });
        }
    }
    // Sort for deterministic output.
    modified_files.sort_by(|a, b| a.path.cmp(&b.path));

    let mut new_files: Vec<String> = current_paths
        .difference(&snapshot_paths)
        .map(|p| (*p).clone())
        .collect();
    new_files.sort();

    let mut deleted_files: Vec<String> = snapshot_paths
        .difference(&current_paths)
        .map(|p| (*p).clone())
        .collect();
    deleted_files.sort();

    // --- Symbol-level changes ---
    // Build the current symbol set from the index.
    let current_symbols: HashMap<String, Vec<String>> = index
        .files
        .iter()
        .map(|f| {
            let names: Vec<String> = f
                .parse_result
                .as_ref()
                .map(|pr| pr.symbols.iter().map(|s| s.name.clone()).collect())
                .unwrap_or_default();
            (f.relative_path.clone(), names)
        })
        .collect();

    // Helper to build a set of (path, symbol_name) pairs.
    let to_flat_set = |map: &HashMap<String, Vec<String>>| -> HashSet<(String, String)> {
        map.iter()
            .flat_map(|(path, names)| names.iter().map(move |n| (path.clone(), n.clone())))
            .collect()
    };

    let snapshot_sym_set = to_flat_set(&snapshot.symbol_set);
    let current_sym_set = to_flat_set(&current_symbols);

    // Build a quick lookup from (path, symbol_name) -> kind string for the
    // current index so we can annotate SymbolChange entries.
    let kind_lookup: HashMap<(String, String), String> = index
        .files
        .iter()
        .flat_map(|f| {
            f.parse_result
                .as_ref()
                .map(|pr| {
                    pr.symbols.iter().map(move |s| {
                        (
                            (f.relative_path.clone(), s.name.clone()),
                            format!("{:?}", s.kind),
                        )
                    })
                })
                .into_iter()
                .flatten()
        })
        .collect();

    let mut new_symbols: Vec<SymbolChange> = current_sym_set
        .difference(&snapshot_sym_set)
        .map(|(path, name)| {
            let kind = kind_lookup
                .get(&(path.clone(), name.clone()))
                .cloned()
                .unwrap_or_else(|| "Unknown".to_string());
            SymbolChange {
                path: path.clone(),
                name: name.clone(),
                kind,
            }
        })
        .collect();
    new_symbols.sort_by(|a, b| a.path.cmp(&b.path).then(a.name.cmp(&b.name)));

    let mut removed_symbols: Vec<SymbolChange> = snapshot_sym_set
        .difference(&current_sym_set)
        .map(|(path, name)| SymbolChange {
            path: path.clone(),
            name: name.clone(),
            kind: "Unknown".to_string(),
        })
        .collect();
    removed_symbols.sort_by(|a, b| a.path.cmp(&b.path).then(a.name.cmp(&b.name)));

    // --- Graph edge changes ---
    let current_edge_set: HashSet<(String, String, String)> = index
        .graph
        .edges
        .iter()
        .flat_map(|(from, targets)| {
            targets
                .iter()
                .map(move |e| (from.clone(), e.target.clone(), format!("{:?}", e.edge_type)))
        })
        .collect();

    let mut graph_changes: Vec<GraphChange> = Vec::new();

    for (from, to, edge_type) in current_edge_set.difference(&snapshot.edge_set) {
        graph_changes.push(GraphChange {
            change_type: "added".to_string(),
            from: from.clone(),
            to: to.clone(),
            edge_type: edge_type.clone(),
        });
    }

    for (from, to, edge_type) in snapshot.edge_set.difference(&current_edge_set) {
        graph_changes.push(GraphChange {
            change_type: "removed".to_string(),
            from: from.clone(),
            to: to.clone(),
            edge_type: edge_type.clone(),
        });
    }
    graph_changes.sort_by(|a, b| {
        a.change_type
            .cmp(&b.change_type)
            .then(a.from.cmp(&b.from))
            .then(a.to.cmp(&b.to))
    });

    // --- Recommendation ---
    let recommendation = build_recommendation(
        &modified_files,
        &new_files,
        &deleted_files,
        &new_symbols,
        &removed_symbols,
        &graph_changes,
    );

    ContextDelta {
        modified_files,
        new_files,
        deleted_files,
        new_symbols,
        removed_symbols,
        graph_changes,
        recommendation,
    }
}

/// Return a delta with no changes and a recommendation to call the initial
/// context tool first, for use when no prior snapshot exists.
pub fn no_snapshot_recommendation() -> ContextDelta {
    ContextDelta {
        modified_files: Vec::new(),
        new_files: Vec::new(),
        deleted_files: Vec::new(),
        new_symbols: Vec::new(),
        removed_symbols: Vec::new(),
        graph_changes: Vec::new(),
        recommendation:
            "No prior context snapshot. Call cxpak_auto_context first to establish a baseline."
                .to_string(),
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn build_recommendation(
    modified_files: &[FileChange],
    new_files: &[String],
    deleted_files: &[String],
    new_symbols: &[SymbolChange],
    removed_symbols: &[SymbolChange],
    graph_changes: &[GraphChange],
) -> String {
    let total_file_changes = modified_files.len() + new_files.len() + deleted_files.len();
    let total_symbol_changes = new_symbols.len() + removed_symbols.len();
    let total_graph_changes = graph_changes.len();

    if total_file_changes == 0 && total_symbol_changes == 0 && total_graph_changes == 0 {
        return "No changes detected since last snapshot. Context is up to date.".to_string();
    }

    let mut parts: Vec<String> = Vec::new();

    if !modified_files.is_empty() {
        let paths: Vec<&str> = modified_files.iter().map(|f| f.path.as_str()).collect();
        if paths.len() == 1 {
            parts.push(format!("File '{}' was modified.", paths[0]));
        } else {
            parts.push(format!(
                "{} files changed: {}.",
                paths.len(),
                paths[..paths.len().min(3)].join(", ")
                    + if paths.len() > 3 { " and more" } else { "" }
            ));
        }
    }

    if !new_files.is_empty() {
        parts.push(format!(
            "{} new file(s) added: {}.",
            new_files.len(),
            new_files[..new_files.len().min(3)].join(", ")
                + if new_files.len() > 3 { " and more" } else { "" }
        ));
    }

    if !deleted_files.is_empty() {
        parts.push(format!(
            "{} file(s) deleted: {}.",
            deleted_files.len(),
            deleted_files[..deleted_files.len().min(3)].join(", ")
                + if deleted_files.len() > 3 {
                    " and more"
                } else {
                    ""
                }
        ));
    }

    if !new_symbols.is_empty() {
        parts.push(format!("{} new symbol(s) introduced.", new_symbols.len()));
    }

    if !removed_symbols.is_empty() {
        parts.push(format!("{} symbol(s) removed.", removed_symbols.len()));
    }

    if !graph_changes.is_empty() {
        parts.push(format!(
            "{} dependency graph edge(s) changed.",
            graph_changes.len()
        ));
    }

    parts.push(
        "Re-run cxpak_auto_context to refresh the context with the latest changes.".to_string(),
    );

    parts.join(" ")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::index::CodebaseIndex;
    use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Build a minimal `CodebaseIndex` from a list of (relative_path, content)
    /// pairs without touching the filesystem.
    fn make_index(files: &[(&str, &str)]) -> CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let scanned: Vec<ScannedFile> = files
            .iter()
            .map(|(rel, _content)| {
                let abs = dir.path().join(rel);
                if let Some(parent) = abs.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                std::fs::write(&abs, "").unwrap(); // empty on-disk file
                ScannedFile {
                    relative_path: rel.to_string(),
                    absolute_path: abs,
                    language: Some("rust".to_string()),
                    size_bytes: 0,
                }
            })
            .collect();

        let content_map: HashMap<String, String> = files
            .iter()
            .map(|(rel, content)| (rel.to_string(), content.to_string()))
            .collect();

        CodebaseIndex::build_with_content(scanned, HashMap::new(), &counter, content_map)
    }

    /// Build an index with explicit parse results (symbols).
    fn make_index_with_symbols(files: &[(&str, &str, Vec<Symbol>)]) -> CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let mut scanned = Vec::new();
        let mut parse_results = HashMap::new();
        let mut content_map = HashMap::new();

        for (rel, content, symbols) in files {
            let abs = dir.path().join(rel);
            if let Some(parent) = abs.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&abs, "").unwrap();
            scanned.push(ScannedFile {
                relative_path: rel.to_string(),
                absolute_path: abs,
                language: Some("rust".to_string()),
                size_bytes: 0,
            });
            parse_results.insert(
                rel.to_string(),
                ParseResult {
                    symbols: symbols.clone(),
                    imports: vec![],
                    exports: vec![],
                },
            );
            content_map.insert(rel.to_string(), content.to_string());
        }

        CodebaseIndex::build_with_content(scanned, parse_results, &counter, content_map)
    }

    fn make_symbol(name: &str) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: format!("pub fn {}()", name),
            body: "{}".to_string(),
            start_line: 1,
            end_line: 1,
        }
    }

    // -----------------------------------------------------------------------
    // test_snapshot_creation
    // -----------------------------------------------------------------------

    #[test]
    fn test_snapshot_creation() {
        let index = make_index(&[("src/a.rs", "fn a() {}"), ("src/b.rs", "fn b() {}")]);
        let snapshot = create_snapshot(&index);

        assert_eq!(snapshot.file_hashes.len(), 2);
        assert!(snapshot.file_hashes.contains_key("src/a.rs"));
        assert!(snapshot.file_hashes.contains_key("src/b.rs"));
        // Hashes must be non-zero (highly likely for any non-empty string).
        assert_ne!(snapshot.file_hashes["src/a.rs"], 0);
    }

    // -----------------------------------------------------------------------
    // test_diff_no_changes
    // -----------------------------------------------------------------------

    #[test]
    fn test_diff_no_changes() {
        let index = make_index(&[("src/a.rs", "fn a() {}")]);
        let snapshot = create_snapshot(&index);
        let delta = compute_diff(&snapshot, &index);

        assert!(delta.modified_files.is_empty());
        assert!(delta.new_files.is_empty());
        assert!(delta.deleted_files.is_empty());
        assert!(delta.new_symbols.is_empty());
        assert!(delta.removed_symbols.is_empty());
        assert!(delta.graph_changes.is_empty());
        assert!(
            delta.recommendation.contains("No changes"),
            "Unexpected recommendation: {}",
            delta.recommendation
        );
    }

    // -----------------------------------------------------------------------
    // test_diff_file_modified
    // -----------------------------------------------------------------------

    #[test]
    fn test_diff_file_modified() {
        // Take snapshot with original content.
        let original = make_index(&[("src/a.rs", "fn a() {}")]);
        let snapshot = create_snapshot(&original);

        // Build a new index where the file has different content.
        let modified = make_index(&[("src/a.rs", "fn a() { /* changed */ }")]);
        let delta = compute_diff(&snapshot, &modified);

        assert_eq!(delta.modified_files.len(), 1);
        assert_eq!(delta.modified_files[0].path, "src/a.rs");
        assert_eq!(delta.modified_files[0].change, "modified");
    }

    // -----------------------------------------------------------------------
    // test_diff_file_added
    // -----------------------------------------------------------------------

    #[test]
    fn test_diff_file_added() {
        let original = make_index(&[("src/a.rs", "fn a() {}")]);
        let snapshot = create_snapshot(&original);

        let with_new = make_index(&[("src/a.rs", "fn a() {}"), ("src/b.rs", "fn b() {}")]);
        let delta = compute_diff(&snapshot, &with_new);

        assert!(delta.new_files.contains(&"src/b.rs".to_string()));
        assert!(delta.deleted_files.is_empty());
    }

    // -----------------------------------------------------------------------
    // test_diff_file_deleted
    // -----------------------------------------------------------------------

    #[test]
    fn test_diff_file_deleted() {
        let original = make_index(&[("src/a.rs", "fn a() {}"), ("src/b.rs", "fn b() {}")]);
        let snapshot = create_snapshot(&original);

        let without_b = make_index(&[("src/a.rs", "fn a() {}")]);
        let delta = compute_diff(&snapshot, &without_b);

        assert!(delta.deleted_files.contains(&"src/b.rs".to_string()));
        assert!(delta.new_files.is_empty());
    }

    // -----------------------------------------------------------------------
    // test_diff_new_symbol
    // -----------------------------------------------------------------------

    #[test]
    fn test_diff_new_symbol() {
        let original =
            make_index_with_symbols(&[("src/a.rs", "fn a() {}", vec![make_symbol("a")])]);
        let snapshot = create_snapshot(&original);

        let with_new_sym = make_index_with_symbols(&[(
            "src/a.rs",
            "fn a() {} fn b() {}",
            vec![make_symbol("a"), make_symbol("b")],
        )]);
        let delta = compute_diff(&snapshot, &with_new_sym);

        let new_names: Vec<&str> = delta.new_symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            new_names.contains(&"b"),
            "Expected 'b' in new_symbols, got {:?}",
            new_names
        );
    }

    // -----------------------------------------------------------------------
    // test_diff_removed_symbol
    // -----------------------------------------------------------------------

    #[test]
    fn test_diff_removed_symbol() {
        let original = make_index_with_symbols(&[(
            "src/a.rs",
            "fn a() {} fn b() {}",
            vec![make_symbol("a"), make_symbol("b")],
        )]);
        let snapshot = create_snapshot(&original);

        let without_b =
            make_index_with_symbols(&[("src/a.rs", "fn a() {}", vec![make_symbol("a")])]);
        let delta = compute_diff(&snapshot, &without_b);

        let removed_names: Vec<&str> = delta
            .removed_symbols
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            removed_names.contains(&"b"),
            "Expected 'b' in removed_symbols, got {:?}",
            removed_names
        );
    }

    // -----------------------------------------------------------------------
    // test_diff_graph_edge_change
    // -----------------------------------------------------------------------

    #[test]
    fn test_diff_graph_edge_change() {
        use crate::schema::EdgeType;

        // Build an index with one file, take a snapshot.
        let original = make_index(&[("src/a.rs", "fn a() {}")]);
        let mut snapshot = create_snapshot(&original);

        // Manually inject an edge into the snapshot's edge_set to simulate
        // an edge that no longer exists in the current index.
        snapshot.edge_set.insert((
            "src/a.rs".to_string(),
            "src/b.rs".to_string(),
            format!("{:?}", EdgeType::Import),
        ));

        let delta = compute_diff(&snapshot, &original);

        let removed_edges: Vec<&GraphChange> = delta
            .graph_changes
            .iter()
            .filter(|g| g.change_type == "removed")
            .collect();

        assert_eq!(removed_edges.len(), 1);
        assert_eq!(removed_edges[0].from, "src/a.rs");
        assert_eq!(removed_edges[0].to, "src/b.rs");
        assert_eq!(
            removed_edges[0].edge_type,
            format!("{:?}", EdgeType::Import)
        );
    }

    // -----------------------------------------------------------------------
    // test_diff_no_snapshot
    // -----------------------------------------------------------------------

    #[test]
    fn test_diff_no_snapshot() {
        let delta = no_snapshot_recommendation();

        assert!(delta.modified_files.is_empty());
        assert!(delta.new_files.is_empty());
        assert!(delta.deleted_files.is_empty());
        assert!(delta.new_symbols.is_empty());
        assert!(delta.removed_symbols.is_empty());
        assert!(delta.graph_changes.is_empty());
        assert!(
            delta.recommendation.contains("No prior context snapshot"),
            "Unexpected recommendation: {}",
            delta.recommendation
        );
        assert!(
            delta.recommendation.contains("cxpak_auto_context"),
            "Recommendation should mention cxpak_auto_context: {}",
            delta.recommendation
        );
    }

    // -----------------------------------------------------------------------
    // test_diff_recommendation_text
    // -----------------------------------------------------------------------

    #[test]
    fn test_diff_recommendation_text() {
        let original = make_index(&[("src/a.rs", "fn a() {}")]);
        let snapshot = create_snapshot(&original);

        // Modify the file so we get a non-empty delta.
        let modified = make_index(&[("src/a.rs", "fn a_changed() {}")]);
        let delta = compute_diff(&snapshot, &modified);

        assert!(
            delta.recommendation.contains("src/a.rs"),
            "Recommendation should mention the changed file. Got: {}",
            delta.recommendation
        );
        assert!(
            delta.recommendation.contains("cxpak_auto_context"),
            "Recommendation should suggest re-running cxpak_auto_context. Got: {}",
            delta.recommendation
        );
    }

    // -----------------------------------------------------------------------
    // Additional edge-case tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_snapshot_edge_set_populated_from_graph() {
        use crate::schema::EdgeType;

        let index = make_index(&[("src/a.rs", "fn a() {}"), ("src/b.rs", "fn b() {}")]);
        // Manually inject an edge so we can verify it appears in the snapshot.
        let mut index_with_edge = index;
        index_with_edge
            .graph
            .add_edge("src/a.rs", "src/b.rs", EdgeType::Import);

        let snapshot = create_snapshot(&index_with_edge);
        let expected_edge = (
            "src/a.rs".to_string(),
            "src/b.rs".to_string(),
            format!("{:?}", EdgeType::Import),
        );
        assert!(
            snapshot.edge_set.contains(&expected_edge),
            "snapshot edge_set should contain the injected edge"
        );
    }

    #[test]
    fn test_diff_added_graph_edge() {
        use crate::schema::EdgeType;

        let original = make_index(&[("src/a.rs", "fn a() {}"), ("src/b.rs", "fn b() {}")]);
        let snapshot = create_snapshot(&original);

        // Add an edge to the "current" index.
        let mut with_edge = make_index(&[("src/a.rs", "fn a() {}"), ("src/b.rs", "fn b() {}")]);
        with_edge
            .graph
            .add_edge("src/a.rs", "src/b.rs", EdgeType::Import);

        let delta = compute_diff(&snapshot, &with_edge);

        let added_edges: Vec<&GraphChange> = delta
            .graph_changes
            .iter()
            .filter(|g| g.change_type == "added")
            .collect();
        assert_eq!(added_edges.len(), 1);
        assert_eq!(added_edges[0].from, "src/a.rs");
        assert_eq!(added_edges[0].to, "src/b.rs");
    }
}
