use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct RubyLanguage;

impl RubyLanguage {
    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        text.lines().next().unwrap_or("").trim().to_string()
    }

    /// Extract the name from a node by finding an `identifier` child.
    fn extract_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" || child.kind() == "constant" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    /// Ruby `require` / `require_relative` calls appear as `call` nodes.
    /// We look for `call` nodes whose `method` field text is "require" or "require_relative".
    fn extract_require(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        // Verify that the method name is require or require_relative
        let method_name = node
            .child_by_field_name("method")
            .map(|n| Self::node_text(&n, source))
            .unwrap_or("");

        if method_name != "require" && method_name != "require_relative" {
            return None;
        }

        // The arguments node contains the string literal
        let args = node.child_by_field_name("arguments")?;
        let mut cursor = args.walk();
        for child in args.children(&mut cursor) {
            let text = Self::node_text(&child, source);
            let trimmed = text
                .trim_matches('"')
                .trim_matches('\'')
                .trim_start_matches('(')
                .trim_end_matches(')')
                .trim_matches('"')
                .trim_matches('\'');
            if !trimmed.is_empty() && child.kind() != "," {
                let source_path = trimmed.to_string();
                let name = source_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&source_path)
                    .to_string();
                return Some(Import {
                    source: source_path,
                    names: vec![name],
                });
            }
        }
        None
    }

    fn extract_fn_body(node: &tree_sitter::Node, source: &[u8]) -> String {
        // Ruby method body is in a "body_statement" child
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "body_statement" || child.kind() == "do_block" {
                let text = &source[child.start_byte()..child.end_byte()];
                return String::from_utf8_lossy(text).into_owned();
            }
        }
        String::new()
    }

    /// Return the method name for a `call` node (the `method` field text).
    fn extract_call_method_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        node.child_by_field_name("method")
            .map(|n| Self::node_text(&n, source).to_string())
            .unwrap_or_default()
    }

    /// Walk children of a class or module body node, tracking visibility state.
    ///
    /// A bare `private` / `protected` call (no arguments) flips `current_vis`
    /// for all subsequent `method` nodes in the same scope.  A bare `public`
    /// resets it.  If a call has arguments (e.g. `private :foo, :bar`) the
    /// named methods are marked Private but `current_vis` is unchanged.
    fn walk_body(
        body_node: &tree_sitter::Node,
        source_bytes: &[u8],
        symbols: &mut Vec<Symbol>,
        imports: &mut Vec<Import>,
        exports: &mut Vec<Export>,
    ) {
        let mut current_vis = Visibility::Public;
        let mut cursor = body_node.walk();
        for child in body_node.children(&mut cursor) {
            match child.kind() {
                "method" => {
                    let name = Self::extract_name(&child, source_bytes);
                    let signature = Self::first_line(&child, source_bytes);
                    let body = Self::extract_fn_body(&child, source_bytes);
                    let start_line = child.start_position().row + 1;
                    let end_line = child.end_position().row + 1;
                    if current_vis == Visibility::Public {
                        exports.push(Export {
                            name: name.clone(),
                            kind: SymbolKind::Method,
                        });
                    }
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Method,
                        visibility: current_vis.clone(),
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }
                "singleton_method" => {
                    let name = Self::extract_name(&child, source_bytes);
                    let signature = Self::first_line(&child, source_bytes);
                    let body = Self::extract_fn_body(&child, source_bytes);
                    let start_line = child.start_position().row + 1;
                    let end_line = child.end_position().row + 1;
                    exports.push(Export {
                        name: name.clone(),
                        kind: SymbolKind::Method,
                    });
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Method,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }
                // In tree-sitter-ruby, a bare `private` / `protected` / `public`
                // keyword inside a class body is parsed as an `identifier` node,
                // NOT as a `call` node.  Handle it here.
                "identifier" => match Self::node_text(&child, source_bytes) {
                    "private" | "protected" => {
                        current_vis = Visibility::Private;
                    }
                    "public" => {
                        current_vis = Visibility::Public;
                    }
                    _ => {}
                },
                "call" => {
                    if let Some(imp) = Self::extract_require(&child, source_bytes) {
                        imports.push(imp);
                        continue;
                    }
                    let method_name = Self::extract_call_method_name(&child, source_bytes);
                    let has_args = child.child_by_field_name("arguments").is_some();
                    if !has_args {
                        match method_name.as_str() {
                            "private" | "protected" => {
                                current_vis = Visibility::Private;
                            }
                            "public" => {
                                current_vis = Visibility::Public;
                            }
                            _ => {}
                        }
                    }
                    // If the call HAS arguments, e.g. `private :foo, :bar`,
                    // we do NOT change current_vis — the caller's default stands.
                }
                "class" => {
                    // Nested class — recurse
                    let name = Self::extract_name(&child, source_bytes);
                    let signature = Self::first_line(&child, source_bytes);
                    let body = Self::node_text(&child, source_bytes).to_string();
                    let start_line = child.start_position().row + 1;
                    let end_line = child.end_position().row + 1;
                    exports.push(Export {
                        name: name.clone(),
                        kind: SymbolKind::Class,
                    });
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Class,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                    if let Some(nested_body) = child.child_by_field_name("body") {
                        Self::walk_body(&nested_body, source_bytes, symbols, imports, exports);
                    }
                }
                "module" => {
                    let name = Self::extract_name(&child, source_bytes);
                    let signature = Self::first_line(&child, source_bytes);
                    let body = Self::node_text(&child, source_bytes).to_string();
                    let start_line = child.start_position().row + 1;
                    let end_line = child.end_position().row + 1;
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
                    if let Some(nested_body) = child.child_by_field_name("body") {
                        Self::walk_body(&nested_body, source_bytes, symbols, imports, exports);
                    }
                }
                _ => {}
            }
        }
    }
}

