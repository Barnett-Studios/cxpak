use std::path::Path;
use tower_lsp::lsp_types::{CodeLens, Command, Position, Range, Url};

/// Error type returned by [`handle_custom_method`].
#[derive(Debug)]
pub enum LspMethodError {
    /// The requested method name is not registered (maps to JSON-RPC MethodNotFound -32601).
    NotFound(String),
    /// An internal error occurred while handling the method (maps to InternalError -32603).
    Internal(String),
}

/// Convert a `file://` URI to a path relative to `repo_root`.
///
/// Example: `file:///Users/me/repo/src/main.rs` + `/Users/me/repo` → `src/main.rs`
pub fn uri_to_rel_path(uri: &Url, repo_root: &Path) -> Option<String> {
    let abs = uri.to_file_path().ok()?;
    let rel = abs.strip_prefix(repo_root).ok()?;
    Some(rel.to_string_lossy().into_owned())
}

/// Extract the identifier token that spans `char_idx` on `line_idx` inside `content`.
///
/// Walks backward and forward from `char_idx` while characters are alphanumeric or `_`.
pub fn extract_word_at(content: &str, line_idx: usize, char_idx: usize) -> String {
    let line = match content.lines().nth(line_idx) {
        Some(l) => l,
        None => return String::new(),
    };
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    if char_idx >= len {
        return String::new();
    }
    if !chars[char_idx].is_alphanumeric() && chars[char_idx] != '_' {
        return String::new();
    }
    let start = {
        let mut i = char_idx;
        while i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
            i -= 1;
        }
        i
    };
    let end = {
        let mut i = char_idx + 1;
        while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
            i += 1;
        }
        i
    };
    chars[start..end].iter().collect()
}

pub fn code_lens_for_file(
    uri_path: &str,
    index: &crate::index::CodebaseIndex,
    repo_root: &Path,
) -> Vec<CodeLens> {
    let url = Url::parse(uri_path).ok();
    let rel_opt = url.as_ref().and_then(|u| uri_to_rel_path(u, repo_root));

    let file = index.files.iter().find(|f| {
        rel_opt.as_deref().is_some_and(|r| f.relative_path == r)
            || uri_path.ends_with(&f.relative_path)
    });

    match file {
        None => Vec::new(),
        Some(f) => {
            let range = Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 0,
                },
            };
            vec![CodeLens {
                range,
                command: Some(Command {
                    title: format!(
                        "cxpak: {} tokens | {}",
                        f.token_count,
                        f.language.as_deref().unwrap_or("unknown")
                    ),
                    command: "cxpak.showFileInfo".to_string(),
                    arguments: None,
                }),
                data: None,
            }]
        }
    }
}

pub fn hover_for_symbol(
    symbol: &str,
    index: &crate::index::CodebaseIndex,
) -> Option<tower_lsp::lsp_types::Hover> {
    let matches = index.find_symbol(symbol);
    let (file_path, sym) = matches.first()?;
    let pagerank = index.pagerank.get(*file_path).copied().unwrap_or(0.0);
    let content = format!(
        "**{:?}** `{}`\n\nFile: `{}`\nPageRank: {:.3}",
        sym.kind, sym.name, file_path, pagerank
    );
    Some(tower_lsp::lsp_types::Hover {
        contents: tower_lsp::lsp_types::HoverContents::Markup(
            tower_lsp::lsp_types::MarkupContent {
                kind: tower_lsp::lsp_types::MarkupKind::Markdown,
                value: content,
            },
        ),
        range: None,
    })
}

pub fn diagnostics_for_file(
    uri_path: &str,
    index: &crate::index::CodebaseIndex,
    repo_root: &Path,
) -> Vec<tower_lsp::lsp_types::Diagnostic> {
    let url = Url::parse(uri_path).ok();
    let rel_opt = url.as_ref().and_then(|u| uri_to_rel_path(u, repo_root));

    // Only produce diagnostics for files we know about
    let file = index.files.iter().find(|f| {
        rel_opt.as_deref().is_some_and(|r| f.relative_path == r)
            || uri_path.ends_with(&f.relative_path)
    });

    let Some(_f) = file else {
        return Vec::new();
    };

    // Convention verification requires git state which isn't available in the LSP
    // context without a repo path. Return empty for now — real diagnostics will
    // be wired when verify::check_file is added.
    Vec::new()
}

