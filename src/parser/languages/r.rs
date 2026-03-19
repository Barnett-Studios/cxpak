use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct RLanguage;

impl RLanguage {
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

    /// Check if the RHS of an assignment is a function definition.
    fn is_function_assignment(node: &tree_sitter::Node, _source: &[u8]) -> bool {
        // Look for assignments like: my_func <- function(x) { ... }
        // The RHS should be a "function_definition" node
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "function_definition" {
                return true;
            }
        }
        false
    }

    /// Extract function body from a function_definition node.
    fn extract_fn_body(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "brace_list" || child.kind() == "braced_expression" {
                let text = &source[child.start_byte()..child.end_byte()];
                return String::from_utf8_lossy(text).into_owned();
            }
        }
        String::new()
    }

    /// Extract the function_definition node from an assignment RHS.
    #[allow(clippy::manual_find)]
    fn find_function_def<'a>(node: &'a tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "function_definition" {
                return Some(child);
            }
        }
        None
    }

    /// Check if a call node is a library() or require() call, and extract the package name.
    /// tree-sitter-r: call -> identifier("library") + arguments -> argument -> identifier("pkg")
    fn extract_library_import(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        // First child is the function name, find it by kind
        let mut func_name = String::new();
        let mut args_node: Option<tree_sitter::Node> = None;

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "identifier" if func_name.is_empty() => {
                    func_name = Self::node_text(&child, source).to_string();
                }
                "arguments" => {
                    args_node = Some(child);
                }
                _ => {}
            }
        }

        if func_name != "library" && func_name != "require" {
            return None;
        }

        let args = args_node?;
        let mut args_cursor = args.walk();
        for child in args.children(&mut args_cursor) {
            // Drill into argument nodes
            if child.kind() == "argument" {
                let mut arg_cursor = child.walk();
                for arg_child in child.children(&mut arg_cursor) {
                    let kind = arg_child.kind();
                    if kind == "identifier" || kind == "string" {
                        let text = Self::node_text(&arg_child, source)
                            .trim_matches('"')
                            .trim_matches('\'')
                            .to_string();
                        if !text.is_empty() {
                            return Some(Import {
                                source: text.clone(),
                                names: vec![text],
                            });
                        }
                    }
                }
            }
            // Also handle direct children (some grammars don't wrap in argument)
            let kind = child.kind();
            if kind == "identifier" || kind == "string" {
                let text = Self::node_text(&child, source)
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                if !text.is_empty() && text != "," && text != "(" && text != ")" {
                    return Some(Import {
                        source: text.clone(),
                        names: vec![text],
                    });
                }
            }
        }
        None
    }
}

impl LanguageSupport for RLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_r::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "r"
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
                // Assignments: my_func <- function(x) { ... }
                // tree-sitter-r uses "binary_operator" for both <- and = assignments
                "left_assignment" | "equals_assignment" | "right_assignment"
                | "binary_operator" => {
                    if Self::is_function_assignment(&node, source_bytes) {
                        let name = Self::extract_name(&node, source_bytes);
                        let signature = Self::first_line(&node, source_bytes);
                        let body = if let Some(fn_def) = Self::find_function_def(&node) {
                            Self::extract_fn_body(&fn_def, source_bytes)
                        } else {
                            String::new()
                        };
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

                "call" => {
                    if let Some(imp) = Self::extract_library_import(&node, source_bytes) {
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
            .set_language(&tree_sitter_r::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_function() {
        let source = r#"my_func <- function(x) {
    x + 1
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected function symbol");
        assert_eq!(funcs[0].name, "my_func");
        assert_eq!(funcs[0].visibility, Visibility::Public);

        assert!(!result.exports.is_empty());
        assert_eq!(result.exports[0].name, "my_func");
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"library(ggplot2)
require(dplyr)
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.imports.is_empty(),
            "expected imports from library/require calls"
        );
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RLanguage;
        let result = lang.extract(source, &tree);

        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_snippet() {
        let source = r#"library(tidyverse)
library(ggplot2)

add <- function(a, b) {
    a + b
}

multiply <- function(a, b) {
    a * b
}

result <- add(1, 2)
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(
            funcs.len() >= 2,
            "expected at least 2 functions, got: {:?}",
            funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        assert!(
            result.imports.len() >= 2,
            "expected at least 2 imports, got: {:?}",
            result.imports
        );
    }

    #[test]
    fn test_non_function_assignment_ignored() {
        let source = r#"x <- 42
name <- "hello"
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            result.symbols.is_empty(),
            "non-function assignments should not produce symbols"
        );
    }

    #[test]
    fn test_equals_assignment_function() {
        let source = r#"greet = function(name) {
    paste("Hello,", name)
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = RLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected function from = assignment");
    }
}
