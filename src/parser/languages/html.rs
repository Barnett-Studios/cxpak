use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct HtmlLanguage;

/// Tags that map to Section (structural landmarks).
const SECTION_TAGS: &[&str] = &[
    "head", "body", "main", "nav", "header", "footer", "section", "article",
];

impl HtmlLanguage {
    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        text.lines().next().unwrap_or("").trim().to_string()
    }

    /// Extract the tag name from an element node by looking at start_tag or self_closing_tag.
    fn extract_tag_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "start_tag" || child.kind() == "self_closing_tag" {
                let mut inner_cursor = child.walk();
                for inner in child.children(&mut inner_cursor) {
                    if inner.kind() == "tag_name" {
                        return Self::node_text(&inner, source).to_string();
                    }
                }
            }
            if child.kind() == "tag_name" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    /// Determine if a tag name is a section-level tag.
    fn is_section_tag(tag: &str) -> bool {
        SECTION_TAGS.contains(&tag.to_lowercase().as_str())
    }
}

impl LanguageSupport for HtmlLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_html::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "html"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let imports: Vec<Import> = Vec::new();
        let exports: Vec<Export> = Vec::new();

        // Walk all top-level children. The root is typically `document` or `fragment`.
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            Self::extract_element(&node, source_bytes, &mut symbols, true);
        }

        ParseResult {
            symbols,
            imports,
            exports,
        }
    }
}

impl HtmlLanguage {
    /// Recursively extract elements from the HTML tree.
    /// `top_level` indicates whether this node is a direct child of the document root.
    fn extract_element(
        node: &tree_sitter::Node,
        source: &[u8],
        symbols: &mut Vec<Symbol>,
        top_level: bool,
    ) {
        match node.kind() {
            "element" => {
                let tag_name = Self::extract_tag_name(node, source);
                let signature = Self::first_line(node, source);
                let body = Self::node_text(node, source).to_string();
                let start_line = node.start_position().row + 1;
                let end_line = node.end_position().row + 1;

                if Self::is_section_tag(&tag_name) {
                    symbols.push(Symbol {
                        name: tag_name,
                        kind: SymbolKind::Section,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                } else if top_level && !tag_name.is_empty() {
                    symbols.push(Symbol {
                        name: tag_name,
                        kind: SymbolKind::Element,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                // Recurse into children to find nested section tags
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    Self::extract_element(&child, source, symbols, false);
                }
            }

            "script_element" => {
                let signature = Self::first_line(node, source);
                let body = Self::node_text(node, source).to_string();
                let start_line = node.start_position().row + 1;
                let end_line = node.end_position().row + 1;

                symbols.push(Symbol {
                    name: "script".to_string(),
                    kind: SymbolKind::Block,
                    visibility: Visibility::Public,
                    signature,
                    body,
                    start_line,
                    end_line,
                });
            }

            "style_element" => {
                let signature = Self::first_line(node, source);
                let body = Self::node_text(node, source).to_string();
                let start_line = node.start_position().row + 1;
                let end_line = node.end_position().row + 1;

                symbols.push(Symbol {
                    name: "style".to_string(),
                    kind: SymbolKind::Block,
                    visibility: Visibility::Public,
                    signature,
                    body,
                    start_line,
                    end_line,
                });
            }

            _ => {
                // Recurse into other node types (e.g., document, fragment)
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    Self::extract_element(&child, source, symbols, top_level);
                }
            }
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
            .set_language(&tree_sitter_html::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_section_elements() {
        let source = r#"<html>
<head><title>Test</title></head>
<body>
  <header><h1>Header</h1></header>
  <main><p>Content</p></main>
  <footer><p>Footer</p></footer>
</body>
</html>
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = HtmlLanguage;
        let result = lang.extract(source, &tree);

        let sections: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Section)
            .collect();
        assert!(
            sections.len() >= 3,
            "expected at least head, body, header/main/footer sections, got: {:?}",
            sections.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
        assert_eq!(sections[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_script_and_style() {
        let source = r#"<html>
<head>
  <script>console.log("hello");</script>
  <style>body { margin: 0; }</style>
</head>
<body></body>
</html>
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = HtmlLanguage;
        let result = lang.extract(source, &tree);

        let blocks: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Block)
            .collect();
        assert!(
            blocks.len() >= 2,
            "expected script and style blocks, got: {:?}",
            blocks.iter().map(|b| &b.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = HtmlLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_html() {
        let source = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <title>My Page</title>
  <style>
    body { font-family: sans-serif; }
  </style>
</head>
<body>
  <nav>
    <a href="/">Home</a>
    <a href="/about">About</a>
  </nav>
  <main>
    <article>
      <h1>Welcome</h1>
      <p>Content goes here.</p>
    </article>
  </main>
  <footer>
    <p>Copyright 2025</p>
  </footer>
  <script>
    document.addEventListener("DOMContentLoaded", function() {
      console.log("loaded");
    });
  </script>
</body>
</html>
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = HtmlLanguage;
        let result = lang.extract(source, &tree);

        let sections: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Section)
            .collect();
        assert!(
            sections.len() >= 4,
            "expected head, body, nav, main, article, footer sections, got: {:?}",
            sections.iter().map(|s| &s.name).collect::<Vec<_>>()
        );

        let blocks: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Block)
            .collect();
        assert!(
            blocks.len() >= 2,
            "expected script and style blocks, got: {:?}",
            blocks.iter().map(|b| &b.name).collect::<Vec<_>>()
        );
    }
}
