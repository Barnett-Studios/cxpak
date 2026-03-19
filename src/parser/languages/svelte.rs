use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct SvelteLanguage;

impl SvelteLanguage {
    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        text.lines().next().unwrap_or("").trim().to_string()
    }

    /// Extract the tag name from an element node.
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
}

impl LanguageSupport for SvelteLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_svelte_ng::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "svelte"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let imports: Vec<Import> = Vec::new();
        let exports: Vec<Export> = Vec::new();

        let mut cursor = root.walk();

        for node in root.children(&mut cursor) {
            match node.kind() {
                "script_element" => {
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
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
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
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

                "element" => {
                    let tag_name = Self::extract_tag_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    let name = if tag_name.is_empty() {
                        "element".to_string()
                    } else {
                        tag_name
                    };

                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Section,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                _ => {}
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
            .set_language(&tree_sitter_svelte_ng::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_script_and_style() {
        let source = r#"<script>
  let count = 0;
</script>

<style>
  h1 { color: red; }
</style>
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = SvelteLanguage;
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
        assert!(blocks.iter().any(|b| b.name == "script"));
        assert!(blocks.iter().any(|b| b.name == "style"));
        assert_eq!(blocks[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_top_level_elements() {
        let source = r#"<div>
  <p>Hello</p>
</div>
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = SvelteLanguage;
        let result = lang.extract(source, &tree);

        let sections: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Section)
            .collect();
        assert!(
            !sections.is_empty(),
            "expected top-level element sections, got symbols: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = SvelteLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_component() {
        let source = r#"<script>
  export let name = 'world';

  function handleClick() {
    alert(`Hello ${name}!`);
  }
</script>

<main>
  <h1>Hello {name}!</h1>
  <button on:click={handleClick}>
    Click me
  </button>
</main>

<style>
  main {
    text-align: center;
    padding: 1em;
  }
</style>
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = SvelteLanguage;
        let result = lang.extract(source, &tree);

        // Should have script block, style block, and at least one element section
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

        let sections: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Section)
            .collect();
        assert!(
            !sections.is_empty(),
            "expected at least one section (main element)"
        );
    }
}
