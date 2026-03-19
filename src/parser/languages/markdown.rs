use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct MarkdownLanguage;

impl MarkdownLanguage {
    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn full_text(node: &tree_sitter::Node, source: &[u8]) -> String {
        Self::node_text(node, source).to_string()
    }

    /// Extract the heading text (without `#` markers).
    fn extract_heading_text(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            if kind == "heading_content" || kind == "inline" || kind == "paragraph" {
                return Self::node_text(&child, source).trim().to_string();
            }
        }
        // Fallback: strip leading # characters
        let text = Self::node_text(node, source).trim().to_string();
        text.trim_start_matches('#').trim().to_string()
    }

    /// Determine heading level from the atx_heading node.
    fn heading_level(node: &tree_sitter::Node, source: &[u8]) -> usize {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            // tree-sitter-markdown uses "atx_h1_marker", "atx_h2_marker", etc.
            if kind.starts_with("atx_h") && kind.ends_with("_marker") {
                let level_str = kind.trim_start_matches("atx_h").trim_end_matches("_marker");
                if let Ok(level) = level_str.parse::<usize>() {
                    return level;
                }
            }
        }
        // Fallback: count leading # characters
        let text = Self::node_text(node, source);
        text.chars().take_while(|&c| c == '#').count().max(1)
    }

    /// Extract the language tag from a fenced code block.
    fn extract_code_block_lang(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "info_string" || child.kind() == "language" {
                return Self::node_text(&child, source).trim().to_string();
            }
        }
        String::new()
    }
}

impl LanguageSupport for MarkdownLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_markdown_updated::language()
    }

    fn name(&self) -> &str {
        "markdown"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let imports: Vec<Import> = Vec::new();
        let exports: Vec<Export> = Vec::new();

        // Markdown's tree structure can be nested in sections, so we do a
        // recursive walk to find headings and code blocks anywhere in the tree.
        let mut stack: Vec<tree_sitter::Node> = Vec::new();
        stack.push(root);

        while let Some(current) = stack.pop() {
            let mut cursor = current.walk();
            for child in current.children(&mut cursor) {
                match child.kind() {
                    "atx_heading" | "setext_heading" => {
                        let heading_text = Self::extract_heading_text(&child, source_bytes);
                        let level = Self::heading_level(&child, source_bytes);
                        let name = if heading_text.is_empty() {
                            format!("h{}", level)
                        } else {
                            heading_text
                        };
                        let start_line = child.start_position().row + 1;
                        let end_line = child.end_position().row + 1;

                        symbols.push(Symbol {
                            name,
                            kind: SymbolKind::Heading,
                            visibility: Visibility::Public,
                            signature: Self::node_text(&child, source_bytes)
                                .lines()
                                .next()
                                .unwrap_or("")
                                .trim()
                                .to_string(),
                            body: Self::full_text(&child, source_bytes),
                            start_line,
                            end_line,
                        });
                    }

                    "fenced_code_block" => {
                        let lang_tag = Self::extract_code_block_lang(&child, source_bytes);
                        let name = if lang_tag.is_empty() {
                            "code block".to_string()
                        } else {
                            format!("code block ({})", lang_tag)
                        };
                        let start_line = child.start_position().row + 1;
                        let end_line = child.end_position().row + 1;

                        symbols.push(Symbol {
                            name,
                            kind: SymbolKind::Block,
                            visibility: Visibility::Public,
                            signature: Self::node_text(&child, source_bytes)
                                .lines()
                                .next()
                                .unwrap_or("")
                                .trim()
                                .to_string(),
                            body: Self::full_text(&child, source_bytes),
                            start_line,
                            end_line,
                        });
                    }

                    // Recurse into section nodes and other container nodes
                    _ => {
                        if child.child_count() > 0 {
                            stack.push(child);
                        }
                    }
                }
            }
        }

        ParseResult {
            symbols,
            imports,
            exports,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::language::{SymbolKind, Visibility};

    fn make_parser() -> tree_sitter::Parser {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_markdown_updated::language())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_headings() {
        let source = r#"# Title

Some paragraph text.

## Section One

Content here.

### Subsection
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = MarkdownLanguage;
        let result = lang.extract(source, &tree);

        let headings: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Heading)
            .collect();
        assert!(
            headings.len() >= 3,
            "expected at least 3 headings, got: {:?}",
            headings.iter().map(|h| &h.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_code_blocks() {
        let source = r#"# Example

```rust
fn main() {
    println!("hello");
}
```

```
plain code
```
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = MarkdownLanguage;
        let result = lang.extract(source, &tree);

        let blocks: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Block)
            .collect();
        assert!(
            blocks.len() >= 2,
            "expected at least 2 code blocks, got: {:?}",
            blocks.iter().map(|b| &b.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = MarkdownLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_markdown() {
        let source = r#"# Project README

## Installation

```bash
cargo install cxpak
```

## Usage

Run the tool:

```
cxpak overview
```

### Advanced

See the docs for more details.

## License

MIT
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = MarkdownLanguage;
        let result = lang.extract(source, &tree);

        let headings: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Heading)
            .collect();
        assert!(
            headings.len() >= 4,
            "expected multiple headings, got: {:?}",
            headings.iter().map(|h| &h.name).collect::<Vec<_>>()
        );

        let blocks: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Block)
            .collect();
        assert!(
            blocks.len() >= 2,
            "expected code blocks, got: {:?}",
            blocks.iter().map(|b| &b.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_symbol_kinds() {
        let source = "# Heading\n\n```rust\nlet x = 1;\n```\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = MarkdownLanguage;
        let result = lang.extract(source, &tree);

        let has_heading = result.symbols.iter().any(|s| s.kind == SymbolKind::Heading);
        let has_block = result.symbols.iter().any(|s| s.kind == SymbolKind::Block);
        assert!(has_heading, "expected Heading symbol kind");
        assert!(has_block, "expected Block symbol kind");

        // All should be Public
        assert!(
            result
                .symbols
                .iter()
                .all(|s| s.visibility == Visibility::Public),
            "all Markdown symbols should be public"
        );
    }

    #[test]
    fn test_no_imports() {
        let source = "# Hello\n\nWorld\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = MarkdownLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.imports.is_empty(), "markdown should have no imports");
        assert!(result.exports.is_empty(), "markdown should have no exports");
    }
}
