use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct CssLanguage;

impl CssLanguage {
    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        text.lines().next().unwrap_or("").trim().to_string()
    }

    fn full_text(node: &tree_sitter::Node, source: &[u8]) -> String {
        Self::node_text(node, source).to_string()
    }

    /// Extract the selector text from a rule_set node.
    fn extract_selector(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "selectors" || child.kind() == "selector" {
                return Self::node_text(&child, source).trim().to_string();
            }
        }
        // Fallback: use first line
        Self::first_line(node, source)
    }

    /// Extract @import source path.
    fn extract_import(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        let text = Self::node_text(node, source);
        if !text.starts_with("@import") {
            return None;
        }
        // Extract the path from @import url("...") or @import "..."
        let after_import = text.trim_start_matches("@import").trim();
        let path = after_import
            .trim_start_matches("url(")
            .trim_end_matches(')')
            .trim_end_matches(';')
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        if path.is_empty() {
            return None;
        }
        Some(Import {
            source: path.clone(),
            names: vec![path],
        })
    }

    /// Check if a declaration is a CSS custom property (variable).
    fn is_custom_property(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "property_name" {
                let name = Self::node_text(&child, source);
                if name.starts_with("--") {
                    return Some(name.to_string());
                }
            }
        }
        None
    }

    /// Extract the at-rule name (e.g., "media", "keyframes").
    fn extract_at_rule_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        // Get the part after @ and before the first space or brace
        let after_at = text.trim_start_matches('@');
        after_at
            .split(|c: char| c.is_whitespace() || c == '{' || c == '(')
            .next()
            .unwrap_or("")
            .to_string()
    }

    /// Walk a block node looking for custom property declarations.
    fn extract_variables_from_block(
        node: &tree_sitter::Node,
        source: &[u8],
        symbols: &mut Vec<Symbol>,
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "declaration" {
                if let Some(var_name) = Self::is_custom_property(&child, source) {
                    let start_line = child.start_position().row + 1;
                    let end_line = child.end_position().row + 1;
                    symbols.push(Symbol {
                        name: var_name,
                        kind: SymbolKind::Variable,
                        visibility: Visibility::Public,
                        signature: Self::first_line(&child, source),
                        body: Self::full_text(&child, source),
                        start_line,
                        end_line,
                    });
                }
            }
            // Recurse into nested blocks
            if child.kind() == "block" {
                Self::extract_variables_from_block(&child, source, symbols);
            }
        }
    }
}

impl LanguageSupport for CssLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_css::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "css"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let mut imports: Vec<Import> = Vec::new();
        let exports: Vec<Export> = Vec::new();

        let mut cursor = root.walk();

        for node in root.children(&mut cursor) {
            match node.kind() {
                "rule_set" => {
                    let name = Self::extract_selector(&node, source_bytes);
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Selector,
                        visibility: Visibility::Public,
                        signature: Self::first_line(&node, source_bytes),
                        body: Self::full_text(&node, source_bytes),
                        start_line,
                        end_line,
                    });

                    // Check for custom properties inside the rule_set
                    let mut block_cursor = node.walk();
                    for child in node.children(&mut block_cursor) {
                        if child.kind() == "block" {
                            Self::extract_variables_from_block(&child, source_bytes, &mut symbols);
                        }
                    }
                }

                "import_statement" => {
                    if let Some(imp) = Self::extract_import(&node, source_bytes) {
                        imports.push(imp);
                    }
                }

                "media_statement"
                | "keyframes_statement"
                | "supports_statement"
                | "at_rule"
                | "charset_statement"
                | "namespace_statement" => {
                    let rule_name = Self::extract_at_rule_name(&node, source_bytes);
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    symbols.push(Symbol {
                        name: format!("@{}", rule_name),
                        kind: SymbolKind::Rule,
                        visibility: Visibility::Public,
                        signature: Self::first_line(&node, source_bytes),
                        body: Self::full_text(&node, source_bytes),
                        start_line,
                        end_line,
                    });
                }

                "declaration" => {
                    // Top-level custom properties (rare, but possible)
                    if let Some(var_name) = Self::is_custom_property(&node, source_bytes) {
                        let start_line = node.start_position().row + 1;
                        let end_line = node.end_position().row + 1;
                        symbols.push(Symbol {
                            name: var_name,
                            kind: SymbolKind::Variable,
                            visibility: Visibility::Public,
                            signature: Self::first_line(&node, source_bytes),
                            body: Self::full_text(&node, source_bytes),
                            start_line,
                            end_line,
                        });
                    }
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
            .set_language(&tree_sitter_css::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_selectors() {
        let source = r#"body {
    margin: 0;
    padding: 0;
}

.container {
    max-width: 1200px;
}

#header {
    background: blue;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = CssLanguage;
        let result = lang.extract(source, &tree);

        let selectors: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Selector)
            .collect();
        assert!(
            selectors.len() >= 3,
            "expected at least 3 selectors, got: {:?}",
            selectors.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
        assert!(selectors.iter().any(|s| s.name == "body"));
        assert!(selectors.iter().any(|s| s.name == ".container"));
        assert!(selectors.iter().any(|s| s.name == "#header"));
    }

    #[test]
    fn test_extract_media_rule() {
        let source = r#"@media (max-width: 768px) {
    .container {
        width: 100%;
    }
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = CssLanguage;
        let result = lang.extract(source, &tree);

        let rules: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Rule)
            .collect();
        assert!(!rules.is_empty(), "expected media rule");
        assert!(rules[0].name.contains("media"));
    }

    #[test]
    fn test_extract_import() {
        let source = r#"@import "reset.css";
@import url("typography.css");

body {
    font-size: 16px;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = CssLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.imports.is_empty(),
            "expected at least one import, got: {:?}",
            result.imports
        );
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CssLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
    }

    #[test]
    fn test_custom_property_variable() {
        let source = r#":root {
    --primary-color: #007bff;
    --font-size: 16px;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = CssLanguage;
        let result = lang.extract(source, &tree);

        let vars: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .collect();
        assert!(
            vars.len() >= 2,
            "expected at least 2 CSS variables, got: {:?}",
            vars.iter().map(|v| &v.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_symbol_kinds() {
        let source = r#"@keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
}

.btn {
    color: red;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = CssLanguage;
        let result = lang.extract(source, &tree);

        let has_rule = result.symbols.iter().any(|s| s.kind == SymbolKind::Rule);
        let has_selector = result
            .symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Selector);
        assert!(has_rule, "expected Rule symbol kind");
        assert!(has_selector, "expected Selector symbol kind");

        // All should be Public
        assert!(
            result
                .symbols
                .iter()
                .all(|s| s.visibility == Visibility::Public),
            "all CSS symbols should be public"
        );
    }

    #[test]
    fn test_complex_css() {
        let source = r#"@import "base.css";

:root {
    --bg: #fff;
}

body {
    background: var(--bg);
}

@media (min-width: 1024px) {
    .container {
        max-width: 960px;
    }
}

h1, h2, h3 {
    font-weight: bold;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = CssLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.symbols.is_empty(),
            "expected symbols from complex CSS"
        );
        assert!(
            !result.imports.is_empty(),
            "expected import from complex CSS"
        );
    }
}
