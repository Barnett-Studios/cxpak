use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct TypeScriptLanguage;

impl TypeScriptLanguage {
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
                || child.kind() == "property_identifier"
            {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    /// Determine visibility: exported items are Public.
    fn is_exported(node: &tree_sitter::Node, _source: &[u8]) -> bool {
        // Check for "export" keyword as a sibling or parent context.
        // In the tree-sitter TS grammar, exported declarations appear inside
        // an `export_statement` node, so we look at the parent.
        if let Some(parent) = node.parent() {
            if parent.kind() == "export_statement" {
                return true;
            }
        }
        // Also handle direct children markers
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "export" {
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
        // import_statement: "import" import_clause "from" string
        // import_clause can be: namespace_import, named_imports, identifier, ...
        let text = Self::node_text(node, source);
        // Extract the module path (after "from")
        let source_path = if let Some(from_idx) = text.rfind(" from ") {
            text[from_idx + 6..]
                .trim()
                .trim_matches(|c| c == '\'' || c == '"' || c == ';')
                .to_string()
        } else {
            String::new()
        };

        // Extract names: look for { Name, Name } pattern
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
            // "import defaultExport from 'module'"
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

impl LanguageSupport for TypeScriptLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    }

    fn name(&self) -> &str {
        "typescript"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let mut imports: Vec<Import> = Vec::new();
        let mut exports: Vec<Export> = Vec::new();

        // Walk all nodes in the tree to catch top-level and export-wrapped declarations
        let mut stack: Vec<tree_sitter::Node> = root.children(&mut root.walk()).collect();

        while let Some(node) = stack.pop() {
            match node.kind() {
                "import_statement" => {
                    if let Some(import) = Self::extract_import(&node, source_bytes) {
                        imports.push(import);
                    }
                }

                "export_statement" => {
                    // Push children so we handle the exported declaration
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        stack.push(child);
                    }
                }

                "function_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_exported(&node, source_bytes);
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
                    let is_pub = Self::is_exported(&node, source_bytes);
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

                "interface_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_exported(&node, source_bytes);
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
                            kind: SymbolKind::Interface,
                        });
                    }
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Interface,
                        visibility,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "type_alias_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_exported(&node, source_bytes);
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
                            kind: SymbolKind::TypeAlias,
                        });
                    }
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::TypeAlias,
                        visibility,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "enum_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_exported(&node, source_bytes);
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
                            kind: SymbolKind::Enum,
                        });
                    }
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Enum,
                        visibility,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                // Arrow functions and function expressions assigned to variables:
                // const handler = (e) => { ... }
                // const add = function(a, b) { ... }
                "lexical_declaration" | "variable_declaration" => {
                    let mut decl_cursor = node.walk();
                    for child in node.children(&mut decl_cursor) {
                        if child.kind() == "variable_declarator" {
                            let value_is_fn = {
                                let mut vc = child.walk();
                                let x = child.children(&mut vc).any(|c| {
                                    c.kind() == "arrow_function"
                                        || c.kind() == "function_expression"
                                });
                                x
                            };
                            if value_is_fn {
                                let name = Self::extract_name(&child, source_bytes);
                                if name.is_empty() {
                                    continue;
                                }
                                let is_pub = Self::is_exported(&node, source_bytes);
                                let visibility = if is_pub {
                                    Visibility::Public
                                } else {
                                    Visibility::Private
                                };
                                let signature = Self::first_line(&child, source_bytes);
                                let body = Self::node_text(&child, source_bytes).to_string();
                                let start_line = child.start_position().row + 1;
                                let end_line = child.end_position().row + 1;

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
                        }
                    }
                }

                // Re-exports: export { A, B } from 'module'
                "export_clause" => {
                    // export_clause is a child of export_statement; collect named re-exports
                    let mut ec_cursor = node.walk();
                    for child in node.children(&mut ec_cursor) {
                        if child.kind() == "export_specifier" {
                            let name = Self::extract_name(&child, source_bytes);
                            if !name.is_empty() {
                                exports.push(Export {
                                    name: name.clone(),
                                    kind: SymbolKind::Variable,
                                });
                                symbols.push(Symbol {
                                    name,
                                    kind: SymbolKind::Variable,
                                    visibility: Visibility::Public,
                                    signature: Self::node_text(&child, source_bytes).to_string(),
                                    body: String::new(),
                                    start_line: child.start_position().row + 1,
                                    end_line: child.end_position().row + 1,
                                });
                            }
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
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_exported_function() {
        let source = r#"export function greet(name: string): string {
    return `Hello, ${name}!`;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected at least one function symbol");
        let sym = funcs[0];
        assert_eq!(sym.name, "greet");
        assert_eq!(sym.visibility, Visibility::Public);

        let exported: Vec<_> = result
            .exports
            .iter()
            .filter(|e| e.name == "greet")
            .collect();
        assert!(!exported.is_empty(), "greet should be exported");
    }

    #[test]
    fn test_extract_interface() {
        let source = r#"export interface Animal {
    name: string;
    speak(): void;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);

        let interfaces: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Interface)
            .collect();
        assert!(!interfaces.is_empty(), "expected interface symbol");
        assert_eq!(interfaces[0].name, "Animal");
        assert_eq!(interfaces[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_import() {
        let source = r#"import { readFile, writeFile } from 'fs';
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.imports.len(), 1);
        let imp = &result.imports[0];
        assert_eq!(imp.source, "fs");
        assert!(imp.names.contains(&"readFile".to_string()));
        assert!(imp.names.contains(&"writeFile".to_string()));
    }

    #[test]
    fn test_extract_type_alias() {
        let source = r#"export type UserId = string;
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);

        let aliases: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::TypeAlias)
            .collect();
        assert!(!aliases.is_empty(), "expected type alias symbol");
        assert_eq!(aliases[0].name, "UserId");
    }

    #[test]
    fn test_extract_class() {
        let source = "export class Animal {\n    name: string;\n    constructor(name: string) { this.name = name; }\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);
        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert!(!classes.is_empty(), "expected class symbol");
        assert_eq!(classes[0].name, "Animal");
        assert_eq!(classes[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_enum() {
        let source = "export enum Direction {\n    Up,\n    Down,\n    Left,\n    Right\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);
        let enums: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Enum)
            .collect();
        assert!(!enums.is_empty(), "expected enum symbol");
        assert_eq!(enums[0].name, "Direction");
        assert_eq!(enums[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_private_function() {
        let source = "function helper(): void {}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);
        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty());
        assert_eq!(funcs[0].name, "helper");
        assert_eq!(funcs[0].visibility, Visibility::Private);
        assert!(
            result.exports.is_empty(),
            "non-exported function should not be in exports"
        );
    }

    #[test]
    fn test_extract_private_interface() {
        let source = "interface InternalConfig {\n    debug: boolean;\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);
        let interfaces: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Interface)
            .collect();
        assert!(!interfaces.is_empty());
        assert_eq!(interfaces[0].visibility, Visibility::Private);
    }

    #[test]
    fn test_extract_default_import() {
        let source = "import React from 'react';\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source, "react");
    }

    #[test]
    fn test_extract_namespace_import() {
        let source = "import * as fs from 'fs';\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source, "fs");
        assert!(result.imports[0].names.contains(&"*".to_string()));
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_extract_union_type() {
        let source = "export type StringOrNumber = string | number;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);
        let types: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::TypeAlias)
            .collect();
        assert!(!types.is_empty());
        assert_eq!(types[0].name, "StringOrNumber");
        assert_eq!(types[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_import_without_closing_brace() {
        // Malformed import with no closing brace — should produce empty names
        let source = "import { Foo from 'bar';\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);
        // Malformed imports may or may not parse; just ensure no panic
        let _ = result.imports;
    }

    #[test]
    fn test_private_class() {
        // Non-exported class should be private
        let source = "class InternalHelper {\n    method() {}\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);

        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert!(!classes.is_empty(), "expected class symbol");
        assert_eq!(classes[0].name, "InternalHelper");
        assert_eq!(classes[0].visibility, Visibility::Private);
        assert!(
            result.exports.is_empty(),
            "private class should not be exported"
        );
    }

    #[test]
    fn test_private_enum() {
        let source = "enum Color { Red, Green, Blue }\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);

        let enums: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Enum)
            .collect();
        assert!(!enums.is_empty(), "expected enum symbol");
        assert_eq!(enums[0].visibility, Visibility::Private);
    }

    #[test]
    fn test_private_type_alias() {
        let source = "type MyType = string | number;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);

        let types: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::TypeAlias)
            .collect();
        assert!(!types.is_empty(), "expected type alias");
        assert_eq!(types[0].name, "MyType");
        assert_eq!(types[0].visibility, Visibility::Private);
    }

    #[test]
    fn test_function_without_body() {
        // A function declaration without a body (abstract-like)
        let source = "export function noBody(): void;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);
        // Just ensure it doesn't panic; may or may not produce a symbol
        let _ = result.symbols;
    }

    #[test]
    fn test_extract_multiple_exports() {
        let source = "export function foo(): void {}\nexport function bar(): void {}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);
        assert_eq!(result.symbols.len(), 2);
        assert_eq!(result.exports.len(), 2);
    }

    #[test]
    fn test_arrow_function_extracted() {
        let source = "const handler = (e: Event): void => { console.log(e); };\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1, "expected arrow function symbol");
        assert_eq!(funcs[0].name, "handler");
        assert_eq!(funcs[0].visibility, Visibility::Private);
    }

    #[test]
    fn test_exported_arrow_function_extracted() {
        let source = "export const add = (a: number, b: number): number => a + b;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1, "expected exported arrow function");
        assert_eq!(funcs[0].name, "add");
        assert_eq!(funcs[0].visibility, Visibility::Public);
        assert!(result.exports.iter().any(|e| e.name == "add"));
    }

    #[test]
    fn test_function_expression_extracted() {
        let source = "const multiply = function(a: number, b: number): number { return a * b; };\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1, "expected function expression symbol");
        assert_eq!(funcs[0].name, "multiply");
    }

    #[test]
    fn test_re_exports_captured() {
        let source = "export { Foo, Bar } from './module';\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = TypeScriptLanguage;
        let result = lang.extract(source, &tree);

        let export_names: Vec<&str> = result.exports.iter().map(|e| e.name.as_str()).collect();
        assert!(
            export_names.contains(&"Foo"),
            "expected Foo in exports: {:?}",
            export_names
        );
        assert!(
            export_names.contains(&"Bar"),
            "expected Bar in exports: {:?}",
            export_names
        );
    }
}
