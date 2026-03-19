use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct TomlLangLanguage;

impl TomlLangLanguage {
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

    /// Extract the table header name (e.g., `[package]` -> `package`, `[dependencies]` -> `dependencies`).
    fn extract_table_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            // Look for the header parts like `[key]` or `[[key]]`
            if kind.contains("key")
                || kind == "dotted_key"
                || kind == "bare_key"
                || kind == "quoted_key"
            {
                return Self::node_text(&child, source)
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
            }
        }
        // Fallback: extract from the first line
        let line = Self::first_line(node, source);
        line.trim_matches('[').trim_matches(']').trim().to_string()
    }

    /// Extract the key name from a pair node.
    fn extract_key_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            if kind.contains("key")
                || kind == "dotted_key"
                || kind == "bare_key"
                || kind == "quoted_key"
            {
                return Self::node_text(&child, source)
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
            }
        }
        // Fallback: text before `=`
        let text = Self::node_text(node, source);
        text.split('=').next().unwrap_or("").trim().to_string()
    }
}

impl LanguageSupport for TomlLangLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_toml_updated::language()
    }

    fn name(&self) -> &str {
        "toml"
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
                "table" | "table_array_element" => {
                    let name = Self::extract_table_name(&node, source_bytes);
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    symbols.push(Symbol {
                        name: if name.is_empty() {
                            Self::first_line(&node, source_bytes)
                        } else {
                            name
                        },
                        kind: SymbolKind::Table,
                        visibility: Visibility::Public,
                        signature: Self::first_line(&node, source_bytes),
                        body: Self::full_text(&node, source_bytes),
                        start_line,
                        end_line,
                    });
                }

                "pair" => {
                    let name = Self::extract_key_name(&node, source_bytes);
                    if !name.is_empty() {
                        let start_line = node.start_position().row + 1;
                        let end_line = node.end_position().row + 1;

                        symbols.push(Symbol {
                            name,
                            kind: SymbolKind::Key,
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
            .set_language(&tree_sitter_toml_updated::language())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_tables() {
        let source = r#"[package]
name = "my-crate"
version = "1.0.0"

[dependencies]
serde = "1"
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = TomlLangLanguage;
        let result = lang.extract(source, &tree);

        let tables: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Table)
            .collect();
        assert!(
            tables.len() >= 2,
            "expected at least 2 tables, got: {:?}",
            tables.iter().map(|t| &t.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_top_level_keys() {
        let source = r#"name = "my-project"
version = "1.0.0"
edition = "2021"
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = TomlLangLanguage;
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
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TomlLangLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_toml() {
        let source = r#"[package]
name = "cxpak"
version = "0.9.0"
edition = "2021"

[dependencies]
clap = { version = "4", features = ["derive"] }
tree-sitter = "0.25"
serde = { version = "1", features = ["derive"] }

[features]
default = ["lang-rust", "lang-python"]
lang-rust = ["dep:tree-sitter-rust"]
lang-python = ["dep:tree-sitter-python"]

[[bin]]
name = "cxpak"
path = "src/main.rs"
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = TomlLangLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.symbols.is_empty(),
            "expected symbols from complex TOML"
        );

        let tables: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Table)
            .collect();
        assert!(
            tables.len() >= 3,
            "expected at least 3 tables, got: {:?}",
            tables.iter().map(|t| &t.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_symbol_kinds() {
        let source = "[section]\nkey = \"value\"\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = TomlLangLanguage;
        let result = lang.extract(source, &tree);

        let has_table = result.symbols.iter().any(|s| s.kind == SymbolKind::Table);
        assert!(has_table, "expected Table symbol kind");

        assert!(
            result
                .symbols
                .iter()
                .all(|s| s.visibility == Visibility::Public),
            "all TOML symbols should be public"
        );
    }

    #[test]
    fn test_no_imports_exports() {
        let source = "key = \"value\"\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = TomlLangLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.imports.is_empty(), "toml should have no imports");
        assert!(result.exports.is_empty(), "toml should have no exports");
    }

    #[test]
    fn test_table_array() {
        let source = r#"[[bin]]
name = "tool"
path = "src/main.rs"

[[bin]]
name = "helper"
path = "src/helper.rs"
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = TomlLangLanguage;
        let result = lang.extract(source, &tree);

        let tables: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Table)
            .collect();
        assert!(
            tables.len() >= 2,
            "expected at least 2 table array elements, got: {:?}",
            tables.iter().map(|t| &t.name).collect::<Vec<_>>()
        );
    }
}
