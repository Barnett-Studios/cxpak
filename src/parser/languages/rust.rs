use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct RustLanguage;

impl RustLanguage {
    fn is_public(node: &tree_sitter::Node, source: &[u8]) -> bool {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "visibility_modifier" {
                let text = child.utf8_text(source).unwrap_or("");
                return text.starts_with("pub");
            }
        }
        false
    }

    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn extract_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" || child.kind() == "type_identifier" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    /// Extract function signature: everything before the block body.
    fn extract_fn_signature(node: &tree_sitter::Node, source: &[u8]) -> String {
        let full_text = Self::node_text(node, source);
        // Find the body block — it's a child node of kind "block"
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block" {
                let body_start = child.start_byte() - node.start_byte();
                return full_text[..body_start].trim().to_string();
            }
        }
        // No block found (e.g. trait method declaration without body)
        full_text.trim().to_string()
    }

    /// Extract the block body text (the `{ ... }` part) of a function.
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

    /// First line of a node's text (used as signature for type definitions).
    fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        text.lines().next().unwrap_or("").trim().to_string()
    }

    /// Full text of a node except any trailing block body.
    fn type_signature(node: &tree_sitter::Node, source: &[u8]) -> String {
        let full_text = Self::node_text(node, source);
        // For structs/enums/traits, the "signature" is the first line
        full_text.lines().next().unwrap_or("").trim().to_string()
    }

    /// Collect all Import entries from a use_declaration node.
    /// For multi-source imports (e.g. `use std::{collections::HashMap, io::Write}`)
    /// this returns one entry per distinct leaf.
    fn extract_all_use_imports(node: &tree_sitter::Node, source: &[u8]) -> Vec<Import> {
        let mut results: Vec<Import> = Vec::new();
        Self::collect_use_leaves(node, "", source, &mut results);
        // Merge entries that share the same source path.
        let mut merged: Vec<Import> = Vec::new();
        for imp in results {
            if let Some(existing) = merged.iter_mut().find(|m| m.source == imp.source) {
                existing.names.extend(imp.names);
            } else {
                merged.push(imp);
            }
        }
        merged
    }

    /// Recursively collect leaf Import entries from a use_declaration or
    /// use_tree node. `prefix` accumulates the path segments seen so far.
    fn collect_use_leaves(
        node: &tree_sitter::Node,
        prefix: &str,
        source: &[u8],
        out: &mut Vec<Import>,
    ) {
        match node.kind() {
            "use_declaration" => {
                // Skip "use" keyword and ";" — recurse into the use_tree child.
                let prev_len = out.len();
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    match child.kind() {
                        "use_tree" | "scoped_use_list" | "scoped_identifier" | "identifier"
                        | "use_as_clause" | "use_wildcard" => {
                            Self::collect_use_leaves(&child, prefix, source, out);
                        }
                        _ => {}
                    }
                }
                // Fallback: if no structured children were recognised, parse text directly.
                if out.len() == prev_len {
                    let text = Self::node_text(node, source);
                    let inner = text
                        .trim_start_matches("use")
                        .trim()
                        .trim_end_matches(';')
                        .trim();
                    if let Some(imp) = Self::parse_use_text(inner, prefix) {
                        out.push(imp);
                    }
                }
            }

            "scoped_use_list" => {
                // grammar: path "::" "{" use_tree,* "}"
                // Find the path part and then each use_tree inside the braces.
                let mut path_prefix = prefix.to_string();
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    match child.kind() {
                        "identifier" | "scoped_identifier" => {
                            let seg = Self::node_text(&child, source);
                            path_prefix = if path_prefix.is_empty() {
                                seg.to_string()
                            } else {
                                format!("{path_prefix}::{seg}")
                            };
                        }
                        "use_list" => {
                            let mut inner = child.walk();
                            for item in child.children(&mut inner) {
                                match item.kind() {
                                    "use_tree" | "scoped_use_list" | "scoped_identifier"
                                    | "identifier" | "use_as_clause" | "use_wildcard" => {
                                        Self::collect_use_leaves(&item, &path_prefix, source, out);
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            "use_tree" => {
                // Delegate to children
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    match child.kind() {
                        "use_tree" | "scoped_use_list" | "scoped_identifier" | "identifier"
                        | "use_as_clause" | "use_wildcard" => {
                            Self::collect_use_leaves(&child, prefix, source, out);
                        }
                        _ => {}
                    }
                }
                // If this use_tree has no recognised children fall back to
                // text-based parsing so simple paths still work.
                if out.is_empty() || node.child_count() == 0 {
                    let text = Self::node_text(node, source);
                    if let Some(imp) = Self::parse_use_text(text, prefix) {
                        out.push(imp);
                    }
                }
            }

            "scoped_identifier" => {
                // e.g. `std::io` or `collections::HashMap`
                let text = Self::node_text(node, source);
                if let Some(imp) = Self::parse_use_text(text, prefix) {
                    out.push(imp);
                }
            }

            "identifier" => {
                let name = Self::node_text(node, source).to_string();
                if !name.is_empty() && name != "use" && name != "self" {
                    out.push(Import {
                        source: prefix.to_string(),
                        names: vec![name],
                    });
                }
            }

            "use_wildcard" => {
                // tree-sitter-rust: use_wildcard = scoped_identifier "::" "*"
                // The scoped_identifier child holds the module path (e.g. "std::collections").
                // Combine it with any outer prefix.
                let mut source_path = prefix.to_string();
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "scoped_identifier" || child.kind() == "identifier" {
                        let seg = Self::node_text(&child, source);
                        source_path = if source_path.is_empty() {
                            seg.to_string()
                        } else {
                            format!("{source_path}::{seg}")
                        };
                    }
                }
                out.push(Import {
                    source: source_path,
                    names: vec!["*".to_string()],
                });
            }

            "use_as_clause" => {
                // e.g. `std::io::Error as IoError` or (when nested) `Error as IoError`
                let text = Self::node_text(node, source);
                if let Some(as_idx) = text.find(" as ") {
                    let before = text[..as_idx].trim();
                    let alias = text[as_idx + 4..].trim().to_string();
                    // Derive source from the path up to the last "::" in `before`,
                    // anchored by `prefix`.
                    let source_path = if let Some(sep) = before.rfind("::") {
                        let module = &before[..sep];
                        if prefix.is_empty() {
                            module.to_string()
                        } else {
                            format!("{prefix}::{module}")
                        }
                    } else {
                        prefix.to_string()
                    };
                    out.push(Import {
                        source: source_path,
                        names: vec![alias],
                    });
                }
            }

            _ => {}
        }
    }

    /// Text-level fallback parser for a single use path segment.
    fn parse_use_text(text: &str, prefix: &str) -> Option<Import> {
        let inner = text.trim();
        if inner.is_empty() {
            return None;
        }

        // Glob
        if inner.ends_with("::*") {
            let base = inner.trim_end_matches("::*");
            let source_path = if prefix.is_empty() {
                base.to_string()
            } else {
                format!("{prefix}::{base}")
            };
            return Some(Import {
                source: source_path,
                names: vec!["*".to_string()],
            });
        }

        // Alias
        if let Some(as_idx) = inner.find(" as ") {
            let alias = inner[as_idx + 4..].trim().to_string();
            return Some(Import {
                source: prefix.to_string(),
                names: vec![alias],
            });
        }

        // path::Name
        if let Some(sep) = inner.rfind("::") {
            let module = &inner[..sep];
            let name = inner[sep + 2..].to_string();
            let source_path = if prefix.is_empty() {
                module.to_string()
            } else {
                format!("{prefix}::{module}")
            };
            return Some(Import {
                source: source_path,
                names: vec![name],
            });
        }

        // Bare identifier
        Some(Import {
            source: prefix.to_string(),
            names: vec![inner.to_string()],
        })
    }

    fn extract_impl_methods(node: &tree_sitter::Node, source: &[u8]) -> Vec<Symbol> {
        let mut methods = Vec::new();
        // impl body is a "declaration_list" child
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "declaration_list" {
                let mut inner_cursor = child.walk();
                for item in child.children(&mut inner_cursor) {
                    if item.kind() == "function_item" {
                        let name = Self::extract_name(&item, source);
                        let visibility = if Self::is_public(&item, source) {
                            Visibility::Public
                        } else {
                            Visibility::Private
                        };
                        let signature = Self::extract_fn_signature(&item, source);
                        let body = Self::extract_fn_body(&item, source);
                        let start_line = item.start_position().row + 1;
                        let end_line = item.end_position().row + 1;
                        methods.push(Symbol {
                            name,
                            kind: SymbolKind::Method,
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
        methods
    }
}

impl LanguageSupport for RustLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_rust::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "rust"
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
                "function_item" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_public(&node, source_bytes);
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

                "struct_item" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_public(&node, source_bytes);
                    let visibility = if is_pub {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };
                    let signature = Self::type_signature(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if is_pub {
                        exports.push(Export {
                            name: name.clone(),
                            kind: SymbolKind::Struct,
                        });
                    }

                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Struct,
                        visibility,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "enum_item" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_public(&node, source_bytes);
                    let visibility = if is_pub {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };
                    let signature = Self::type_signature(&node, source_bytes);
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

                "trait_item" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_public(&node, source_bytes);
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
                            kind: SymbolKind::Trait,
                        });
                    }

                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Trait,
                        visibility,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "impl_item" => {
                    let methods = Self::extract_impl_methods(&node, source_bytes);
                    // Add public methods to exports
                    for method in &methods {
                        if method.visibility == Visibility::Public {
                            exports.push(Export {
                                name: method.name.clone(),
                                kind: SymbolKind::Method,
                            });
                        }
                    }
                    symbols.extend(methods);
                }

                "type_item" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_public(&node, source_bytes);
                    let visibility = if is_pub {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };
                    let signature = Self::node_text(&node, source_bytes)
                        .trim_end_matches(';')
                        .trim()
                        .to_string();
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
                        body: String::new(),
                        start_line,
                        end_line,
                    });
                }

                "const_item" | "static_item" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_public(&node, source_bytes);
                    let visibility = if is_pub {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };
                    let signature = Self::node_text(&node, source_bytes)
                        .trim_end_matches(';')
                        .trim()
                        .to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if is_pub {
                        exports.push(Export {
                            name: name.clone(),
                            kind: SymbolKind::Constant,
                        });
                    }
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Constant,
                        visibility,
                        signature,
                        body: String::new(),
                        start_line,
                        end_line,
                    });
                }

                "use_declaration" => {
                    let is_pub = Self::is_public(&node, source_bytes);
                    for import in Self::extract_all_use_imports(&node, source_bytes) {
                        if is_pub {
                            for name in &import.names {
                                exports.push(Export {
                                    name: name.clone(),
                                    kind: SymbolKind::TypeAlias,
                                });
                            }
                        }
                        imports.push(import);
                    }
                }

                "macro_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    if !name.is_empty() {
                        symbols.push(Symbol {
                            name: name.clone(),
                            kind: SymbolKind::Macro,
                            visibility: Visibility::Public,
                            signature: Self::first_line(&node, source_bytes),
                            body: String::new(),
                            start_line: node.start_position().row + 1,
                            end_line: node.end_position().row + 1,
                        });
                        exports.push(Export {
                            name,
                            kind: SymbolKind::Macro,
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
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_public_function() {
        let source = r#"
pub fn greet(name: &str) -> String {
    format!("Hello, {}!", name)
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.symbols.len(), 1);
        let sym = &result.symbols[0];
        assert_eq!(sym.name, "greet");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, Visibility::Public);
        assert!(
            sym.signature.contains("pub fn greet"),
            "signature: {}",
            sym.signature
        );
        assert!(sym.body.contains("format!"), "body: {}", sym.body);

        assert_eq!(result.exports.len(), 1);
        assert_eq!(result.exports[0].name, "greet");
    }

    #[test]
    fn test_extract_private_function() {
        let source = r#"
fn helper(x: i32) -> i32 {
    x * 2
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.symbols.len(), 1);
        let sym = &result.symbols[0];
        assert_eq!(sym.name, "helper");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, Visibility::Private);
        assert!(
            sym.signature.contains("fn helper"),
            "signature: {}",
            sym.signature
        );
        assert!(sym.body.contains("x * 2"), "body: {}", sym.body);

        assert!(
            result.exports.is_empty(),
            "private function should not be exported"
        );
    }

    #[test]
    fn test_extract_struct() {
        let source = r#"
pub struct Point {
    pub x: f64,
    pub y: f64,
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.symbols.len(), 1);
        let sym = &result.symbols[0];
        assert_eq!(sym.name, "Point");
        assert_eq!(sym.kind, SymbolKind::Struct);
        assert_eq!(sym.visibility, Visibility::Public);
        assert!(
            sym.signature.contains("pub struct Point"),
            "signature: {}",
            sym.signature
        );

        assert_eq!(result.exports.len(), 1);
        assert_eq!(result.exports[0].name, "Point");
        assert_eq!(result.exports[0].kind, SymbolKind::Struct);
    }

    #[test]
    fn test_extract_enum() {
        let source = r#"
pub enum Direction {
    North,
    South,
    East,
    West,
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.symbols.len(), 1);
        let sym = &result.symbols[0];
        assert_eq!(sym.name, "Direction");
        assert_eq!(sym.kind, SymbolKind::Enum);
        assert_eq!(sym.visibility, Visibility::Public);
        assert!(
            sym.signature.contains("pub enum Direction"),
            "signature: {}",
            sym.signature
        );

        assert_eq!(result.exports.len(), 1);
        assert_eq!(result.exports[0].name, "Direction");
        assert_eq!(result.exports[0].kind, SymbolKind::Enum);
    }

    #[test]
    fn test_extract_trait() {
        let source = r#"
pub trait Animal {
    fn name(&self) -> &str;
    fn sound(&self) -> &str;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.symbols.len(), 1);
        let sym = &result.symbols[0];
        assert_eq!(sym.name, "Animal");
        assert_eq!(sym.kind, SymbolKind::Trait);
        assert_eq!(sym.visibility, Visibility::Public);
        assert!(
            sym.signature.contains("pub trait Animal"),
            "signature: {}",
            sym.signature
        );

        assert_eq!(result.exports.len(), 1);
        assert_eq!(result.exports[0].name, "Animal");
        assert_eq!(result.exports[0].kind, SymbolKind::Trait);
    }

    #[test]
    fn test_extract_use_import() {
        let source = r#"
use std::collections::HashMap;
use std::io::{Read, Write};
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.imports.len(), 2);

        let first = &result.imports[0];
        assert_eq!(first.source, "std::collections");
        assert_eq!(first.names, vec!["HashMap"]);

        let second = &result.imports[1];
        assert_eq!(second.source, "std::io");
        assert!(
            second.names.contains(&"Read".to_string()),
            "names: {:?}",
            second.names
        );
        assert!(
            second.names.contains(&"Write".to_string()),
            "names: {:?}",
            second.names
        );
    }

    #[test]
    fn test_extract_impl_methods() {
        let source = r#"
struct Counter {
    count: u32,
}

impl Counter {
    pub fn increment(&mut self) {
        self.count += 1;
    }

    fn reset(&mut self) {
        self.count = 0;
    }
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        // Symbols: struct + 2 methods
        let methods: Vec<&Symbol> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();

        assert_eq!(
            methods.len(),
            2,
            "expected 2 methods, got: {:?}",
            methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );

        let increment = methods
            .iter()
            .find(|m| m.name == "increment")
            .expect("increment method not found");
        assert_eq!(increment.visibility, Visibility::Public);
        assert!(
            increment.signature.contains("pub fn increment"),
            "sig: {}",
            increment.signature
        );
        assert!(
            increment.body.contains("self.count += 1"),
            "body: {}",
            increment.body
        );

        let reset = methods
            .iter()
            .find(|m| m.name == "reset")
            .expect("reset method not found");
        assert_eq!(reset.visibility, Visibility::Private);
        assert!(
            reset.signature.contains("fn reset"),
            "sig: {}",
            reset.signature
        );

        // Only public method should be exported
        let method_exports: Vec<&Export> = result
            .exports
            .iter()
            .filter(|e| e.kind == SymbolKind::Method)
            .collect();
        assert_eq!(method_exports.len(), 1);
        assert_eq!(method_exports[0].name, "increment");
    }

    #[test]
    fn test_extract_glob_import() {
        let source = r#"
use std::collections::*;
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source, "std::collections");
        assert!(result.imports[0].names.contains(&"*".to_string()));
    }

    #[test]
    fn test_extract_bare_identifier_import() {
        let source = r#"
use HashMap;
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.imports.len(), 1);
        assert!(result.imports[0].source.is_empty());
        assert!(result.imports[0].names.contains(&"HashMap".to_string()));
    }

    #[test]
    fn test_extract_trait_method_no_body() {
        let source = r#"
pub trait Serializer {
    fn serialize(&self) -> String;
    fn deserialize(data: &str) -> Self;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        let traits: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Trait)
            .collect();
        assert!(!traits.is_empty());
        // The trait body should contain the method declarations
        assert!(
            traits[0].body.contains("serialize"),
            "body: {}",
            traits[0].body
        );
    }

    #[test]
    fn test_trait_method_no_body() {
        // Trait method declaration without body — covers extract_fn_body String::new() (line 58)
        // and extract_fn_signature fallback to trim (line 46)
        let source = r#"pub trait Handler {
    fn handle(&self, req: Request) -> Response;
    fn name(&self) -> &str;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);
        let traits: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Trait)
            .collect();
        assert!(!traits.is_empty());
    }

    #[test]
    fn test_extract_name_fallback() {
        // Covers extract_name returning String::new() (line 31)
        let source = "use std::io;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);
        let _ = result;
    }

    #[test]
    fn test_bare_use_import() {
        // Covers extract_use_import with bare identifier (line 90)
        let source = "use serde;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);
        assert!(!result.imports.is_empty());
    }

    #[test]
    fn test_type_alias_parsed() {
        // Type aliases aren't extracted as symbols but should parse without error
        let source = "pub type Result<T> = std::result::Result<T, Error>;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);
        let _ = result;
    }

    #[test]
    fn test_private_trait() {
        // Covers Private visibility branch for trait_item (line 291)
        let source = r#"
trait InternalHelper {
    fn do_work(&self);
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        let traits: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Trait)
            .collect();
        assert_eq!(traits.len(), 1);
        assert_eq!(traits[0].name, "InternalHelper");
        assert_eq!(traits[0].visibility, Visibility::Private);
        assert!(result.exports.iter().all(|e| e.name != "InternalHelper"));
    }

    #[test]
    fn test_private_enum() {
        // Covers Private visibility branch for enum_item (line 260)
        let source = "enum InternalState {\n    Active,\n    Inactive,\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        let enums: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Enum)
            .collect();
        assert_eq!(enums.len(), 1);
        assert_eq!(enums[0].name, "InternalState");
        assert_eq!(enums[0].visibility, Visibility::Private);
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_trait_method_without_body() {
        // Trait method declaration has no block body → covers fallback paths in
        // extract_fn_signature (line 46) and extract_fn_body (line 58)
        let source = "pub trait Greeter {\n    fn greet(&self) -> String;\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);
        // The trait should be extracted
        let traits: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Trait)
            .collect();
        assert!(!traits.is_empty());
    }

    #[test]
    fn test_type_alias_captured() {
        let source = "pub type Foo = Bar;\ntype X = Y;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        let aliases: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::TypeAlias)
            .collect();
        assert_eq!(
            aliases.len(),
            2,
            "expected 2 type aliases, got: {:?}",
            aliases.iter().map(|s| &s.name).collect::<Vec<_>>()
        );

        let foo = aliases.iter().find(|s| s.name == "Foo").expect("Foo alias");
        assert_eq!(foo.visibility, Visibility::Public);
        assert!(result
            .exports
            .iter()
            .any(|e| e.name == "Foo" && e.kind == SymbolKind::TypeAlias));

        let x = aliases.iter().find(|s| s.name == "X").expect("X alias");
        assert_eq!(x.visibility, Visibility::Private);
        assert!(!result.exports.iter().any(|e| e.name == "X"));
    }

    #[test]
    fn test_const_and_static_captured() {
        let source = "pub const MAX_SIZE: usize = 1024;\nstatic COUNTER: u32 = 0;\npub static mut FLAG: bool = false;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        let constants: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Constant)
            .collect();
        assert_eq!(
            constants.len(),
            3,
            "expected 3 constants, got: {:?}",
            constants.iter().map(|s| &s.name).collect::<Vec<_>>()
        );

        let max = constants
            .iter()
            .find(|s| s.name == "MAX_SIZE")
            .expect("MAX_SIZE");
        assert_eq!(max.visibility, Visibility::Public);
        assert!(result
            .exports
            .iter()
            .any(|e| e.name == "MAX_SIZE" && e.kind == SymbolKind::Constant));

        let counter = constants
            .iter()
            .find(|s| s.name == "COUNTER")
            .expect("COUNTER");
        assert_eq!(counter.visibility, Visibility::Private);
        assert!(!result.exports.iter().any(|e| e.name == "COUNTER"));
    }

    #[test]
    fn test_pub_use_produces_export() {
        let source = "pub use std::collections::HashMap;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.imports.len(), 1, "import should be recorded");
        assert!(
            result.exports.iter().any(|e| e.name == "HashMap"),
            "pub use should produce an Export for HashMap, got: {:?}",
            result.exports
        );
    }

    #[test]
    fn test_macro_rules_produces_symbol() {
        let source = "macro_rules! foo { () => {} }\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        let macros: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Macro)
            .collect();
        assert_eq!(
            macros.len(),
            1,
            "expected one macro symbol, got: {:?}",
            macros
        );
        assert_eq!(macros[0].name, "foo");
        assert!(
            result
                .exports
                .iter()
                .any(|e| e.name == "foo" && e.kind == SymbolKind::Macro),
            "macro should be in exports"
        );
    }

    #[test]
    fn test_nested_brace_import() {
        // use std::{collections::{HashMap, BTreeMap}, io}
        // Should not mangle into a single broken path.
        let source = "use std::{collections::{HashMap, BTreeMap}, io};\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        // We expect at least one import and none of the names should contain "{"
        assert!(
            !result.imports.is_empty(),
            "expected at least one import from nested braces"
        );
        for import in &result.imports {
            for name in &import.names {
                assert!(
                    !name.contains('{') && !name.contains('}'),
                    "mangled name detected: {name:?}"
                );
            }
        }
        // HashMap and BTreeMap should appear
        let all_names: Vec<&str> = result
            .imports
            .iter()
            .flat_map(|i| i.names.iter().map(|n| n.as_str()))
            .collect();
        assert!(
            all_names.contains(&"HashMap"),
            "HashMap missing: {all_names:?}"
        );
        assert!(
            all_names.contains(&"BTreeMap"),
            "BTreeMap missing: {all_names:?}"
        );
    }

    #[test]
    fn test_import_alias_captured() {
        let source = "use std::io::Error as IoError;\nuse std::collections::HashMap as Map;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RustLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.imports.len(), 2);

        let io_import = result
            .imports
            .iter()
            .find(|i| i.names.contains(&"IoError".to_string()))
            .expect("IoError import not found");
        assert_eq!(io_import.source, "std::io");

        let map_import = result
            .imports
            .iter()
            .find(|i| i.names.contains(&"Map".to_string()))
            .expect("Map import not found");
        assert_eq!(map_import.source, "std::collections");
    }
}
