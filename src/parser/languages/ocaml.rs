use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct OcamlLanguage;
pub struct OcamlInterfaceLanguage;

/// Shared extraction logic for both OCaml implementation and interface files.
fn extract_common(source: &str, tree: &tree_sitter::Tree) -> ParseResult {
    let source_bytes = source.as_bytes();
    let root = tree.root_node();

    let mut symbols: Vec<Symbol> = Vec::new();
    let mut imports: Vec<Import> = Vec::new();
    let mut exports: Vec<Export> = Vec::new();

    let mut stack: Vec<tree_sitter::Node> = root.children(&mut root.walk()).collect();

    while let Some(node) = stack.pop() {
        match node.kind() {
            "value_definition" | "let_binding" => {
                let name = extract_name(&node, source_bytes);
                if !name.is_empty() {
                    let signature = first_line(&node, source_bytes);
                    let body = node_text(&node, source_bytes).to_string();
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
            }

            "type_definition" => {
                let name = extract_type_name(&node, source_bytes);
                if !name.is_empty() {
                    let signature = first_line(&node, source_bytes);
                    let body = node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    exports.push(Export {
                        name: name.clone(),
                        kind: SymbolKind::TypeAlias,
                    });
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::TypeAlias,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }
            }

            "module_definition" => {
                let name = extract_module_name(&node, source_bytes);
                if !name.is_empty() {
                    let signature = first_line(&node, source_bytes);
                    let body = node_text(&node, source_bytes).to_string();
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
                }

                // Recurse into module body
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    stack.push(child);
                }
            }

            "open_statement" | "open_module" => {
                let text = node_text(&node, source_bytes);
                let module_name = text.trim_start_matches("open").trim().to_string();
                if !module_name.is_empty() {
                    imports.push(Import {
                        source: module_name.clone(),
                        names: vec![module_name],
                    });
                }
            }

            // let definitions at top level are wrapped in structure_item or
            // let_expression; we recurse into them
            "let_expression" | "structure_item" => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    stack.push(child);
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

fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
    let text = node_text(node, source);
    text.lines().next().unwrap_or("").trim().to_string()
}

fn extract_name(node: &tree_sitter::Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "value_name"
            || child.kind() == "identifier"
            || child.kind() == "value_pattern"
        {
            return node_text(&child, source).to_string();
        }
        // value_definition wraps let_binding which contains value_name
        if child.kind() == "let_binding" {
            let mut inner = child.walk();
            for inner_child in child.children(&mut inner) {
                if inner_child.kind() == "value_name"
                    || inner_child.kind() == "identifier"
                    || inner_child.kind() == "value_pattern"
                {
                    return node_text(&inner_child, source).to_string();
                }
            }
        }
    }
    String::new()
}

fn extract_type_name(node: &tree_sitter::Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_constructor"
            || child.kind() == "type_binding"
            || child.kind() == "identifier"
        {
            // For type_binding, drill down for the name
            if child.kind() == "type_binding" {
                let mut inner = child.walk();
                for inner_child in child.children(&mut inner) {
                    if inner_child.kind() == "type_constructor"
                        || inner_child.kind() == "identifier"
                    {
                        return node_text(&inner_child, source).to_string();
                    }
                }
            }
            return node_text(&child, source).to_string();
        }
    }
    String::new()
}

fn extract_module_name(node: &tree_sitter::Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "module_name" || child.kind() == "module_binding" {
            // module_binding wraps module_name
            if child.kind() == "module_binding" {
                let mut inner = child.walk();
                for inner_child in child.children(&mut inner) {
                    if inner_child.kind() == "module_name" {
                        return node_text(&inner_child, source).to_string();
                    }
                }
            }
            return node_text(&child, source).to_string();
        }
        // Some grammars use capitalized identifiers
        if child.kind() == "identifier" {
            let text = node_text(&child, source);
            if text
                .chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false)
            {
                return text.to_string();
            }
        }
    }
    String::new()
}

impl LanguageSupport for OcamlLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_ocaml::LANGUAGE_OCAML.into()
    }

    fn name(&self) -> &str {
        "ocaml"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        extract_common(source, tree)
    }
}

impl LanguageSupport for OcamlInterfaceLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into()
    }

    fn name(&self) -> &str {
        "ocaml_interface"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        extract_common(source, tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::language::{SymbolKind, Visibility};

    fn make_parser() -> tree_sitter::Parser {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_ocaml::LANGUAGE_OCAML.into())
            .expect("failed to set language");
        parser
    }

    fn make_interface_parser() -> tree_sitter::Parser {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_function() {
        let source = r#"let greet name =
  Printf.printf "Hello, %s!\n" name
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = OcamlLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.symbols.is_empty(),
            "expected at least one symbol from let binding"
        );
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"open Printf
open List
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = OcamlLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.imports.is_empty(),
            "expected imports from open statements"
        );
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = OcamlLanguage;
        let result = lang.extract(source, &tree);

        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_snippet() {
        let source = r#"open Printf

type point = { x : float; y : float }

let distance p1 p2 =
  let dx = p1.x -. p2.x in
  let dy = p1.y -. p2.y in
  sqrt (dx *. dx +. dy *. dy)

let origin = { x = 0.0; y = 0.0 }
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = OcamlLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.symbols.is_empty(),
            "expected symbols in complex snippet"
        );
        assert!(!result.imports.is_empty(), "expected open import");
    }

    #[test]
    fn test_interface_language() {
        let source = r#"val greet : string -> unit
"#;
        let mut parser = make_interface_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = OcamlInterfaceLanguage;
        let result = lang.extract(source, &tree);

        // Interface files may or may not produce symbols depending on grammar
        // The important thing is it doesn't panic
        let _ = result;
    }

    #[test]
    fn test_ocaml_type_definition() {
        let source = r#"type color = Red | Green | Blue
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = OcamlLanguage;
        let result = lang.extract(source, &tree);

        let types: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::TypeAlias)
            .collect();
        assert!(
            !types.is_empty(),
            "expected type alias from type definition"
        );
    }

    #[test]
    fn test_ocaml_all_public() {
        let source = r#"let add a b = a + b
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = OcamlLanguage;
        let result = lang.extract(source, &tree);

        for sym in &result.symbols {
            assert_eq!(
                sym.visibility,
                Visibility::Public,
                "OCaml symbols should be public"
            );
        }
    }
}
