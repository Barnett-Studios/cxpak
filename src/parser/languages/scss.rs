use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct ScssLanguage;

impl ScssLanguage {
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
        Self::first_line(node, source)
    }

    /// Extract the mixin name from a mixin_statement node.
    fn extract_mixin_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "name" || child.kind() == "identifier" {
                return Self::node_text(&child, source).to_string();
            }
        }
        // Fallback: parse from text
        let text = Self::node_text(node, source);
        let after_mixin = text.trim_start_matches("@mixin").trim();
        after_mixin
            .split(|c: char| c.is_whitespace() || c == '(' || c == '{')
            .next()
            .unwrap_or("")
            .to_string()
    }

    /// Extract SCSS variable name from a declaration node (e.g., `$var: value;`).
    fn extract_variable_name(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_name" || child.kind() == "variable" {
                let name = Self::node_text(&child, source).to_string();
                if name.starts_with('$') {
                    return Some(name);
                }
            }
            // Some SCSS grammars put the variable as property_name
            if child.kind() == "property_name" {
                let name = Self::node_text(&child, source);
                if name.starts_with('$') {
                    return Some(name.to_string());
                }
            }
        }
        // Fallback: check the raw text
        let text = Self::node_text(node, source).trim().to_string();
        if text.starts_with('$') {
            let var_name = text.split(':').next().unwrap_or("").trim().to_string();
            if !var_name.is_empty() {
                return Some(var_name);
            }
        }
        None
    }

    /// Extract @import/@use/@forward as imports.
    fn extract_import(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        let text = Self::node_text(node, source);
        let trimmed = text.trim();

        let (keyword, rest) = if trimmed.starts_with("@import") {
            ("@import", trimmed.trim_start_matches("@import").trim())
        } else if trimmed.starts_with("@use") {
            ("@use", trimmed.trim_start_matches("@use").trim())
        } else if trimmed.starts_with("@forward") {
            ("@forward", trimmed.trim_start_matches("@forward").trim())
        } else {
            return None;
        };

        let path = rest
            .trim_end_matches(';')
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();

        if path.is_empty() {
            return None;
        }

        let _ = keyword;
        Some(Import {
            source: path.clone(),
            names: vec![path],
        })
    }

    /// Extract @include as an import reference.
    fn extract_include(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        let text = Self::node_text(node, source);
        let trimmed = text.trim();
        if !trimmed.starts_with("@include") {
            return None;
        }
        let after_include = trimmed.trim_start_matches("@include").trim();
        let name = after_include
            .split(|c: char| c.is_whitespace() || c == '(' || c == '{' || c == ';')
            .next()
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            return None;
        }
        Some(Import {
            source: String::new(),
            names: vec![name],
        })
    }
}

impl LanguageSupport for ScssLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_scss::language()
    }

    fn name(&self) -> &str {
        "scss"
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
                }

                "mixin_statement" => {
                    let name = Self::extract_mixin_name(&node, source_bytes);
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Mixin,
                        visibility: Visibility::Public,
                        signature: Self::first_line(&node, source_bytes),
                        body: Self::full_text(&node, source_bytes),
                        start_line,
                        end_line,
                    });
                }

                "declaration" => {
                    if let Some(var_name) = Self::extract_variable_name(&node, source_bytes) {
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

                "import_statement" | "use_statement" | "forward_statement" => {
                    if let Some(imp) = Self::extract_import(&node, source_bytes) {
                        imports.push(imp);
                    }
                }

                "include_statement" => {
                    if let Some(imp) = Self::extract_include(&node, source_bytes) {
                        imports.push(imp);
                    }
                }

                "media_statement" | "keyframes_statement" | "supports_statement" | "at_rule" => {
                    let text = Self::node_text(&node, source_bytes);
                    let after_at = text.trim_start_matches('@');
                    let rule_name = after_at
                        .split(|c: char| c.is_whitespace() || c == '{' || c == '(')
                        .next()
                        .unwrap_or("")
                        .to_string();
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
            .set_language(&tree_sitter_scss::language())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_selectors() {
        let source = r#".container {
    width: 100%;
}

#header {
    background: blue;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ScssLanguage;
        let result = lang.extract(source, &tree);

        let selectors: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Selector)
            .collect();
        assert!(
            selectors.len() >= 2,
            "expected at least 2 selectors, got: {:?}",
            selectors.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_mixin() {
        let source = r#"@mixin flex-center {
    display: flex;
    align-items: center;
    justify-content: center;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ScssLanguage;
        let result = lang.extract(source, &tree);

        let mixins: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Mixin)
            .collect();
        assert!(
            !mixins.is_empty(),
            "expected mixin symbol, got: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_variable() {
        let source = r#"$primary: #007bff;
$font-size: 16px;

body {
    color: $primary;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ScssLanguage;
        let result = lang.extract(source, &tree);

        let vars: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .collect();
        assert!(
            !vars.is_empty(),
            "expected SCSS variables, got symbols: {:?}",
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
        let lang = ScssLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
    }

    #[test]
    fn test_symbol_kinds() {
        let source = r#"$color: red;

@mixin btn($bg) {
    background: $bg;
}

.button {
    @include btn($color);
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ScssLanguage;
        let result = lang.extract(source, &tree);

        // Should have at least a Selector
        let has_selector = result
            .symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Selector);
        assert!(has_selector, "expected Selector symbol kind");

        // All should be Public
        assert!(
            result
                .symbols
                .iter()
                .all(|s| s.visibility == Visibility::Public),
            "all SCSS symbols should be public"
        );
    }

    #[test]
    fn test_complex_scss() {
        let source = r#"$base-size: 16px;

@mixin responsive($breakpoint) {
    @media (min-width: $breakpoint) {
        @content;
    }
}

.nav {
    display: flex;

    &__item {
        padding: $base-size;
    }
}

@media (max-width: 768px) {
    .nav {
        flex-direction: column;
    }
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ScssLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.symbols.is_empty(),
            "expected symbols from complex SCSS"
        );
    }
}
