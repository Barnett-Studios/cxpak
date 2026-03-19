use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct DartLanguage;

impl DartLanguage {
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
            if child.kind() == "identifier" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    /// Dart visibility: names starting with `_` are private.
    fn is_public(name: &str) -> bool {
        !name.starts_with('_')
    }

    fn extract_fn_body(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "function_body" || child.kind() == "block" {
                let text = &source[child.start_byte()..child.end_byte()];
                return String::from_utf8_lossy(text).into_owned();
            }
        }
        String::new()
    }

    /// Extract import from import_or_export node.
    fn extract_import(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        let text = Self::node_text(node, source).trim().to_string();
        // e.g., "import 'package:flutter/material.dart';"
        // e.g., "import 'dart:math' as math;"
        // e.g., "export 'src/utils.dart';"
        let trimmed = text
            .trim_start_matches("import")
            .trim_start_matches("export")
            .trim()
            .trim_end_matches(';')
            .trim()
            .to_string();

        // Remove "as alias", "show names", "hide names" suffixes
        let path = if let Some(idx) = trimmed.find(" as ") {
            trimmed[..idx].trim().to_string()
        } else if let Some(idx) = trimmed.find(" show ") {
            trimmed[..idx].trim().to_string()
        } else if let Some(idx) = trimmed.find(" hide ") {
            trimmed[..idx].trim().to_string()
        } else {
            trimmed
        };

        let path = path.trim_matches('\'').trim_matches('"').to_string();

        if path.is_empty() {
            return None;
        }

        let name = path
            .rsplit('/')
            .next()
            .unwrap_or(&path)
            .trim_end_matches(".dart")
            .to_string();

        Some(Import {
            source: path,
            names: vec![name],
        })
    }

    /// Extract methods from a class body.
    /// In the tree-sitter-dart grammar, class body children are `class_member`
    /// nodes that wrap `method_signature` / `function_body` pairs.
    fn extract_methods(node: &tree_sitter::Node, source: &[u8]) -> Vec<Symbol> {
        let mut methods = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "class_body" {
                let mut inner_cursor = child.walk();
                for item in child.children(&mut inner_cursor) {
                    // class_member wraps method_signature + function_body
                    if item.kind() == "class_member" {
                        let mut has_method_sig = false;
                        let mut name = String::new();
                        let mut sig_node: Option<tree_sitter::Node> = None;
                        let mut body_text = String::new();
                        let mut member_cursor = item.walk();
                        for member_child in item.children(&mut member_cursor) {
                            match member_child.kind() {
                                "method_signature" | "function_signature" => {
                                    has_method_sig = true;
                                    name = Self::extract_name(&member_child, source);
                                    sig_node = Some(member_child);
                                }
                                "function_body" | "block" => {
                                    let text =
                                        &source[member_child.start_byte()..member_child.end_byte()];
                                    body_text = String::from_utf8_lossy(text).into_owned();
                                }
                                _ => {}
                            }
                        }
                        if has_method_sig && !name.is_empty() {
                            let is_pub = Self::is_public(&name);
                            let visibility = if is_pub {
                                Visibility::Public
                            } else {
                                Visibility::Private
                            };
                            let signature = if let Some(sn) = sig_node {
                                Self::node_text(&sn, source).trim().to_string()
                            } else {
                                String::new()
                            };
                            let start_line = item.start_position().row + 1;
                            let end_line = item.end_position().row + 1;

                            methods.push(Symbol {
                                name,
                                kind: SymbolKind::Method,
                                visibility,
                                signature,
                                body: body_text,
                                start_line,
                                end_line,
                            });
                        }
                    }
                }
            }
        }
        methods
    }
}

impl LanguageSupport for DartLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_dart::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "dart"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let mut imports: Vec<Import> = Vec::new();
        let mut exports: Vec<Export> = Vec::new();

        // Collect children into a Vec so we can look ahead for function_body siblings.
        let mut cursor = root.walk();
        let children: Vec<tree_sitter::Node> = root.children(&mut cursor).collect();

        let mut i = 0;
        while i < children.len() {
            let node = children[i];
            match node.kind() {
                // Top-level functions: function_signature followed by function_body sibling
                "function_signature" | "function_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_public(&name);
                    let visibility = if is_pub {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };
                    let signature = Self::node_text(&node, source_bytes).trim().to_string();
                    // Look ahead for sibling function_body
                    let body =
                        if i + 1 < children.len() && children[i + 1].kind() == "function_body" {
                            let body_node = children[i + 1];
                            let text = &source_bytes[body_node.start_byte()..body_node.end_byte()];
                            i += 1; // skip the function_body node
                            String::from_utf8_lossy(text).into_owned()
                        } else {
                            Self::extract_fn_body(&node, source_bytes)
                        };
                    let start_line = node.start_position().row + 1;
                    let end_line = if i < children.len() {
                        children[i].end_position().row + 1
                    } else {
                        node.end_position().row + 1
                    };

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

                "class_definition" | "class_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_public(&name);
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
                        name: name.clone(),
                        kind: SymbolKind::Class,
                        visibility,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });

                    // Extract methods from class body
                    let methods = Self::extract_methods(&node, source_bytes);
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

                "enum_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_public(&name);
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

                "import_or_export" => {
                    if let Some(imp) = Self::extract_import(&node, source_bytes) {
                        imports.push(imp);
                    }
                }

                _ => {}
            }
            i += 1;
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
            .set_language(&tree_sitter_dart::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_function() {
        let source = r#"void greet(String name) {
  print('Hello, $name!');
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = DartLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(
            !funcs.is_empty(),
            "expected function symbol, got symbols: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
        assert_eq!(funcs[0].visibility, Visibility::Public);

        assert!(
            !result.exports.is_empty(),
            "public function should be exported"
        );
    }

    #[test]
    fn test_extract_private_function() {
        let source = r#"void _helper(int x) {
  return;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = DartLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        if !funcs.is_empty() {
            assert_eq!(funcs[0].visibility, Visibility::Private);
            assert!(
                result.exports.is_empty(),
                "private function should not be exported"
            );
        }
    }

    #[test]
    fn test_extract_class() {
        let source = r#"class Animal {
  String name;

  void speak() {
    print(name);
  }
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = DartLanguage;
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
    fn test_extract_imports() {
        let source = "import 'dart:math';\nimport 'package:flutter/material.dart';\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = DartLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.imports.is_empty(),
            "expected imports, got: {:?}",
            result.imports
        );
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = DartLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
    }

    #[test]
    fn test_complex_dart() {
        let source = r#"import 'dart:async';
import 'package:http/http.dart' as http;

class ApiService {
  final String baseUrl;

  ApiService(this.baseUrl);

  Future<String> fetch(String path) async {
    return '';
  }

  void _log(String msg) {
    print(msg);
  }
}

void main() {
  final svc = ApiService('https://api.example.com');
  svc.fetch('/users');
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = DartLanguage;
        let result = lang.extract(source, &tree);

        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert!(!classes.is_empty(), "expected class");

        assert!(!result.imports.is_empty(), "expected imports");

        // Should have at least the main function
        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected top-level function (main)");
    }

    #[test]
    fn test_private_class() {
        let source = r#"class _InternalWidget {
  void build() {}
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = DartLanguage;
        let result = lang.extract(source, &tree);

        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        if !classes.is_empty() {
            assert_eq!(classes[0].visibility, Visibility::Private);
            assert!(
                !result.exports.iter().any(|e| e.name == "_InternalWidget"),
                "private class should not be exported"
            );
        }
    }
}
