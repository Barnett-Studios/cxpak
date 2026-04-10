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
}
