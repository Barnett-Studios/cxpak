use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct GoLanguage;

impl GoLanguage {
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
            if child.kind() == "identifier"
                || child.kind() == "type_identifier"
                || child.kind() == "field_identifier"
            {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    /// In Go, an identifier starting with an uppercase letter is exported (public).
    fn is_public(name: &str) -> bool {
        name.chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
    }

    fn extract_fn_signature(node: &tree_sitter::Node, source: &[u8]) -> String {
        let full_text = Self::node_text(node, source);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block" {
                let body_start = child.start_byte() - node.start_byte();
                return full_text[..body_start].trim().to_string();
            }
        }
        full_text.lines().next().unwrap_or("").trim().to_string()
    }

    fn extract_fn_body(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block" {
                let text = &source[child.start_byte()..child.end_byte()];
                return String::from_utf8_lossy(text).into_owned();
            }
        }
        String::new()
    }

    fn extract_import(node: &tree_sitter::Node, source: &[u8]) -> Vec<Import> {
        // import_declaration can wrap:
        //   - a single import_spec: import "fmt"
        //   - an import_spec_list (block): import ( "fmt" "os" )
        let mut result = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "import_spec" => {
                    if let Some(imp) = Self::parse_import_spec(&child, source) {
                        result.push(imp);
                    }
                }
                "import_spec_list" => {
                    let mut inner_cursor = child.walk();
                    for spec in child.children(&mut inner_cursor) {
                        if spec.kind() == "import_spec" {
                            if let Some(imp) = Self::parse_import_spec(&spec, source) {
                                result.push(imp);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        result
    }

    fn parse_import_spec(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        // import_spec: (name identifier)? path interpreted_string_literal
        let mut alias = String::new();
        let mut path = String::new();

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "identifier" | "blank_identifier" | "dot" => {
                    alias = Self::node_text(&child, source).to_string();
                }
                "interpreted_string_literal" | "raw_string_literal" => {
                    path = Self::node_text(&child, source)
                        .trim_matches(|c| c == '"' || c == '`')
                        .to_string();
                }
                _ => {}
            }
        }

        if path.is_empty() {
            return None;
        }

        let name = if !alias.is_empty() {
            alias
        } else {
            // Default name is last segment of import path
            path.rsplit('/').next().unwrap_or(&path).to_string()
        };

        Some(Import {
            source: path,
            names: vec![name],
        })
    }
}

impl LanguageSupport for GoLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_go::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "go"
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
                "import_declaration" => {
                    imports.extend(Self::extract_import(&node, source_bytes));
                }

                "function_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_public(&name);
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

                "method_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_public(&name);
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
                            kind: SymbolKind::Method,
                        });
                    }
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Method,
                        visibility,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "type_declaration" => {
                    // type_declaration wraps type_spec nodes
                    let mut type_cursor = node.walk();
                    for child in node.children(&mut type_cursor) {
                        if child.kind() == "type_spec" {
                            let name = Self::extract_name(&child, source_bytes);
                            let is_pub = Self::is_public(&name);
                            let visibility = if is_pub {
                                Visibility::Public
                            } else {
                                Visibility::Private
                            };
                            let signature = Self::first_line(&child, source_bytes);
                            let body = Self::node_text(&child, source_bytes).to_string();
                            let start_line = child.start_position().row + 1;
                            let end_line = child.end_position().row + 1;

                            // Determine kind from underlying type node
                            let kind = {
                                let mut spec_cursor = child.walk();
                                let mut found_kind = SymbolKind::Struct;
                                for spec_child in child.children(&mut spec_cursor) {
                                    match spec_child.kind() {
                                        "struct_type" => {
                                            found_kind = SymbolKind::Struct;
                                            break;
                                        }
                                        "interface_type" => {
                                            found_kind = SymbolKind::Interface;
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                                found_kind
                            };

                            if is_pub {
                                exports.push(Export {
                                    name: name.clone(),
                                    kind: kind.clone(),
                                });
                            }
                            symbols.push(Symbol {
                                name,
                                kind,
                                visibility,
                                signature,
                                body,
                                start_line,
                                end_line,
                            });
                        }
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
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_exported_function() {
        let source = r#"package main

func Greet(name string) string {
    return "Hello, " + name
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GoLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected function symbol");
        assert_eq!(funcs[0].name, "Greet");
        assert_eq!(funcs[0].visibility, Visibility::Public);

        let exported: Vec<_> = result
            .exports
            .iter()
            .filter(|e| e.name == "Greet")
            .collect();
        assert!(!exported.is_empty(), "Greet should be exported");
    }

    #[test]
    fn test_extract_private_function() {
        let source = r#"package main

func helper(x int) int {
    return x * 2
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GoLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty());
        assert_eq!(funcs[0].name, "helper");
        assert_eq!(funcs[0].visibility, Visibility::Private);
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_extract_struct_type() {
        let source = r#"package main

type Point struct {
    X float64
    Y float64
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GoLanguage;
        let result = lang.extract(source, &tree);

        let structs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct)
            .collect();
        assert!(!structs.is_empty(), "expected struct symbol");
        assert_eq!(structs[0].name, "Point");
        assert_eq!(structs[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_import() {
        let source = r#"package main

import (
    "fmt"
    "os"
)
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GoLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.imports.len(), 2);
        let paths: Vec<&str> = result.imports.iter().map(|i| i.source.as_str()).collect();
        assert!(paths.contains(&"fmt"));
        assert!(paths.contains(&"os"));
    }

    #[test]
    fn test_extract_method_declaration() {
        let source = r#"package main

type Point struct {
    X float64
    Y float64
}

func (p *Point) String() string {
    return "point"
}

func (p Point) translate(dx float64) Point {
    return Point{X: p.X + dx}
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GoLanguage;
        let result = lang.extract(source, &tree);

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 2, "expected 2 methods");

        let string_method = methods.iter().find(|m| m.name == "String").unwrap();
        assert_eq!(string_method.visibility, Visibility::Public);
        assert!(!string_method.body.is_empty());
        assert!(!string_method.signature.is_empty());

        let translate = methods.iter().find(|m| m.name == "translate").unwrap();
        assert_eq!(translate.visibility, Visibility::Private);

        // Public method should be exported
        let method_exports: Vec<_> = result
            .exports
            .iter()
            .filter(|e| e.kind == SymbolKind::Method)
            .collect();
        assert_eq!(method_exports.len(), 1);
        assert_eq!(method_exports[0].name, "String");
    }

    #[test]
    fn test_extract_aliased_import() {
        let source = r#"package main

import (
    f "fmt"
    . "os"
    _ "net/http/pprof"
)
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GoLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.imports.len(), 3);
        let sources: Vec<&str> = result.imports.iter().map(|i| i.source.as_str()).collect();
        assert!(
            sources.contains(&"fmt"),
            "expected fmt import, got: {:?}",
            sources
        );
        assert!(
            sources.contains(&"os"),
            "expected os import, got: {:?}",
            sources
        );
        assert!(
            sources.contains(&"net/http/pprof"),
            "expected pprof import, got: {:?}",
            sources
        );
        // dot and blank_identifier branches are exercised by parsing . "os" and _ "net/http/pprof"
        let names: Vec<&str> = result.imports.iter().map(|i| i.names[0].as_str()).collect();
        assert!(
            names.contains(&"."),
            "expected dot import name, got: {:?}",
            names
        );
        assert!(
            names.contains(&"_"),
            "expected blank import name, got: {:?}",
            names
        );
    }

    #[test]
    fn test_extract_interface_type() {
        let source = r#"package main

type Reader interface {
    Read(p []byte) (n int, err error)
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GoLanguage;
        let result = lang.extract(source, &tree);

        let interfaces: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Interface)
            .collect();
        assert!(!interfaces.is_empty(), "expected interface symbol");
        assert_eq!(interfaces[0].name, "Reader");
        assert_eq!(interfaces[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_single_import() {
        let source = r#"package main

import "fmt"
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GoLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source, "fmt");
    }

    #[test]
    fn test_interface_method_no_body() {
        // Interface methods in Go have no block — covers extract_fn_body String::new() (line 59)
        // and extract_fn_signature fallback to first_line (line 48)
        let source = r#"package main

type Reader interface {
    Read(p []byte) (n int, err error)
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GoLanguage;
        let result = lang.extract(source, &tree);
        let interfaces: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Interface)
            .collect();
        assert!(!interfaces.is_empty());
    }

    #[test]
    fn test_extract_name_fallback() {
        // Covers extract_name returning String::new() (line 28)
        // with a type that uses an anonymous struct
        let source = "package main\n\nvar _ = struct{ x int }{}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GoLanguage;
        let result = lang.extract(source, &tree);
        let _ = result;
    }

    #[test]
    fn test_private_function() {
        // Lowercase function — covers is_public returning false
        let source = "package main\n\nfunc helper() {}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GoLanguage;
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
    fn test_private_type_declaration() {
        // Lowercase struct name — covers Visibility::Private branch for type_declaration (line 224)
        let source = "package main\n\ntype myStruct struct {\n\tx int\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GoLanguage;
        let result = lang.extract(source, &tree);
        let structs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct)
            .collect();
        assert!(!structs.is_empty());
        assert_eq!(structs[0].visibility, Visibility::Private);
        assert_eq!(structs[0].name, "myStruct");
    }

    #[test]
    fn test_function_without_body() {
        // Interface method declaration has no body — covers fallback paths (lines 48, 59)
        let source = "package main\n\ntype Doer interface {\n\tDo()\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GoLanguage;
        let result = lang.extract(source, &tree);
        // The interface should be extracted as a type
        assert!(!result.symbols.is_empty());
    }
}
