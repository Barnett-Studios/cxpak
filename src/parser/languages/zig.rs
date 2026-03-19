use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct ZigLanguage;

impl ZigLanguage {
    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        text.lines().next().unwrap_or("").trim().to_string()
    }

    #[allow(dead_code)]
    fn extract_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" || child.kind() == "IDENTIFIER" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    /// Check if a declaration has the `pub` visibility keyword.
    fn has_pub_keyword(node: &tree_sitter::Node, source: &[u8]) -> bool {
        let text = Self::node_text(node, source);
        text.starts_with("pub ")
    }

    /// Extract function signature (everything before the body block).
    fn extract_fn_signature(node: &tree_sitter::Node, source: &[u8]) -> String {
        let full_text = Self::node_text(node, source);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block" || child.kind() == "Block" {
                let body_start = child.start_byte() - node.start_byte();
                if body_start < full_text.len() {
                    return full_text[..body_start].trim().to_string();
                }
            }
        }
        full_text.lines().next().unwrap_or("").trim().to_string()
    }

    /// Extract the body block text.
    fn extract_fn_body(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block" || child.kind() == "Block" {
                let text = &source[child.start_byte()..child.end_byte()];
                return String::from_utf8_lossy(text).into_owned();
            }
        }
        String::new()
    }

    /// Extract @import("...") calls from variable declarations.
    fn extract_import_from_var(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        let text = Self::node_text(node, source);

        // Look for @import("...") pattern in the text
        if let Some(import_start) = text.find("@import(") {
            let after = &text[import_start + 8..];
            // Find the closing quote and paren
            if let Some(quote_start) = after.find('"') {
                let after_quote = &after[quote_start + 1..];
                if let Some(quote_end) = after_quote.find('"') {
                    let module = &after_quote[..quote_end];
                    let name = module.rsplit('/').next().unwrap_or(module);
                    let name = name.trim_end_matches(".zig").to_string();
                    return Some(Import {
                        source: module.to_string(),
                        names: vec![name],
                    });
                }
            }
        }
        None
    }
}

