use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct JuliaLanguage;

impl JuliaLanguage {
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
            // function_definition: name is inside signature -> call_expression -> identifier
            if child.kind() == "signature" {
                let mut sig_cursor = child.walk();
                for sig_child in child.children(&mut sig_cursor) {
                    if sig_child.kind() == "call_expression" {
                        let mut call_cursor = sig_child.walk();
                        for call_child in sig_child.children(&mut call_cursor) {
                            if call_child.kind() == "identifier" {
                                return Self::node_text(&call_child, source).to_string();
                            }
                        }
                    }
                    if sig_child.kind() == "identifier" {
                        return Self::node_text(&sig_child, source).to_string();
                    }
                }
            }
            // struct_definition: name is inside type_head -> identifier
            if child.kind() == "type_head" {
                let mut th_cursor = child.walk();
                for th_child in child.children(&mut th_cursor) {
                    if th_child.kind() == "identifier" {
                        return Self::node_text(&th_child, source).to_string();
                    }
                }
            }
        }
        String::new()
    }

    fn extract_fn_body(node: &tree_sitter::Node, source: &[u8]) -> String {
        // Julia function bodies are typically everything between the signature and "end"
        let text = Self::node_text(node, source);
        // Skip the first line (signature) and last line ("end")
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > 2 {
            lines[1..lines.len() - 1].join("\n")
        } else {
            String::new()
        }
    }

    /// Extract import/using module names from an import or using statement.
    fn extract_import_names(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        let text = Self::node_text(node, source).trim().to_string();

        // Handle "import Foo" / "import Foo: bar, baz"
        // Handle "using Foo" / "using Foo: bar, baz"
        let stripped = text
            .trim_start_matches("import")
            .trim_start_matches("using")
            .trim();

        if stripped.is_empty() {
            return None;
        }

        if let Some(colon_idx) = stripped.find(':') {
            let module = stripped[..colon_idx].trim().to_string();
            let names: Vec<String> = stripped[colon_idx + 1..]
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            Some(Import {
                source: module,
                names,
            })
        } else {
            let names: Vec<String> = stripped
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let source_mod = names.first().cloned().unwrap_or_default();
            Some(Import {
                source: source_mod,
                names,
            })
        }
    }

    /// Collect exported names from `export` statements for later visibility checks.
    fn collect_exported_names(root: &tree_sitter::Node, source: &[u8]) -> Vec<String> {
        let mut exported = Vec::new();
        let mut stack: Vec<tree_sitter::Node> = root.children(&mut root.walk()).collect();

        while let Some(node) = stack.pop() {
            if node.kind() == "export_statement" {
                let text = Self::node_text(&node, source);
                let stripped = text.trim_start_matches("export").trim();
                for name in stripped.split(',') {
                    let trimmed = name.trim().to_string();
                    if !trimmed.is_empty() {
                        exported.push(trimmed);
                    }
                }
            }
            // Recurse into module bodies
            if node.kind() == "module_definition" {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    stack.push(child);
                }
            }
        }
        exported
    }
}

impl LanguageSupport for JuliaLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_julia::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "julia"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let mut imports: Vec<Import> = Vec::new();
        let mut exports: Vec<Export> = Vec::new();

        let exported_names = Self::collect_exported_names(&root, source_bytes);

        let mut stack: Vec<tree_sitter::Node> = root.children(&mut root.walk()).collect();

        while let Some(node) = stack.pop() {
            match node.kind() {
                "function_definition" | "short_function_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::extract_fn_body(&node, source_bytes);
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    let is_exported = exported_names.contains(&name);
                    let visibility = if is_exported {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };

                    if is_exported {
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

                "struct_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    let is_exported = exported_names.contains(&name);
                    let visibility = if is_exported {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };

                    if is_exported {
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

                "module_definition" => {
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

                    // Recurse into module body
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        stack.push(child);
                    }
                }

                "macro_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::extract_fn_body(&node, source_bytes);
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    let is_exported = exported_names.contains(&name);
                    let visibility = if is_exported {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };

                    if is_exported {
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

                "import_statement" => {
                    if let Some(imp) = Self::extract_import_names(&node, source_bytes) {
                        imports.push(imp);
                    }
                }

                "using_statement" => {
                    if let Some(imp) = Self::extract_import_names(&node, source_bytes) {
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
    use crate::parser::language::SymbolKind;

    fn make_parser() -> tree_sitter::Parser {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_julia::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_function() {
        let source = r#"function greet(name)
    println("Hello, $name!")
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JuliaLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected function symbol");
        assert_eq!(funcs[0].name, "greet");
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"using LinearAlgebra
import Base: show
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JuliaLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.imports.is_empty(),
            "expected imports from using/import statements"
        );
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JuliaLanguage;
        let result = lang.extract(source, &tree);

        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_snippet() {
        let source = r#"module MyModule

export greet

function greet(name)
    println("Hello, $name!")
end

function helper(x)
    x + 1
end

struct Point
    x::Float64
    y::Float64
end

end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JuliaLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.symbols.is_empty(),
            "expected symbols in complex snippet"
        );

        // Module should be found
        let modules: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert!(!modules.is_empty(), "expected module symbol");
    }

    #[test]
    fn test_extract_struct() {
        let source = r#"struct Point
    x::Float64
    y::Float64
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = JuliaLanguage;
        let result = lang.extract(source, &tree);

        let structs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct)
            .collect();
        assert!(!structs.is_empty(), "expected struct symbol");
        assert_eq!(structs[0].name, "Point");
    }
}