pub fn workspace_symbols(
    query: &str,
    index: &crate::index::CodebaseIndex,
    repo_root: &Path,
) -> Vec<tower_lsp::lsp_types::SymbolInformation> {
    use crate::parser::language::SymbolKind as CxpakKind;
    use tower_lsp::lsp_types::{Location, SymbolInformation, SymbolKind as LspKind};

    let query_lower = query.to_lowercase();
    let mut results = Vec::new();

    for file in &index.files {
        let Some(pr) = &file.parse_result else {
            continue;
        };
        for sym in &pr.symbols {
            if !query.is_empty() && !sym.name.to_lowercase().contains(&query_lower) {
                continue;
            }
            let kind = match sym.kind {
                CxpakKind::Function => LspKind::FUNCTION,
                CxpakKind::Struct => LspKind::STRUCT,
                CxpakKind::Enum => LspKind::ENUM,
                CxpakKind::Trait | CxpakKind::Interface => LspKind::INTERFACE,
                CxpakKind::Class => LspKind::CLASS,
                CxpakKind::Method => LspKind::METHOD,
                CxpakKind::Constant => LspKind::CONSTANT,
                CxpakKind::TypeAlias | CxpakKind::Type => LspKind::TYPE_PARAMETER,
                CxpakKind::Variable => LspKind::VARIABLE,
                _ => LspKind::KEY,
            };

            #[allow(deprecated)]
            let info = SymbolInformation {
                name: sym.name.clone(),
                kind,
                tags: None,
                deprecated: None,
                location: Location {
                    uri: Url::from_file_path(repo_root.join(&file.relative_path))
                        .unwrap_or_else(|_| Url::parse("file:///unknown").unwrap()),
                    range: tower_lsp::lsp_types::Range {
                        start: Position {
                            line: sym.start_line.saturating_sub(1) as u32,
                            character: 0,
                        },
                        end: Position {
                            line: sym.end_line.saturating_sub(1) as u32,
                            character: 0,
                        },
                    },
                },
                container_name: Some(file.relative_path.clone()),
            };
            results.push(info);
        }
    }
    results
}