impl LanguageSupport for ZigLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_zig::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "zig"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let mut imports: Vec<Import> = Vec::new();
        let mut exports: Vec<Export> = Vec::new();

        // Zig tree-sitter grammar uses different node kinds.
        // Walk all top-level children.
        let mut stack: Vec<tree_sitter::Node> = root.children(&mut root.walk()).collect();

        while let Some(node) = stack.pop() {
            let kind = node.kind();

            match kind {
                // Function declarations: fn_decl, FnDecl, or Decl containing fn
                "FnDecl" | "fn_decl" | "TopLevelDecl" => {
                    let text = Self::node_text(&node, source_bytes);
                    if text.contains("fn ") {
                        let name = Self::extract_fn_name_from_text(text);
                        let is_pub = Self::has_pub_keyword(&node, source_bytes);
                        let visibility = if is_pub {
                            Visibility::Public
                        } else {
                            Visibility::Private
                        };
                        let signature = Self::extract_fn_signature(&node, source_bytes);
                        let body = Self::extract_fn_body(&node, source_bytes);
                        let start_line = node.start_position().row + 1;
                        let end_line = node.end_position().row + 1;

                        if !name.is_empty() {
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
                    } else {
                        // Could be a variable/const declaration at top level
                        let mut cursor = node.walk();
                        for child in node.children(&mut cursor) {
                            stack.push(child);
                        }
                    }
                }

                // Variable/const declarations
                "VarDecl" | "var_decl" => {
                    let text = Self::node_text(&node, source_bytes);
                    let is_pub = Self::has_pub_keyword(&node, source_bytes);
                    let visibility = if is_pub {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };

                    // Check for @import
                    if text.contains("@import(") {
                        if let Some(imp) = Self::extract_import_from_var(&node, source_bytes) {
                            imports.push(imp);
                        }
                    }

                    // Check if it's a const (could be a struct or constant)
                    if text.starts_with("pub const ") || text.starts_with("const ") {
                        let name = Self::extract_const_name(text);
                        if !name.is_empty() {
                            let sym_kind =
                                if text.contains("struct") || text.contains("packed struct") {
                                    SymbolKind::Struct
                                } else {
                                    SymbolKind::Constant
                                };

                            if is_pub {
                                exports.push(Export {
                                    name: name.clone(),
                                    kind: sym_kind.clone(),
                                });
                            }

                            let signature = Self::first_line(&node, source_bytes);
                            let body = Self::node_text(&node, source_bytes).to_string();
                            let start_line = node.start_position().row + 1;
                            let end_line = node.end_position().row + 1;

                            symbols.push(Symbol {
                                name,
                                kind: sym_kind,
                                visibility,
                                signature,
                                body,
                                start_line,
                                end_line,
                            });
                        }
                    }
                }

                // Some grammars wrap top-level declarations in a container
                "ContainerDecl" | "container_decl" => {
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        stack.push(child);
                    }
                }

                _ => {
                    // For any other node, check if it might contain declarations
                    // and push children
                    let text = Self::node_text(&node, source_bytes);

                    // Handle cases where the grammar wraps things differently
                    if text.contains("fn ") || text.contains("const ") || text.contains("@import") {
                        // Try to extract fn declaration from text
                        if text.contains("fn ") && !text.starts_with("//") {
                            let is_pub = text.starts_with("pub ");
                            let fn_name = Self::extract_fn_name_from_text(text);
                            if !fn_name.is_empty() && !symbols.iter().any(|s| s.name == fn_name) {
                                let visibility = if is_pub {
                                    Visibility::Public
                                } else {
                                    Visibility::Private
                                };
                                let signature = Self::first_line(&node, source_bytes);
                                let body = Self::extract_fn_body(&node, source_bytes);
                                let start_line = node.start_position().row + 1;
                                let end_line = node.end_position().row + 1;

                                if is_pub {
                                    exports.push(Export {
                                        name: fn_name.clone(),
                                        kind: SymbolKind::Function,
                                    });
                                }
                                symbols.push(Symbol {
                                    name: fn_name,
                                    kind: SymbolKind::Function,
                                    visibility,
                                    signature,
                                    body,
                                    start_line,
                                    end_line,
                                });
                            }
                        }

                        // Try to extract const/var declarations
                        if (text.starts_with("const ") || text.starts_with("pub const "))
                            && !symbols
                                .iter()
                                .any(|s| s.name == Self::extract_const_name(text))
                        {
                            let is_pub = text.starts_with("pub ");
                            if text.contains("@import(") {
                                if let Some(imp) =
                                    Self::extract_import_from_var(&node, source_bytes)
                                {
                                    if !imports.iter().any(|i| i.source == imp.source) {
                                        imports.push(imp);
                                    }
                                }
                            }

                            let name = Self::extract_const_name(text);
                            if !name.is_empty() && !name.contains('@') {
                                let sym_kind = if text.contains("struct") {
                                    SymbolKind::Struct
                                } else {
                                    SymbolKind::Constant
                                };
                                let visibility = if is_pub {
                                    Visibility::Public
                                } else {
                                    Visibility::Private
                                };

                                if is_pub {
                                    exports.push(Export {
                                        name: name.clone(),
                                        kind: sym_kind.clone(),
                                    });
                                }

                                let signature = Self::first_line(&node, source_bytes);
                                let body = Self::node_text(&node, source_bytes).to_string();
                                let start_line = node.start_position().row + 1;
                                let end_line = node.end_position().row + 1;

                                symbols.push(Symbol {
                                    name,
                                    kind: sym_kind,
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
            }
        }

        ParseResult {
            symbols,
            imports,
            exports,
        }
    }
}

impl ZigLanguage {
    /// Extract function name from text like "pub fn main() void {" or "fn helper() void {"
    fn extract_fn_name_from_text(text: &str) -> String {
        // Find "fn " and extract the identifier that follows
        if let Some(fn_pos) = text.find("fn ") {
            let after_fn = &text[fn_pos + 3..];
            let name: String = after_fn
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return name;
            }
        }
        String::new()
    }

    /// Extract const name from text like "pub const Foo = struct {" or "const bar = 42;"
    fn extract_const_name(text: &str) -> String {
        let after_const = if let Some(rest) = text.strip_prefix("pub const ") {
            rest
        } else if let Some(rest) = text.strip_prefix("const ") {
            rest
        } else {
            return String::new();
        };

        let name: String = after_const
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::language::{SymbolKind, Visibility};

    fn make_parser() -> tree_sitter::Parser {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_zig::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_function() {
        let source = r#"pub fn main() void {
    const x = 42;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ZigLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected function symbol");
        assert_eq!(funcs[0].name, "main");
        assert_eq!(funcs[0].visibility, Visibility::Public);

        let exported: Vec<_> = result.exports.iter().filter(|e| e.name == "main").collect();
        assert!(!exported.is_empty(), "pub fn should be exported");
    }

    #[test]
    fn test_extract_private_function() {
        let source = r#"fn helper() u32 {
    return 42;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ZigLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected private function");
        assert_eq!(funcs[0].name, "helper");
        assert_eq!(funcs[0].visibility, Visibility::Private);
        assert!(
            result.exports.is_empty(),
            "private function should not be exported"
        );
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"const std = @import("std");
const fs = @import("std").fs;
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ZigLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.imports.is_empty(),
            "expected imports from @import, got: {:?}",
            result.imports
        );
        let sources: Vec<&str> = result.imports.iter().map(|i| i.source.as_str()).collect();
        assert!(
            sources.contains(&"std"),
            "expected std import, got: {:?}",
            sources
        );
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = ZigLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_extract_const_struct() {
        let source = r#"pub const Point = struct {
    x: f64,
    y: f64,
};
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ZigLanguage;
        let result = lang.extract(source, &tree);

        let structs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct)
            .collect();
        assert!(
            !structs.is_empty(),
            "expected struct symbol, got symbols: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
        assert_eq!(structs[0].name, "Point");
        assert_eq!(structs[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_constant() {
        let source = r#"pub const MAX_SIZE = 1024;
const internal_val = 42;
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ZigLanguage;
        let result = lang.extract(source, &tree);

        let constants: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Constant)
            .collect();
        assert!(
            !constants.is_empty(),
            "expected constant symbols, got: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
    }
}