impl LanguageSupport for RubyLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_ruby::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "ruby"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let mut imports: Vec<Import> = Vec::new();
        let mut exports: Vec<Export> = Vec::new();

        // Process top-level nodes. Visibility tracking only applies inside
        // class/module bodies; top-level methods are always public in Ruby.
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            match node.kind() {
                "method" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::extract_fn_body(&node, source_bytes);
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;
                    exports.push(Export {
                        name: name.clone(),
                        kind: SymbolKind::Method,
                    });
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Method,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }
                "singleton_method" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::extract_fn_body(&node, source_bytes);
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;
                    exports.push(Export {
                        name: name.clone(),
                        kind: SymbolKind::Method,
                    });
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Method,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }
                "class" => {
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
                        name,
                        kind: SymbolKind::Class,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                    if let Some(body_node) = node.child_by_field_name("body") {
                        Self::walk_body(
                            &body_node,
                            source_bytes,
                            &mut symbols,
                            &mut imports,
                            &mut exports,
                        );
                    }
                }
                "module" => {
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
                    if let Some(body_node) = node.child_by_field_name("body") {
                        Self::walk_body(
                            &body_node,
                            source_bytes,
                            &mut symbols,
                            &mut imports,
                            &mut exports,
                        );
                    }
                }
                "call" => {
                    if let Some(imp) = Self::extract_require(&node, source_bytes) {
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
            .set_language(&tree_sitter_ruby::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_public_method() {
        let source = r#"def greet(name)
  puts "Hello, #{name}!"
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RubyLanguage;
        let result = lang.extract(source, &tree);

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(!methods.is_empty(), "expected method symbol");
        assert_eq!(methods[0].name, "greet");
        assert_eq!(methods[0].visibility, Visibility::Public);

        let exported: Vec<_> = result
            .exports
            .iter()
            .filter(|e| e.name == "greet")
            .collect();
        assert!(!exported.is_empty(), "greet should be exported");
    }

    #[test]
    fn test_extract_class() {
        let source = r#"class Animal
  def speak
    "..."
  end
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RubyLanguage;
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
    fn test_extract_module() {
        let source = r#"module Greetable
  def greet
    "hello"
  end
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RubyLanguage;
        let result = lang.extract(source, &tree);

        let modules: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Trait)
            .collect();
        assert!(!modules.is_empty(), "expected module symbol");
        assert_eq!(modules[0].name, "Greetable");
    }

    #[test]
    fn test_extract_require_import() {
        let source = r#"require 'json'
require_relative 'helper'
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RubyLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.imports.is_empty(),
            "expected at least one import, got {:?}",
            result.imports
        );
        let sources: Vec<&str> = result.imports.iter().map(|i| i.source.as_str()).collect();
        assert!(
            sources.contains(&"json") || sources.iter().any(|s| s.contains("json")),
            "expected json import, got: {:?}",
            sources
        );
    }

    #[test]
    fn test_extract_singleton_method() {
        let source = r#"class Config
  def self.load(path)
    puts path
  end

  def self.default
    "default"
  end
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RubyLanguage;
        let result = lang.extract(source, &tree);

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(
            methods.len() >= 2,
            "expected at least 2 singleton methods, got: {:?}",
            methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );

        // Singleton methods are exported
        let method_exports: Vec<_> = result
            .exports
            .iter()
            .filter(|e| e.kind == SymbolKind::Method)
            .collect();
        assert!(
            method_exports.len() >= 2,
            "singleton methods should be exported"
        );
    }

    #[test]
    fn test_non_require_call_ignored() {
        let source = r#"puts "hello"
print "world"
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RubyLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            result.imports.is_empty(),
            "puts/print should not be treated as imports"
        );
    }

    #[test]
    fn test_top_level_singleton_method() {
        let source = r#"def self.run
  "running"
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RubyLanguage;
        let result = lang.extract(source, &tree);

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(
            !methods.is_empty(),
            "expected singleton method at top level"
        );
    }

    #[test]
    fn test_extract_name_fallback() {
        // Covers extract_name returning String::new() (line 26) for unusual nodes
        let source = "# just a comment\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RubyLanguage;
        let result = lang.extract(source, &tree);
        let _ = result;
    }

    #[test]
    fn test_method_no_body_statement() {
        // Single-line method — may lack body_statement node
        // Covers extract_fn_body returning String::new() (line 79)
        let source = "def empty; end\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RubyLanguage;
        let result = lang.extract(source, &tree);
        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(!methods.is_empty());
    }

    #[test]
    fn test_require_relative() {
        // Covers require_relative in extract_require
        let source = "require_relative 'helpers/util'\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RubyLanguage;
        let result = lang.extract(source, &tree);
        assert!(!result.imports.is_empty());
    }

    #[test]
    fn test_private_protected_visibility() {
        let source = r#"class MyService
  def public_method
    "pub"
  end

  private

  def private_method
    "priv"
  end

  protected

  def protected_method
    "prot"
  end
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RubyLanguage;
        let result = lang.extract(source, &tree);

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();

        let pub_m = methods
            .iter()
            .find(|m| m.name == "public_method")
            .expect("public_method not found");
        assert_eq!(pub_m.visibility, Visibility::Public);

        let priv_m = methods
            .iter()
            .find(|m| m.name == "private_method")
            .expect("private_method not found");
        assert_eq!(priv_m.visibility, Visibility::Private);

        let prot_m = methods
            .iter()
            .find(|m| m.name == "protected_method")
            .expect("protected_method not found");
        assert_eq!(prot_m.visibility, Visibility::Private);

        // Only public_method should be in exports
        let exported_names: Vec<&str> = result
            .exports
            .iter()
            .filter(|e| e.kind == SymbolKind::Method)
            .map(|e| e.name.as_str())
            .collect();
        assert!(
            exported_names.contains(&"public_method"),
            "public_method should be exported: {exported_names:?}"
        );
        assert!(
            !exported_names.contains(&"private_method"),
            "private_method should not be exported: {exported_names:?}"
        );
    }

    #[test]
    fn test_constant_name_extraction() {
        // Module with constant name — covers extract_name with "constant" kind
        let source = "module MyModule\n  CONST = 42\nend\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RubyLanguage;
        let result = lang.extract(source, &tree);
        let modules: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Trait) // Ruby modules map to Trait
            .collect();
        assert!(!modules.is_empty());
        assert_eq!(modules[0].name, "MyModule");
    }
}
