use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct SwiftLanguage;

impl SwiftLanguage {
    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        text.lines().next().unwrap_or("").trim().to_string()
    }

    fn extract_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        if let Some(name_node) = node.child_by_field_name("name") {
            return Self::node_text(&name_node, source).to_string();
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "simple_identifier" || child.kind() == "identifier" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    /// Swift visibility comes from modifier nodes: `public`, `private`, `internal`, `open`.
    /// The default visibility is `internal` — treated as Private here.
    fn extract_visibility(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            if kind == "modifiers" || kind == "visibility_modifier" || kind == "modifier" {
                let text = Self::node_text(&child, source);
                if text.contains("public") || text.contains("open") {
                    return Visibility::Public;
                }
            }
            // Some grammars surface modifiers as direct children with keyword kinds
            if kind == "public" || kind == "open" {
                return Visibility::Public;
            }
        }
        Visibility::Private
    }

    fn extract_fn_body(node: &tree_sitter::Node, source: &[u8]) -> String {
        if let Some(body_node) = node.child_by_field_name("body") {
            let text = &source[body_node.start_byte()..body_node.end_byte()];
            return String::from_utf8_lossy(text).into_owned();
        }
        // Fall back to looking for a code_block child
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "code_block" || child.kind() == "function_body" {
                let text = &source[child.start_byte()..child.end_byte()];
                return String::from_utf8_lossy(text).into_owned();
            }
        }
        String::new()
    }

    fn extract_fn_signature(node: &tree_sitter::Node, source: &[u8]) -> String {
        let full_text = Self::node_text(node, source);
        // Signature is everything before the body block
        if let Some(body_node) = node.child_by_field_name("body") {
            let body_start = body_node.start_byte() - node.start_byte();
            return full_text[..body_start].trim().to_string();
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "code_block" || child.kind() == "function_body" {
                let body_start = child.start_byte() - node.start_byte();
                return full_text[..body_start].trim().to_string();
            }
        }
        full_text.lines().next().unwrap_or("").trim().to_string()
    }

    /// Determine the SymbolKind for a `class_declaration` node by inspecting the
    /// `declaration_kind` field, which contains tokens like `class`, `struct`, `enum`,
    /// `actor`, `extension`.
    fn class_declaration_kind(node: &tree_sitter::Node, source: &[u8]) -> SymbolKind {
        if let Some(kind_node) = node.child_by_field_name("declaration_kind") {
            let kind_text = Self::node_text(&kind_node, source);
            return match kind_text {
                "struct" => SymbolKind::Struct,
                "enum" => SymbolKind::Enum,
                _ => SymbolKind::Class,
            };
        }
        SymbolKind::Class
    }

    /// Extract the import path from an `import_declaration` node.
    /// e.g. `import Foundation` → source="Foundation", names=["Foundation"]
    fn extract_import(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        let text = Self::node_text(node, source);
        // Strip leading "import" keyword and optional kind (e.g., "import class Foundation.NSString")
        let stripped = text.trim_start_matches("import").trim();
        // Drop optional import kind (class, struct, enum, func, var, let, typealias, protocol)
        let import_kinds = [
            "class",
            "struct",
            "enum",
            "func",
            "var",
            "let",
            "typealias",
            "protocol",
            "actor",
        ];
        let module_part = {
            let mut s = stripped;
            for kw in &import_kinds {
                if let Some(rest) = s.strip_prefix(kw) {
                    if rest.starts_with(|c: char| c.is_whitespace()) {
                        s = rest.trim();
                        break;
                    }
                }
            }
            s
        };

        if module_part.is_empty() {
            return None;
        }

        let name = module_part
            .rsplit('.')
            .next()
            .unwrap_or(module_part)
            .to_string();
        Some(Import {
            source: module_part.to_string(),
            names: vec![name],
        })
    }
}

impl LanguageSupport for SwiftLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_swift::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "swift"
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
                "import_declaration" => {
                    if let Some(imp) = Self::extract_import(&node, source_bytes) {
                        imports.push(imp);
                    }
                }

                "function_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let visibility = Self::extract_visibility(&node, source_bytes);
                    let is_pub = visibility == Visibility::Public;
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

                // Swift uses `class_declaration` for class, struct, enum, actor, extension
                "class_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let visibility = Self::extract_visibility(&node, source_bytes);
                    let is_pub = visibility == Visibility::Public;
                    let kind = Self::class_declaration_kind(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

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

                    // Recurse into the body to find nested declarations
                    if let Some(body_node) = node.child_by_field_name("body") {
                        let mut cursor = body_node.walk();
                        for child in body_node.children(&mut cursor) {
                            stack.push(child);
                        }
                    }
                }

                "protocol_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let visibility = Self::extract_visibility(&node, source_bytes);
                    let is_pub = visibility == Visibility::Public;
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
            .set_language(&tree_sitter_swift::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_public_function() {
        let source = r#"public func greet(name: String) -> String {
    return "Hello, \(name)!"
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = SwiftLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected function symbol");
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
    fn test_extract_struct() {
        let source = r#"public struct Point {
    var x: Double
    var y: Double
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = SwiftLanguage;
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
        let source = r#"import Foundation
import UIKit
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = SwiftLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(
            result.imports.len(),
            2,
            "expected 2 imports, got {:?}",
            result.imports
        );
        let sources: Vec<&str> = result.imports.iter().map(|i| i.source.as_str()).collect();
        assert!(
            sources.contains(&"Foundation"),
            "expected Foundation import, got: {:?}",
            sources
        );
        assert!(
            sources.contains(&"UIKit"),
            "expected UIKit import, got: {:?}",
            sources
        );
    }
}
