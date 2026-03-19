use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct ElixirLanguage;

impl ElixirLanguage {
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
            if child.kind() == "identifier" || child.kind() == "atom" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    /// Extract the function/macro name from a `call` node representing def/defp/defmacro/defmodule.
    /// The structure is: call -> arguments -> (first argument is call node with the fn name, or atom)
    fn extract_def_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        // In Elixir tree-sitter grammar, `def foo(x)` parses as:
        //   (call target: (identifier "def") arguments: (arguments (call target: (identifier "foo") ...)))
        // or for no-arg: (call target: (identifier "def") arguments: (arguments (identifier "foo")))
        let args = match node.child_by_field_name("arguments") {
            Some(a) => a,
            None => {
                // Fallback: iterate children looking for arguments
                let mut cursor = node.walk();
                let mut found = None;
                for child in node.children(&mut cursor) {
                    if child.kind() == "arguments" {
                        found = Some(child);
                        break;
                    }
                }
                match found {
                    Some(a) => a,
                    None => return String::new(),
                }
            }
        };

        let mut cursor = args.walk();
        for child in args.children(&mut cursor) {
            match child.kind() {
                "call" => {
                    // def foo(args) — the first child of arguments is a call node
                    // whose target is the function name
                    return Self::extract_call_target(&child, source);
                }
                "identifier" => {
                    return Self::node_text(&child, source).to_string();
                }
                "atom" => {
                    return Self::node_text(&child, source)
                        .trim_start_matches(':')
                        .to_string();
                }
                "binary_operator" => {
                    // Pattern like `def foo(x) when is_integer(x)`
                    // The left side has the call
                    let mut inner_cursor = child.walk();
                    for inner in child.children(&mut inner_cursor) {
                        if inner.kind() == "call" {
                            return Self::extract_call_target(&inner, source);
                        }
                        if inner.kind() == "identifier" {
                            return Self::node_text(&inner, source).to_string();
                        }
                    }
                }
                _ => {}
            }
        }
        String::new()
    }

    /// Extract the target (function name) from a call node.
    fn extract_call_target(node: &tree_sitter::Node, source: &[u8]) -> String {
        if let Some(target) = node.child_by_field_name("target") {
            return Self::node_text(&target, source).to_string();
        }
        // Fallback: first identifier child
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    /// Extract module name from defmodule call.
    /// `defmodule MyApp.Router do ... end`
    fn extract_module_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let args = match node.child_by_field_name("arguments") {
            Some(a) => a,
            None => {
                let mut cursor = node.walk();
                let mut found = None;
                for child in node.children(&mut cursor) {
                    if child.kind() == "arguments" {
                        found = Some(child);
                        break;
                    }
                }
                match found {
                    Some(a) => a,
                    None => return String::new(),
                }
            }
        };

        let mut cursor = args.walk();
        for child in args.children(&mut cursor) {
            match child.kind() {
                "alias" => {
                    return Self::node_text(&child, source).to_string();
                }
                "atom" => {
                    return Self::node_text(&child, source)
                        .trim_start_matches(':')
                        .to_string();
                }
                "identifier" => {
                    return Self::node_text(&child, source).to_string();
                }
                "dot" => {
                    // Dotted module name like MyApp.Router
                    return Self::node_text(&child, source).to_string();
                }
                _ => {}
            }
        }
        String::new()
    }

    /// Extract import source from alias/import/use calls.
    fn extract_import_from_call(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        let target_text = Self::extract_call_target(node, source);
        if target_text != "alias"
            && target_text != "import"
            && target_text != "use"
            && target_text != "require"
        {
            return None;
        }

        let args = match node.child_by_field_name("arguments") {
            Some(a) => a,
            None => {
                let mut cursor = node.walk();
                let mut found = None;
                for child in node.children(&mut cursor) {
                    if child.kind() == "arguments" {
                        found = Some(child);
                        break;
                    }
                }
                found?
            }
        };

        let mut cursor = args.walk();
        for child in args.children(&mut cursor) {
            match child.kind() {
                "alias" | "atom" | "identifier" => {
                    let name = Self::node_text(&child, source).to_string();
                    if !name.is_empty() {
                        let short = name.rsplit('.').next().unwrap_or(&name).to_string();
                        return Some(Import {
                            source: name,
                            names: vec![short],
                        });
                    }
                }
                "dot" => {
                    let name = Self::node_text(&child, source).to_string();
                    if !name.is_empty() {
                        let short = name.rsplit('.').next().unwrap_or(&name).to_string();
                        return Some(Import {
                            source: name,
                            names: vec![short],
                        });
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Check if a call node is a def/defp/defmacro/defmacrop/defmodule.
    fn call_target_text(node: &tree_sitter::Node, source: &[u8]) -> String {
        Self::extract_call_target(node, source)
    }
}

impl LanguageSupport for ElixirLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_elixir::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "elixir"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let mut imports: Vec<Import> = Vec::new();
        let mut exports: Vec<Export> = Vec::new();

        // Use a stack to walk into do blocks and module bodies
        let mut stack: Vec<tree_sitter::Node> = root.children(&mut root.walk()).collect();

        while let Some(node) = stack.pop() {
            if node.kind() == "call" {
                let target = Self::call_target_text(&node, source_bytes);

                match target.as_str() {
                    "defmodule" => {
                        let name = Self::extract_module_name(&node, source_bytes);
                        let signature = Self::first_line(&node, source_bytes);
                        let body = Self::node_text(&node, source_bytes).to_string();
                        let start_line = node.start_position().row + 1;
                        let end_line = node.end_position().row + 1;

                        if !name.is_empty() {
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
                        }

                        // Recurse into the do block
                        Self::push_do_children(&node, &mut stack);
                    }

                    "def" | "defmacro" => {
                        let name = Self::extract_def_name(&node, source_bytes);
                        let signature = Self::first_line(&node, source_bytes);
                        let body = Self::node_text(&node, source_bytes).to_string();
                        let start_line = node.start_position().row + 1;
                        let end_line = node.end_position().row + 1;

                        let _target = target; // used in guard above
                        let kind = SymbolKind::Function;

                        if !name.is_empty() {
                            exports.push(Export {
                                name: name.clone(),
                                kind: kind.clone(),
                            });
                            symbols.push(Symbol {
                                name,
                                kind,
                                visibility: Visibility::Public,
                                signature,
                                body,
                                start_line,
                                end_line,
                            });
                        }
                    }

                    "defp" | "defmacrop" => {
                        let name = Self::extract_def_name(&node, source_bytes);
                        let signature = Self::first_line(&node, source_bytes);
                        let body = Self::node_text(&node, source_bytes).to_string();
                        let start_line = node.start_position().row + 1;
                        let end_line = node.end_position().row + 1;

                        if !name.is_empty() {
                            symbols.push(Symbol {
                                name,
                                kind: SymbolKind::Function,
                                visibility: Visibility::Private,
                                signature,
                                body,
                                start_line,
                                end_line,
                            });
                        }
                    }

                    "alias" | "import" | "use" | "require" => {
                        if let Some(imp) = Self::extract_import_from_call(&node, source_bytes) {
                            imports.push(imp);
                        }
                    }

                    _ => {
                        // Recurse into unknown calls that might contain do blocks with defs
                        Self::push_do_children(&node, &mut stack);
                    }
                }
            } else {
                // For non-call nodes, push all children to continue scanning
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    stack.push(child);
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

impl ElixirLanguage {
    /// Push children of `do_block` nodes into the stack for further processing.
    fn push_do_children<'a>(node: &tree_sitter::Node<'a>, stack: &mut Vec<tree_sitter::Node<'a>>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "do_block" {
                let mut inner_cursor = child.walk();
                for inner in child.children(&mut inner_cursor) {
                    stack.push(inner);
                }
            }
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
            .set_language(&tree_sitter_elixir::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_public_function() {
        let source = r#"defmodule MyApp do
  def greet(name) do
    "Hello, #{name}!"
  end
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ElixirLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function && s.name == "greet")
            .collect();
        assert!(!funcs.is_empty(), "expected public function 'greet'");
        assert_eq!(funcs[0].visibility, Visibility::Public);

        let exported: Vec<_> = result
            .exports
            .iter()
            .filter(|e| e.name == "greet")
            .collect();
        assert!(!exported.is_empty(), "public function should be exported");
    }

    #[test]
    fn test_extract_private_function() {
        let source = r#"defmodule MyApp do
  defp helper(x) do
    x * 2
  end
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ElixirLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function && s.name == "helper")
            .collect();
        assert!(!funcs.is_empty(), "expected private function 'helper'");
        assert_eq!(funcs[0].visibility, Visibility::Private);

        assert!(
            !result.exports.iter().any(|e| e.name == "helper"),
            "private function should not be exported"
        );
    }

    #[test]
    fn test_extract_module() {
        let source = r#"defmodule MyApp.Router do
  def index do
    :ok
  end
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ElixirLanguage;
        let result = lang.extract(source, &tree);

        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert!(!classes.is_empty(), "expected module as class symbol");
        assert_eq!(classes[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"defmodule MyApp do
  alias MyApp.Repo
  import Ecto.Query
  use GenServer
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ElixirLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.imports.is_empty(),
            "expected imports from alias/import/use, got: {:?}",
            result.imports
        );
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = ElixirLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_module_with_macro() {
        let source = r#"defmodule MyApp.Helpers do
  defmacro debug(msg) do
    quote do
      IO.puts(unquote(msg))
    end
  end

  def run do
    debug("starting")
  end

  defp internal do
    :ok
  end
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ElixirLanguage;
        let result = lang.extract(source, &tree);

        // Should have module + functions
        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert!(!classes.is_empty(), "expected module");

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(
            funcs.len() >= 2,
            "expected at least 2 functions (def + defmacro), got: {:?}",
            funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
        );

        // Private function should exist but not be exported
        let private_funcs: Vec<_> = funcs
            .iter()
            .filter(|f| f.visibility == Visibility::Private)
            .collect();
        assert!(
            !private_funcs.is_empty(),
            "expected private function 'internal'"
        );
    }
}
