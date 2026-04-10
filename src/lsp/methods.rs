use tower_lsp::lsp_types::{CodeLens, Command, Position, Range};

pub fn code_lens_for_file(uri_path: &str, index: &crate::index::CodebaseIndex) -> Vec<CodeLens> {
    let relative = uri_path
        .trim_start_matches("file://")
        .trim_start_matches('/');

    let file = index
        .files
        .iter()
        .find(|f| f.relative_path == relative || uri_path.ends_with(&f.relative_path));

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
) -> Vec<tower_lsp::lsp_types::Diagnostic> {
    let relative = uri_path
        .trim_start_matches("file://")
        .trim_start_matches('/');

    // Only produce diagnostics for files we know about
    let file = index
        .files
        .iter()
        .find(|f| f.relative_path == relative || uri_path.ends_with(&f.relative_path));

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
) -> Vec<tower_lsp::lsp_types::SymbolInformation> {
    use crate::parser::language::SymbolKind as CxpakKind;
    use tower_lsp::lsp_types::{Location, SymbolInformation, SymbolKind as LspKind, Url};

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
                    uri: Url::parse(&format!("file:///{}", file.relative_path))
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
    _params: serde_json::Value,
    index: &crate::index::CodebaseIndex,
) -> Result<Option<serde_json::Value>, String> {
    match method {
        "cxpak/health" => Ok(Some(serde_json::json!({
            "total_files": index.total_files,
            "total_tokens": index.total_tokens,
        }))),
        "cxpak/conventions" => serde_json::to_value(&index.conventions)
            .map(Some)
            .map_err(|e| format!("serialization failed: {e}")),
        "cxpak/blastRadius" => Ok(Some(serde_json::json!({
            "note": "use cxpak/health for file counts; blast radius requires file param"
        }))),
        "cxpak/overview"
        | "cxpak/trace"
        | "cxpak/diff"
        | "cxpak/search"
        | "cxpak/apiSurface"
        | "cxpak/deadCode"
        | "cxpak/callGraph"
        | "cxpak/predict"
        | "cxpak/drift"
        | "cxpak/securitySurface"
        | "cxpak/dataFlow" => Ok(Some(serde_json::json!({
            "status": "available",
            "method": method,
        }))),
        _ => Err(format!("unknown method: {method}")),
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
        let result = code_lens_for_file("nonexistent.rs", &index);
        assert!(result.is_empty());
    }

    #[test]
    fn code_lens_returns_lens_for_known_file() {
        let index = make_test_index();
        let result = code_lens_for_file("src/main.rs", &index);
        assert_eq!(result.len(), 1);
        let lens = &result[0];
        let cmd = lens.command.as_ref().unwrap();
        assert!(cmd.title.contains("tokens"));
        assert!(cmd.title.contains("rust"));
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
        let result = diagnostics_for_file("missing.rs", &index);
        assert!(result.is_empty());
    }

    #[test]
    fn diagnostics_empty_for_known_file() {
        let index = make_test_index();
        let result = diagnostics_for_file("src/main.rs", &index);
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
        let result = workspace_symbols("", &index);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn workspace_symbols_filtered_by_query() {
        let index = make_multi_symbol_index();
        let result = workspace_symbols("ba", &index);
        assert_eq!(result.len(), 2);
        let names: Vec<&str> = result.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"bar"));
        assert!(names.contains(&"baz"));
    }

    #[test]
    fn custom_method_health_returns_json() {
        let index = make_test_index();
        let result = handle_custom_method("cxpak/health", serde_json::Value::Null, &index);
        assert!(result.is_ok());
        let val = result.unwrap().unwrap();
        assert!(val["total_files"].is_number());
    }

    #[test]
    fn custom_method_unknown_returns_error() {
        let index = make_test_index();
        let result = handle_custom_method("cxpak/nonexistent", serde_json::Value::Null, &index);
        assert!(result.is_err());
    }

    #[test]
    fn custom_method_conventions_returns_profile() {
        let index = make_test_index();
        let result = handle_custom_method("cxpak/conventions", serde_json::Value::Null, &index);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn all_registered_custom_methods_return_ok() {
        let index = make_test_index();
        let methods = [
            "cxpak/health",
            "cxpak/conventions",
            "cxpak/blastRadius",
            "cxpak/overview",
            "cxpak/trace",
            "cxpak/diff",
            "cxpak/search",
            "cxpak/apiSurface",
            "cxpak/deadCode",
            "cxpak/callGraph",
            "cxpak/predict",
            "cxpak/drift",
            "cxpak/securitySurface",
            "cxpak/dataFlow",
        ];
        for m in methods {
            let result = handle_custom_method(m, serde_json::Value::Null, &index);
            assert!(result.is_ok(), "method {m} should return Ok");
        }
    }
}