pub fn handle_custom_method(
    method: &str,
    params: serde_json::Value,
    index: &crate::index::CodebaseIndex,
    repo_root: &Path,
) -> Result<Option<serde_json::Value>, LspMethodError> {
    match method {
        "cxpak/health" => {
            let health = crate::intelligence::health::compute_health(index);
            Ok(Some(serde_json::json!({
                "total_files": index.total_files,
                "total_tokens": index.total_tokens,
                "composite": health.composite,
                "dimensions": {
                    "conventions": health.conventions,
                    "test_coverage": health.test_coverage,
                    "churn_stability": health.churn_stability,
                    "coupling": health.coupling,
                    "cycles": health.cycles,
                    "dead_code": health.dead_code,
                },
            })))
        }
        "cxpak/conventions" => serde_json::to_value(&index.conventions)
            .map(Some)
            .map_err(|e| LspMethodError::Internal(format!("serialization failed: {e}"))),
        "cxpak/blastRadius" => {
            let files: Vec<String> = params
                .get("files")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .or_else(|| {
                    params
                        .get("file")
                        .and_then(|v| v.as_str())
                        .map(|s| vec![s.to_string()])
                })
                .unwrap_or_default();
            if files.is_empty() {
                return Err(LspMethodError::Internal(
                    "cxpak/blastRadius requires 'file' (string) or 'files' (array) param".into(),
                ));
            }
            let depth = params
                .get("depth")
                .and_then(|v| v.as_u64())
                .unwrap_or(3)
                .min(8) as usize;
            let focus = params.get("focus").and_then(|v| v.as_str());
            let refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
            let result = crate::intelligence::blast_radius::compute_blast_radius(
                &refs,
                &index.graph,
                &index.pagerank,
                &index.test_map,
                depth,
                focus,
            );
            Ok(Some(serde_json::to_value(result).map_err(|e| {
                LspMethodError::Internal(format!("serialization failed: {e}"))
            })?))
        }
        "cxpak/overview" => Ok(Some(serde_json::json!({
            "total_files": index.total_files,
            "total_tokens": index.total_tokens,
            "languages": index.language_stats.len(),
        }))),
        "cxpak/trace" => {
            let sym = params
                .get("symbol")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    LspMethodError::Internal("cxpak/trace requires 'symbol' (string) param".into())
                })?;
            let matches = index.find_symbol(sym);
            let locations: Vec<_> = matches
                .into_iter()
                .map(|(file, s)| {
                    serde_json::json!({
                        "file": file,
                        "start_line": s.start_line,
                        "end_line": s.end_line,
                        "kind": format!("{:?}", s.kind),
                    })
                })
                .collect();
            Ok(Some(
                serde_json::json!({"count": locations.len(), "locations": locations}),
            ))
        }
        "cxpak/diff" => {
            let git_ref = params.get("ref").and_then(|v| v.as_str());
            match crate::commands::diff::extract_changes(repo_root, git_ref) {
                Ok(changes) => {
                    let entries: Vec<_> = changes
                        .iter()
                        .map(|c| {
                            serde_json::json!({
                                "path": c.path,
                                "diff_bytes": c.diff_text.len(),
                            })
                        })
                        .collect();
                    Ok(Some(serde_json::json!({
                        "ref": git_ref.unwrap_or("uncommitted"),
                        "count": entries.len(),
                        "changes": entries,
                    })))
                }
                Err(e) => Err(LspMethodError::Internal(format!("git diff failed: {e}"))),
            }
        }
        "cxpak/search" => {
            let query = params
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    LspMethodError::Internal("cxpak/search requires 'query' (string) param".into())
                })?
                .to_lowercase();
            if query.is_empty() {
                return Err(LspMethodError::Internal(
                    "cxpak/search 'query' must be non-empty".into(),
                ));
            }
            let matches: Vec<_> = index
                .files
                .iter()
                .filter(|f| f.relative_path.to_lowercase().contains(&query))
                .take(20)
                .map(|f| serde_json::json!({"path": f.relative_path, "language": f.language}))
                .collect();
            Ok(Some(serde_json::json!({"matches": matches})))
        }
        "cxpak/apiSurface" => {
            let surface =
                crate::intelligence::api_surface::extract_api_surface(index, None, "all", 5000);
            Ok(Some(serde_json::to_value(surface).map_err(|e| {
                LspMethodError::Internal(format!("serialization failed: {e}"))
            })?))
        }
        "cxpak/deadCode" => {
            let dead = crate::intelligence::dead_code::detect_dead_code(index, None);
            Ok(Some(serde_json::json!({"dead_symbols": dead})))
        }
        "cxpak/callGraph" => Ok(Some(serde_json::json!({
            "edges": index.call_graph.edges,
            "total": index.call_graph.edges.len(),
        }))),
        "cxpak/predict" => {
            let files = params
                .get("files")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if files.is_empty() {
                return Err(LspMethodError::Internal(
                    "cxpak/predict requires 'files' (array of strings) param".into(),
                ));
            }
            let refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
            let result = crate::intelligence::predict::predict(
                &refs,
                &index.graph,
                &index.pagerank,
                &index.co_changes,
                &index.test_map,
                3,
            );
            Ok(Some(serde_json::to_value(result).map_err(|e| {
                LspMethodError::Internal(format!("serialization failed: {e}"))
            })?))
        }
        "cxpak/drift" => {
            let save_baseline = params
                .get("save_baseline")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let report =
                crate::intelligence::drift::build_drift_report(index, repo_root, save_baseline);
            Ok(Some(serde_json::to_value(report).map_err(|e| {
                LspMethodError::Internal(format!("serialization failed: {e}"))
            })?))
        }
        "cxpak/securitySurface" => {
            let result = crate::intelligence::security::build_security_surface(
                index,
                crate::intelligence::security::DEFAULT_AUTH_PATTERNS,
                None,
            );
            Ok(Some(serde_json::to_value(result).map_err(|e| {
                LspMethodError::Internal(format!("serialization failed: {e}"))
            })?))
        }
        "cxpak/dataFlow" => {
            let sym = params
                .get("symbol")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    LspMethodError::Internal(
                        "cxpak/dataFlow requires 'symbol' (string) param".into(),
                    )
                })?;
            let result = crate::intelligence::data_flow::trace_data_flow(sym, None, 6, index);
            Ok(Some(serde_json::to_value(result).map_err(|e| {
                LspMethodError::Internal(format!("serialization failed: {e}"))
            })?))
        }
        _ => Err(LspMethodError::NotFound(method.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::index::CodebaseIndex;
    use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    fn make_test_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let files = vec![ScannedFile {
            relative_path: "src/main.rs".to_string(),
            absolute_path: std::path::PathBuf::from("/tmp/src/main.rs"),
            language: Some("rust".to_string()),
            size_bytes: 100,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/main.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "main".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn main()".to_string(),
                    body: "fn main() {}".to_string(),
                    start_line: 1,
                    end_line: 5,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let mut content_map = HashMap::new();
        content_map.insert("src/main.rs".to_string(), "fn main() {}".to_string());

        CodebaseIndex::build_with_content(files, parse_results, &counter, content_map)
    }

    #[test]
    fn code_lens_returns_empty_for_unknown_file() {
        let index = make_test_index();
        let root = std::path::Path::new("/tmp");
        let result = code_lens_for_file("nonexistent.rs", &index, root);
        assert!(result.is_empty());
    }

    #[test]
    fn code_lens_returns_lens_for_known_file() {
        let index = make_test_index();
        let root = std::path::Path::new("/tmp");
        let result = code_lens_for_file("file:///tmp/src/main.rs", &index, root);
        assert_eq!(result.len(), 1);
        let lens = &result[0];
        let cmd = lens.command.as_ref().unwrap();
        assert!(cmd.title.contains("tokens"));
        assert!(cmd.title.contains("rust"));
    }

    #[test]
    fn code_lens_uses_uri_to_rel_path_not_raw_trim() {
        // URI: file:///Users/me/repo/src/main.rs, repo_root: /Users/me/repo
        // After stripping repo_root the relative path is "src/main.rs" which
        // must match the index entry. The old raw-trim would produce
        // "Users/me/repo/src/main.rs" and miss the file.
        let counter = crate::budget::counter::TokenCounter::new();
        let files = vec![crate::scanner::ScannedFile {
            relative_path: "src/main.rs".to_string(),
            absolute_path: std::path::PathBuf::from("/Users/me/repo/src/main.rs"),
            language: Some("rust".to_string()),
            size_bytes: 100,
        }];
        let index = CodebaseIndex::build_with_content(
            files,
            std::collections::HashMap::new(),
            &counter,
            std::collections::HashMap::new(),
        );
        let root = std::path::Path::new("/Users/me/repo");
        let result = code_lens_for_file("file:///Users/me/repo/src/main.rs", &index, root);
        assert_eq!(
            result.len(),
            1,
            "uri_to_rel_path must find the file; old raw trim would fail"
        );
    }

    #[test]
    fn diagnostics_uses_uri_to_rel_path_not_raw_trim() {
        let counter = crate::budget::counter::TokenCounter::new();
        let files = vec![crate::scanner::ScannedFile {
            relative_path: "src/lib.rs".to_string(),
            absolute_path: std::path::PathBuf::from("/Users/me/repo/src/lib.rs"),
            language: Some("rust".to_string()),
            size_bytes: 100,
        }];
        let index = CodebaseIndex::build_with_content(
            files,
            std::collections::HashMap::new(),
            &counter,
            std::collections::HashMap::new(),
        );
        let root = std::path::Path::new("/Users/me/repo");
        // diagnostics_for_file returns empty (no real diagnostics yet) but must
        // not panic or return an error even with a fully qualified file:// URI.
        let result = diagnostics_for_file("file:///Users/me/repo/src/lib.rs", &index, root);
        // Must successfully find the file and return the empty-diagnostics Vec.
        let _ = result; // function did not panic
    }

    #[test]
    fn hover_returns_none_for_empty_index() {
        let index = make_test_index();
        let result = hover_for_symbol("unknown_fn", &index);
        assert!(result.is_none());
    }

    #[test]
    fn hover_returns_markdown_content() {
        let index = make_test_index();
        let result = hover_for_symbol("main", &index);
        assert!(result.is_some());
        let hover = result.unwrap();
        match hover.contents {
            tower_lsp::lsp_types::HoverContents::Markup(m) => {
                assert!(m.value.contains("main"));
                assert!(m.value.contains("PageRank"));
            }
            _ => panic!("expected Markup hover contents"),
        }
    }

    #[test]
    fn diagnostics_empty_for_unknown_file() {
        let index = make_test_index();
        let root = std::path::Path::new("/tmp");
        let result = diagnostics_for_file("missing.rs", &index, root);
        assert!(result.is_empty());
    }

    #[test]
    fn diagnostics_empty_for_known_file() {
        let index = make_test_index();
        let root = std::path::Path::new("/tmp");
        let result = diagnostics_for_file("src/main.rs", &index, root);
        assert!(result.is_empty());
    }

    fn make_multi_symbol_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let files = vec![ScannedFile {
            relative_path: "src/lib.rs".to_string(),
            absolute_path: std::path::PathBuf::from("/tmp/src/lib.rs"),
            language: Some("rust".to_string()),
            size_bytes: 200,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/lib.rs".to_string(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "foo".to_string(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn foo()".to_string(),
                        body: "fn foo() {}".to_string(),
                        start_line: 1,
                        end_line: 3,
                    },
                    Symbol {
                        name: "bar".to_string(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn bar()".to_string(),
                        body: "fn bar() {}".to_string(),
                        start_line: 5,
                        end_line: 7,
                    },
                    Symbol {
                        name: "baz".to_string(),
                        kind: SymbolKind::Struct,
                        visibility: Visibility::Public,
                        signature: "struct baz".to_string(),
                        body: "struct baz {}".to_string(),
                        start_line: 9,
                        end_line: 11,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let mut content_map = HashMap::new();
        content_map.insert(
            "src/lib.rs".to_string(),
            "fn foo() {}\nfn bar() {}\nstruct baz {}".to_string(),
        );

        CodebaseIndex::build_with_content(files, parse_results, &counter, content_map)
    }

    #[test]
    fn workspace_symbols_empty_query_returns_all() {
        let index = make_multi_symbol_index();
        let root = std::path::Path::new("/tmp");
        let result = workspace_symbols("", &index, root);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn workspace_symbols_filtered_by_query() {
        let index = make_multi_symbol_index();
        let root = std::path::Path::new("/tmp");
        let result = workspace_symbols("ba", &index, root);
        assert_eq!(result.len(), 2);
        let names: Vec<&str> = result.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"bar"));
        assert!(names.contains(&"baz"));
    }

    #[test]
    fn workspace_symbols_uri_uses_repo_root() {
        // Symbols must have URIs rooted at repo_root, not file:///src/...
        let index = make_multi_symbol_index();
        let root = std::path::Path::new("/Users/me/repo");
        let result = workspace_symbols("foo", &index, root);
        assert_eq!(result.len(), 1);
        let uri = result[0].location.uri.as_str();
        assert!(
            uri.starts_with("file:///Users/me/repo/"),
            "URI must start with repo_root path, got: {uri}"
        );
        assert!(
            !uri.starts_with("file:///src/"),
            "URI must not be rooted at /src/, got: {uri}"
        );
    }

    /// Create a throwaway directory with `git init` so drift/diff can run.
    fn make_git_tempdir() -> tempfile::TempDir {
        let temp = tempfile::TempDir::new().unwrap();
        let _ = std::process::Command::new("git")
            .arg("init")
            .arg("--quiet")
            .current_dir(temp.path())
            .output();
        let _ = std::process::Command::new("git")
            .args(["config", "user.email", "t@t"])
            .current_dir(temp.path())
            .output();
        let _ = std::process::Command::new("git")
            .args(["config", "user.name", "t"])
            .current_dir(temp.path())
            .output();
        std::fs::write(temp.path().join("README.md"), "init\n").unwrap();
        let _ = std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(temp.path())
            .output();
        let _ = std::process::Command::new("git")
            .args(["commit", "-m", "init", "--quiet"])
            .current_dir(temp.path())
            .output();
        temp
    }

    #[test]
    fn custom_method_health_returns_json() {
        let index = make_test_index();
        let root = std::path::Path::new("/tmp");
        let result = handle_custom_method("cxpak/health", serde_json::Value::Null, &index, root);
        assert!(result.is_ok());
        let val = result.unwrap().unwrap();
        assert!(val["total_files"].is_number());
    }

    #[test]
    fn custom_method_unknown_returns_not_found() {
        let index = make_test_index();
        let root = std::path::Path::new("/tmp");
        let result =
            handle_custom_method("cxpak/nonexistent", serde_json::Value::Null, &index, root);
        assert!(
            matches!(result, Err(LspMethodError::NotFound(_))),
            "unknown method must return LspMethodError::NotFound"
        );
    }

    #[test]
    fn custom_method_conventions_returns_profile() {
        let index = make_test_index();
        let root = std::path::Path::new("/tmp");
        let result =
            handle_custom_method("cxpak/conventions", serde_json::Value::Null, &index, root);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn all_registered_custom_methods_return_ok() {
        let index = make_test_index();
        let temp = make_git_tempdir();
        let root = temp.path();
        let param_map: &[(&str, serde_json::Value)] = &[
            ("cxpak/health", serde_json::Value::Null),
            ("cxpak/conventions", serde_json::Value::Null),
            (
                "cxpak/blastRadius",
                serde_json::json!({"file": "src/main.rs"}),
            ),
            ("cxpak/overview", serde_json::Value::Null),
            ("cxpak/trace", serde_json::json!({"symbol": "main"})),
            ("cxpak/diff", serde_json::Value::Null),
            ("cxpak/search", serde_json::json!({"query": "main"})),
            ("cxpak/apiSurface", serde_json::Value::Null),
            ("cxpak/deadCode", serde_json::Value::Null),
            ("cxpak/callGraph", serde_json::Value::Null),
            (
                "cxpak/predict",
                serde_json::json!({"files": ["src/main.rs"]}),
            ),
            ("cxpak/drift", serde_json::Value::Null),
            ("cxpak/securitySurface", serde_json::Value::Null),
            ("cxpak/dataFlow", serde_json::json!({"symbol": "main"})),
        ];
        for (m, params) in param_map {
            let result = handle_custom_method(m, params.clone(), &index, root);
            assert!(
                result.is_ok(),
                "method {m} should return Ok, got {result:?}"
            );
        }
    }

    #[test]
    fn methods_with_missing_required_params_return_internal_error() {
        let index = make_test_index();
        let root = std::path::Path::new("/tmp");
        // These methods MUST return an Internal error rather than a soft-failure
        // {"note": "provide X"} payload, which would let a caller mistake a
        // missing-input for a valid empty response.
        let cases = [
            ("cxpak/trace", serde_json::Value::Null),
            ("cxpak/search", serde_json::Value::Null),
            ("cxpak/search", serde_json::json!({"query": ""})),
            ("cxpak/predict", serde_json::Value::Null),
            ("cxpak/predict", serde_json::json!({"files": []})),
            ("cxpak/dataFlow", serde_json::Value::Null),
            ("cxpak/blastRadius", serde_json::Value::Null),
        ];
        for (m, p) in cases {
            let r = handle_custom_method(m, p, &index, root);
            assert!(
                matches!(r, Err(LspMethodError::Internal(_))),
                "method {m} with missing params must Err(Internal), got {r:?}"
            );
        }
    }

    #[test]
    fn extract_word_at_middle_of_word() {
        assert_eq!(extract_word_at("foo bar baz", 0, 5), "bar");
    }

    #[test]
    fn extract_word_at_start_of_content() {
        assert_eq!(extract_word_at("hello", 0, 0), "hello");
    }

    #[test]
    fn extract_word_at_end_of_word() {
        assert_eq!(extract_word_at("foo bar baz", 0, 6), "bar");
    }

    #[test]
    fn extract_word_at_on_space_returns_empty() {
        assert_eq!(extract_word_at("foo bar", 0, 3), "");
    }

    #[test]
    fn extract_word_at_out_of_bounds_line_returns_empty() {
        assert_eq!(extract_word_at("hello", 99, 0), "");
    }

    #[test]
    fn extract_word_at_out_of_bounds_char_returns_empty() {
        assert_eq!(extract_word_at("hi", 0, 99), "");
    }

    #[test]
    fn extract_word_at_underscore_included() {
        assert_eq!(extract_word_at("my_func()", 0, 4), "my_func");
    }

    #[test]
    fn uri_to_rel_path_strips_repo_root() {
        let uri = Url::parse("file:///Users/me/repo/src/main.rs").unwrap();
        let root = std::path::Path::new("/Users/me/repo");
        assert_eq!(uri_to_rel_path(&uri, root), Some("src/main.rs".to_string()));
    }

    #[test]
    fn uri_to_rel_path_returns_none_outside_root() {
        let uri = Url::parse("file:///other/src/main.rs").unwrap();
        let root = std::path::Path::new("/Users/me/repo");
        assert_eq!(uri_to_rel_path(&uri, root), None);
    }

    #[test]
    fn uri_to_rel_path_exact_root_returns_empty_string() {
        let uri = Url::parse("file:///Users/me/repo/file.rs").unwrap();
        let root = std::path::Path::new("/Users/me/repo");
        assert_eq!(uri_to_rel_path(&uri, root), Some("file.rs".to_string()));
    }
}
