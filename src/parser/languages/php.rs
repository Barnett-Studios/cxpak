use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct PhpLanguage;

impl PhpLanguage {
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
            if child.kind() == "name" || child.kind() == "identifier" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    fn extract_fn_signature(node: &tree_sitter::Node, source: &[u8]) -> String {
        let full_text = Self::node_text(node, source);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "compound_statement" {
                let body_start = child.start_byte() - node.start_byte();
                return full_text[..body_start].trim().to_string();
            }
        }
        full_text.lines().next().unwrap_or("").trim().to_string()
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

    /// Determine visibility from modifier keywords within a declaration.
    fn extract_visibility(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "visibility_modifier" {
                let text = Self::node_text(&child, source);
                if text == "private" {
                    return Visibility::Private;
                }
                // public, protected treated as Public
                return Visibility::Public;
            }
        }
        Visibility::Public
    }

    /// Extract use declarations (namespace imports).
    fn extract_use_import(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        let text = Self::node_text(node, source).trim().to_string();
        // e.g., "use App\Models\User;" or "use App\Http\Controllers\{UserController, PostController};"
        let trimmed = text
            .trim_start_matches("use")
            .trim()
            .trim_end_matches(';')
            .trim()
            .to_string();

        if trimmed.is_empty() {
            return None;
        }

        // Handle grouped imports: use App\Http\{Foo, Bar};
        if let Some(brace_start) = trimmed.find('{') {
            let base = trimmed[..brace_start].trim_end_matches('\\').to_string();
            let names_str = &trimmed[brace_start + 1..];
            let names_str = names_str.trim_end_matches('}');
            let names: Vec<String> = names_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            Some(Import {
                source: base,
                names,
            })
        } else {
            // Single import: use App\Models\User
            let name = trimmed.rsplit('\\').next().unwrap_or(&trimmed).to_string();
            Some(Import {
                source: trimmed,
                names: vec![name],
            })
        }
    }

    /// Extract methods from a class body (declaration_list).
    fn extract_methods(node: &tree_sitter::Node, source: &[u8]) -> Vec<Symbol> {
        let mut methods = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "declaration_list" {
                let mut inner_cursor = child.walk();
                for item in child.children(&mut inner_cursor) {
                    if item.kind() == "method_declaration" {
                        let name = Self::extract_name(&item, source);
                        let visibility = Self::extract_visibility(&item, source);
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

impl LanguageSupport for PhpLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_php::LANGUAGE_PHP.into()
    }

    fn name(&self) -> &str {
        "php"
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
                    let signature = Self::extract_fn_signature(&node, source_bytes);
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

                "class_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    exports.push(Export {
                        name: name.clone(),
                        kind: SymbolKind::Class,
                    });
                    symbols.push(Symbol {
                        name: name.clone(),
                        kind: SymbolKind::Class,
                        visibility: Visibility::Public,
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

                "interface_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    exports.push(Export {
                        name: name.clone(),
                        kind: SymbolKind::Interface,
                    });
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Interface,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "trait_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    exports.push(Export {
                        name: name.clone(),
                        kind: SymbolKind::Trait,
                    });
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Trait,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "namespace_use_declaration" => {
                    if let Some(imp) = Self::extract_use_import(&node, source_bytes) {
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
            .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_function() {
        let source = r#"<?php

function greet($name) {
    return "Hello, " . $name;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = PhpLanguage;
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
    fn test_extract_class_with_methods() {
        let source = r#"<?php

class User {
    public function getName() {
        return $this->name;
    }

    private function validate() {
        return true;
    }
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = PhpLanguage;
        let result = lang.extract(source, &tree);

        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert!(!classes.is_empty(), "expected class symbol");
        assert_eq!(classes[0].name, "User");

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(
            methods.len() >= 2,
            "expected at least 2 methods, got: {:?}",
            methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );

        let get_name = methods.iter().find(|m| m.name == "getName");
        assert!(get_name.is_some(), "expected getName method");
        assert_eq!(get_name.unwrap().visibility, Visibility::Public);

        let validate = methods.iter().find(|m| m.name == "validate");
        assert!(validate.is_some(), "expected validate method");
        assert_eq!(validate.unwrap().visibility, Visibility::Private);
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"<?php

use App\Models\User;
use App\Http\Controllers\UserController;
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = PhpLanguage;
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
        let lang = PhpLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
    }

    #[test]
    fn test_complex_php() {
        let source = r#"<?php

namespace App\Services;

use App\Models\User;
use App\Contracts\AuthInterface;

interface Authenticatable {
    public function authenticate();
}

trait HasRoles {
    public function getRoles() {
        return [];
    }
}

class AuthService {
    public function login($email, $password) {
        return true;
    }

    protected function hash($value) {
        return md5($value);
    }

    private function log($message) {
        echo $message;
    }
}

function helper() {
    return "help";
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = PhpLanguage;
        let result = lang.extract(source, &tree);

        // Should have interface, trait, class, function
        let interfaces: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Interface)
            .collect();
        assert!(!interfaces.is_empty(), "expected interface");

        let traits: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Trait)
            .collect();
        assert!(!traits.is_empty(), "expected trait");

        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert!(!classes.is_empty(), "expected class");

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected top-level function");

        assert!(!result.imports.is_empty(), "expected imports");
    }

    #[test]
    fn test_protected_method_is_public_visibility() {
        let source = r#"<?php

class Base {
    protected function setup() {
        return true;
    }
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = PhpLanguage;
        let result = lang.extract(source, &tree);

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(!methods.is_empty(), "expected method");
        assert_eq!(methods[0].visibility, Visibility::Public);
    }
}
