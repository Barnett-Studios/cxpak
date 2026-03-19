use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct BashLanguage;

impl BashLanguage {
    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        text.lines().next().unwrap_or("").trim().to_string()
    }

    fn extract_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "word" || child.kind() == "identifier" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    fn extract_fn_body(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "compound_statement" {
                let text = &source[child.start_byte()..child.end_byte()];
                return String::from_utf8_lossy(text).into_owned();
            }
        }
        String::new()
    }

    /// Extract the variable name from a variable_assignment node.
    fn extract_variable_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_name" {
                return Self::node_text(&child, source).to_string();
            }
        }
        // Fallback: try to parse from the text (e.g., "FOO=bar")
        let text = Self::node_text(node, source);
        if let Some(idx) = text.find('=') {
            return text[..idx].trim().to_string();
        }
        String::new()
    }

    /// Extract `source` commands as imports.
    fn extract_source_import(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        // A command node whose first word is "source" or "."
        let mut cursor = node.walk();
        let mut is_source = false;
        let mut path = String::new();

        for child in node.children(&mut cursor) {
            let text = Self::node_text(&child, source);
            if child.kind() == "command_name" {
                let cmd = text.trim();
                if cmd == "source" || cmd == "." {
                    is_source = true;
                } else {
                    return None;
                }
            } else if is_source
                && (child.kind() == "word"
                    || child.kind() == "string"
                    || child.kind() == "raw_string")
            {
                path = text.trim_matches('"').trim_matches('\'').to_string();
            }
        }

        if is_source && !path.is_empty() {
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            Some(Import {
                source: path,
                names: vec![name],
            })
        } else {
            None
        }
    }
}

impl LanguageSupport for BashLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_bash::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "bash"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let mut imports: Vec<Import> = Vec::new();
        let mut exports: Vec<Export> = Vec::new();

        let mut cursor = root.walk();

        for node in root.children(&mut cursor) {
            match node.kind() {
                "function_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::extract_fn_body(&node, source_bytes);
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    exports.push(Export {
                        name: name.clone(),
                        kind: SymbolKind::Function,
                    });
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "variable_assignment" => {
                    let name = Self::extract_variable_name(&node, source_bytes);
                    if !name.is_empty() {
                        let signature = Self::first_line(&node, source_bytes);
                        let body = String::new();
                        let start_line = node.start_position().row + 1;
                        let end_line = node.end_position().row + 1;

                        exports.push(Export {
                            name: name.clone(),
                            kind: SymbolKind::Variable,
                        });
                        symbols.push(Symbol {
                            name,
                            kind: SymbolKind::Variable,
                            visibility: Visibility::Public,
                            signature,
                            body,
                            start_line,
                            end_line,
                        });
                    }
                }

                "command" => {
                    if let Some(imp) = Self::extract_source_import(&node, source_bytes) {
                        imports.push(imp);
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
            .set_language(&tree_sitter_bash::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_function() {
        let source = r#"greet() {
    echo "Hello, $1!"
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = BashLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected function symbol");
        assert_eq!(funcs[0].name, "greet");
        assert_eq!(funcs[0].visibility, Visibility::Public);

        assert!(
            result.exports.iter().any(|e| e.name == "greet"),
            "greet should be exported"
        );
    }

    #[test]
    fn test_extract_variable() {
        let source = r#"MY_VAR="hello"
COUNT=42
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = BashLanguage;
        let result = lang.extract(source, &tree);

        let vars: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .collect();
        assert!(
            vars.len() >= 2,
            "expected at least 2 variables, got: {:?}",
            vars.iter().map(|v| &v.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_source_import() {
        let source = "source /etc/profile\nsource ./helpers.sh\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = BashLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.imports.is_empty(),
            "expected source imports, got: {:?}",
            result.imports
        );
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = BashLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
    }

    #[test]
    fn test_complex_script() {
        let source = r#"#!/bin/bash

VERSION="1.0.0"

setup() {
    mkdir -p /tmp/app
    echo "Setting up..."
}

cleanup() {
    rm -rf /tmp/app
}

source ./config.sh

main() {
    setup
    echo "Running version $VERSION"
    cleanup
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = BashLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(
            funcs.len() >= 3,
            "expected at least 3 functions, got: {:?}",
            funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
        );

        let vars: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .collect();
        assert!(!vars.is_empty(), "expected VERSION variable");

        assert!(!result.imports.is_empty(), "expected source import");
    }

    #[test]
    fn test_function_with_keyword() {
        let source = r#"function deploy() {
    echo "deploying..."
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = BashLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(
            !funcs.is_empty(),
            "expected function with 'function' keyword"
        );
        assert_eq!(funcs[0].name, "deploy");
    }
}
