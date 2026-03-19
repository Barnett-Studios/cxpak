use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct JsonLangLanguage;

impl JsonLangLanguage {
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

    /// Extract the key string from a pair node. Returns the unquoted key name.
    fn extract_key_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "string" || child.kind() == "string_content" {
                let text = Self::node_text(&child, source);
                return text.trim_matches('"').to_string();
            }
        }
        String::new()
    }

    /// Extract top-level keys from an object node.
    fn extract_object_keys(node: &tree_sitter::Node, source: &[u8], symbols: &mut Vec<Symbol>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "pair" {
                let name = Self::extract_key_name(&child, source);
                if !name.is_empty() {
                    let start_line = child.start_position().row + 1;
                    let end_line = child.end_position().row + 1;

                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Key,
                        visibility: Visibility::Public,
                        signature: Self::first_line(&child, source),
                        body: Self::full_text(&child, source),
                        start_line,
                        end_line,
                    });
                }
            }
        }
    }
}

impl LanguageSupport for JsonLangLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_json::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "json"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let imports: Vec<Import> = Vec::new();
        let exports: Vec<Export> = Vec::new();

        // The root of a JSON file is typically a `document` containing one value.
        // We want top-level keys if the root value is an object.
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            if node.kind() == "object" {
                Self::extract_object_keys(&node, source_bytes, &mut symbols);
            } else if node.kind() == "array" {
                // For top-level arrays, record the array itself
                let start_line = node.start_position().row + 1;
                let end_line = node.end_position().row + 1;
                symbols.push(Symbol {
                    name: "root".to_string(),
                    kind: SymbolKind::Key,
                    visibility: Visibility::Public,
                    signature: Self::first_line(&node, source_bytes),
                    body: Self::full_text(&node, source_bytes),
                    start_line,
                    end_line,
                });
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
            .set_language(&tree_sitter_json::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_top_level_keys() {
        let source = r#"{
    "name": "my-project",
    "version": "1.0.0",
    "description": "A test project"
}"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JsonLangLanguage;
        let result = lang.extract(source, &tree);

        let keys: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Key)
            .collect();
        assert!(
            keys.len() >= 3,
            "expected at least 3 keys, got: {:?}",
            keys.iter().map(|k| &k.name).collect::<Vec<_>>()
        );
        assert!(keys.iter().any(|k| k.name == "name"));
        assert!(keys.iter().any(|k| k.name == "version"));
        assert!(keys.iter().any(|k| k.name == "description"));
    }

    #[test]
    fn test_extract_nested_object() {
        let source = r#"{
    "dependencies": {
        "lodash": "^4.0.0",
        "express": "^4.18.0"
    },
    "scripts": {
        "build": "tsc",
        "test": "jest"
    }
}"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JsonLangLanguage;
        let result = lang.extract(source, &tree);

        // Should extract top-level keys only
        let keys: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Key)
            .collect();
        assert!(
            keys.len() >= 2,
            "expected at least 2 top-level keys, got: {:?}",
            keys.iter().map(|k| &k.name).collect::<Vec<_>>()
        );
        assert!(keys.iter().any(|k| k.name == "dependencies"));
        assert!(keys.iter().any(|k| k.name == "scripts"));
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = JsonLangLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_json() {
        let source = r#"{
    "name": "@scope/package",
    "version": "2.0.0",
    "main": "index.js",
    "repository": {
        "type": "git",
        "url": "https://github.com/example/repo.git"
    },
    "keywords": ["json", "parser"],
    "license": "MIT"
}"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JsonLangLanguage;
        let result = lang.extract(source, &tree);

        let keys: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Key)
            .collect();
        assert!(
            keys.len() >= 5,
            "expected at least 5 keys, got: {:?}",
            keys.iter().map(|k| &k.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_symbol_kinds() {
        let source = r#"{"key": "value"}"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JsonLangLanguage;
        let result = lang.extract(source, &tree);

        assert!(!result.symbols.is_empty(), "expected at least one symbol");
        assert!(
            result.symbols.iter().all(|s| s.kind == SymbolKind::Key),
            "all JSON symbols should be Key kind"
        );
        assert!(
            result
                .symbols
                .iter()
                .all(|s| s.visibility == Visibility::Public),
            "all JSON symbols should be public"
        );
    }

    #[test]
    fn test_no_imports_exports() {
        let source = r#"{"a": 1}"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JsonLangLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.imports.is_empty(), "json should have no imports");
        assert!(result.exports.is_empty(), "json should have no exports");
    }

    #[test]
    fn test_top_level_array() {
        let source = r#"[1, 2, 3]"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JsonLangLanguage;
        let result = lang.extract(source, &tree);

        // For arrays, we create a single "root" Key
        assert!(
            !result.symbols.is_empty(),
            "expected root symbol for top-level array"
        );
    }
}
