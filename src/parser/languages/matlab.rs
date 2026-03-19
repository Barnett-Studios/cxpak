use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct MatlabLanguage;

impl MatlabLanguage {
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

    fn extract_fn_body(node: &tree_sitter::Node, source: &[u8]) -> String {
        // MATLAB function body is everything between the signature and "end"
        let text = Self::node_text(node, source);
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > 2 {
            lines[1..lines.len() - 1].join("\n")
        } else {
            String::new()
        }
    }

    /// Extract methods from within a class body.
    fn extract_methods(node: &tree_sitter::Node, source: &[u8]) -> Vec<Symbol> {
        let mut methods = Vec::new();
        let mut stack: Vec<tree_sitter::Node> = Vec::new();

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }

        while let Some(child) = stack.pop() {
            if child.kind() == "function_definition" {
                let name = Self::extract_name(&child, source);
                let signature = Self::first_line(&child, source);
                let body = Self::extract_fn_body(&child, source);
                let start_line = child.start_position().row + 1;
                let end_line = child.end_position().row + 1;

                methods.push(Symbol {
                    name,
                    kind: SymbolKind::Method,
                    visibility: Visibility::Public,
                    signature,
                    body,
                    start_line,
                    end_line,
                });
            } else {
                // Recurse into methods blocks etc.
                let mut inner = child.walk();
                for grandchild in child.children(&mut inner) {
                    stack.push(grandchild);
                }
            }
        }
        methods
    }
}

impl LanguageSupport for MatlabLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_matlab::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "matlab"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let imports: Vec<Import> = Vec::new();
        let mut exports: Vec<Export> = Vec::new();

        let mut stack: Vec<tree_sitter::Node> = root.children(&mut root.walk()).collect();

        while let Some(node) = stack.pop() {
            match node.kind() {
                "function_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
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

                "class_definition" => {
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
                        exports.push(Export {
                            name: method.name.clone(),
                            kind: SymbolKind::Method,
                        });
                    }
                    symbols.extend(methods);
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
            .set_language(&tree_sitter_matlab::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_function() {
        let source = r#"function result = add(a, b)
    result = a + b;
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = MatlabLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected function symbol");
        assert_eq!(funcs[0].visibility, Visibility::Public);
        assert!(!result.exports.is_empty());
    }

    #[test]
    fn test_extract_imports() {
        // MATLAB doesn't have a standard import mechanism
        let source = r#"function y = compute(x)
    y = x * 2;
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = MatlabLanguage;
        let result = lang.extract(source, &tree);

        assert!(result.imports.is_empty(), "MATLAB typically has no imports");
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = MatlabLanguage;
        let result = lang.extract(source, &tree);

        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_snippet() {
        let source = r#"function result = add(a, b)
    result = a + b;
end

function result = multiply(a, b)
    result = a * b;
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = MatlabLanguage;
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
    }

    #[test]
    fn test_all_public() {
        let source = r#"function y = helper(x)
    y = x + 1;
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = MatlabLanguage;
        let result = lang.extract(source, &tree);

        for sym in &result.symbols {
            assert_eq!(
                sym.visibility,
                Visibility::Public,
                "MATLAB symbols should all be public"
            );
        }
    }
}
