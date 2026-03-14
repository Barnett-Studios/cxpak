use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct JavaScriptLanguage;

impl JavaScriptLanguage {
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
            if child.kind() == "identifier" || child.kind() == "property_identifier" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    fn is_exported(node: &tree_sitter::Node) -> bool {
        if let Some(parent) = node.parent() {
            if parent.kind() == "export_statement" {
                return true;
            }
        }
        false
    }

    fn extract_fn_signature(node: &tree_sitter::Node, source: &[u8]) -> String {
        let full_text = Self::node_text(node, source);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "statement_block" {
                let body_start = child.start_byte() - node.start_byte();
                return full_text[..body_start].trim().to_string();
            }
        }
        full_text.lines().next().unwrap_or("").trim().to_string()
    }

    fn extract_fn_body(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "statement_block" {
                let text = &source[child.start_byte()..child.end_byte()];
                return String::from_utf8_lossy(text).into_owned();
            }
        }
        String::new()
    }

    fn extract_import(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        let text = Self::node_text(node, source);
        let source_path = if let Some(from_idx) = text.rfind(" from ") {
            text[from_idx + 6..]
                .trim()
                .trim_matches(|c| c == '\'' || c == '"' || c == ';')
                .to_string()
        } else {
            String::new()
        };

        let names = if let Some(brace_start) = text.find('{') {
            if let Some(brace_end) = text.find('}') {
                text[brace_start + 1..brace_end]
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            } else {
                vec![]
            }
        } else if text.contains("* as ") {
            vec!["*".to_string()]
        } else {
            let after_import = text.trim_start_matches("import").trim();
            let name = after_import
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if name.is_empty() || name == "from" {
                vec![]
            } else {
                vec![name]
            }
        };

        if source_path.is_empty() && names.is_empty() {
            None
        } else {
            Some(Import {
                source: source_path,
                names,
            })
        }
    }
}

impl LanguageSupport for JavaScriptLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_javascript::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "javascript"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let mut imports: Vec<Import> = Vec::new();
        let mut exports: Vec<Export> = Vec::new();

        let mut stack: Vec<tree_sitter::Node> = root.children(&mut root.walk()).collect();

        while let Some(node) = stack.pop() {
            match node.kind() {
                "import_statement" => {
                    if let Some(import) = Self::extract_import(&node, source_bytes) {
                        imports.push(import);
                    }
                }

                "export_statement" => {
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        stack.push(child);
                    }
                }

                "function_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_exported(&node);
                    let visibility = if is_pub {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };
                    let signature = Self::extract_fn_signature(&node, source_bytes);
                    let body = Self::extract_fn_body(&node, source_bytes);
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if is_pub {
                        exports.push(Export {
                            name: name.clone(),
                            kind: SymbolKind::Function,
                        });
                    }
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Function,
                        visibility,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "class_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_exported(&node);
                    let visibility = if is_pub {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if is_pub {
                        exports.push(Export {
                            name: name.clone(),
                            kind: SymbolKind::Class,
                        });
                    }
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Class,
                        visibility,
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
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_exported_function() {
        let source = r#"export function greet(name) {
    return `Hello, ${name}!`;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JavaScriptLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected at least one function");
        assert_eq!(funcs[0].name, "greet");
        assert_eq!(funcs[0].visibility, Visibility::Public);

        let exported: Vec<_> = result
            .exports
            .iter()
            .filter(|e| e.name == "greet")
            .collect();
        assert!(!exported.is_empty(), "greet should be exported");
    }

    #[test]
    fn test_extract_private_function() {
        let source = r#"function helper(x) {
    return x * 2;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JavaScriptLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.symbols.len(), 1);
        let sym = &result.symbols[0];
        assert_eq!(sym.name, "helper");
        assert_eq!(sym.visibility, Visibility::Private);
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_extract_import() {
        let source = r#"import { readFile, writeFile } from 'fs';
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JavaScriptLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.imports.len(), 1);
        let imp = &result.imports[0];
        assert_eq!(imp.source, "fs");
        assert!(imp.names.contains(&"readFile".to_string()));
        assert!(imp.names.contains(&"writeFile".to_string()));
    }

    #[test]
    fn test_extract_class() {
        let source = r#"export class Dog {
    constructor(name) {
        this.name = name;
    }
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JavaScriptLanguage;
        let result = lang.extract(source, &tree);

        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert!(!classes.is_empty(), "expected class symbol");
        assert_eq!(classes[0].name, "Dog");
        assert_eq!(classes[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_namespace_import() {
        let source = r#"import * as fs from 'fs';
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JavaScriptLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source, "fs");
        assert!(
            result.imports[0].names.contains(&"*".to_string()),
            "expected wildcard import, got: {:?}",
            result.imports[0].names
        );
    }

    #[test]
    fn test_extract_default_import() {
        let source = r#"import React from 'react';
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JavaScriptLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source, "react");
        assert!(
            result.imports[0].names.contains(&"React".to_string()),
            "expected default import 'React', got: {:?}",
            result.imports[0].names
        );
    }

    #[test]
    fn test_extract_function_signature_and_body() {
        let source = r#"function add(a, b) {
    return a + b;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JavaScriptLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty());
        assert!(!funcs[0].signature.is_empty(), "expected signature");
        assert!(!funcs[0].body.is_empty(), "expected body");
    }

    #[test]
    fn test_wildcard_import() {
        // Covers "* as " branch in extract_import (line 81-82)
        let source = r#"import * as utils from './utils';
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JavaScriptLanguage;
        let result = lang.extract(source, &tree);
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].names, vec!["*".to_string()]);
    }

    #[test]
    fn test_default_import() {
        // Covers default import name extraction (lines 84-93)
        let source = r#"import React from 'react';
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JavaScriptLanguage;
        let result = lang.extract(source, &tree);
        assert_eq!(result.imports.len(), 1);
    }

    #[test]
    fn test_import_without_from() {
        // Side-effect import — covers empty source_path (line 68)
        let source = r#"import './polyfill';
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JavaScriptLanguage;
        let result = lang.extract(source, &tree);
        // May produce import with empty source or names depending on parser
        let _ = result;
    }

    #[test]
    fn test_private_function_body_and_sig() {
        // Non-exported function — covers extract_fn_signature statement_block path
        // and extract_fn_body returning the block content
        let source = r#"function helper() {
    return 42;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JavaScriptLanguage;
        let result = lang.extract(source, &tree);
        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty());
        assert_eq!(funcs[0].visibility, Visibility::Private);
    }

    #[test]
    fn test_class_with_method() {
        // Covers class extraction with methods
        let source = r#"export class Calculator {
    add(a, b) {
        return a + b;
    }
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JavaScriptLanguage;
        let result = lang.extract(source, &tree);
        assert!(!result.symbols.is_empty());
    }

    #[test]
    fn test_private_class_declaration() {
        // Non-exported class → Visibility::Private (covers line 178)
        let source = "class InternalHelper {\n  constructor() {}\n  help() {}\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JavaScriptLanguage;
        let result = lang.extract(source, &tree);
        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert!(!classes.is_empty());
        assert_eq!(classes[0].visibility, Visibility::Private);
    }
}
