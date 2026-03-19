use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct GroovyLanguage;

impl GroovyLanguage {
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
            if child.kind() == "identifier" || child.kind() == "simple_name" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    /// Determine visibility from access modifiers in the node text.
    fn determine_visibility(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
        let text = Self::node_text(node, source);
        let first_line = text.lines().next().unwrap_or("");

        // Check for explicit access modifiers
        if first_line.contains("private ") || first_line.contains("protected ") {
            Visibility::Private
        } else {
            // In Groovy, default visibility is public (unlike Java)
            Visibility::Public
        }
    }

    /// Extract function/method signature (everything before the body block).
    fn extract_fn_signature(node: &tree_sitter::Node, source: &[u8]) -> String {
        let full_text = Self::node_text(node, source);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block"
                || child.kind() == "closure"
                || child.kind() == "statement_block"
            {
                let body_start = child.start_byte() - node.start_byte();
                if body_start < full_text.len() {
                    return full_text[..body_start].trim().to_string();
                }
            }
        }
        full_text.lines().next().unwrap_or("").trim().to_string()
    }

    /// Extract the body block text.
    fn extract_fn_body(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block"
                || child.kind() == "closure"
                || child.kind() == "statement_block"
            {
                let text = &source[child.start_byte()..child.end_byte()];
                return String::from_utf8_lossy(text).into_owned();
            }
        }
        String::new()
    }

    /// Extract import declaration: import foo.bar.Baz or import static foo.Bar.*
    fn extract_import(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        let text = Self::node_text(node, source);
        let trimmed = text
            .trim()
            .trim_end_matches(';')
            .trim_start_matches("import")
            .trim()
            .trim_start_matches("static")
            .trim();

        if trimmed.is_empty() {
            return None;
        }

        // Handle wildcard: import foo.bar.*
        if trimmed.ends_with(".*") {
            let source_path = trimmed.trim_end_matches(".*").to_string();
            return Some(Import {
                source: source_path,
                names: vec!["*".to_string()],
            });
        }

        // Regular import: import foo.bar.Baz
        if let Some(last_dot) = trimmed.rfind('.') {
            let source_path = trimmed[..last_dot].to_string();
            let name = trimmed[last_dot + 1..].to_string();
            Some(Import {
                source: source_path,
                names: vec![name],
            })
        } else {
            Some(Import {
                source: String::new(),
                names: vec![trimmed.to_string()],
            })
        }
    }

    /// Extract class name, handling Groovy class declarations.
    fn extract_class_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        // Try direct child lookup
        let name = Self::extract_name(node, source);
        if !name.is_empty() && name != "class" {
            return name;
        }

        // Fallback: parse from text
        let text = Self::node_text(node, source);
        let trimmed = text.trim();

        // Find "class" keyword and extract the following identifier
        if let Some(class_pos) = trimmed.find("class ") {
            let after_class = &trimmed[class_pos + 6..];
            let name: String = after_class
                .trim()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            return name;
        }

        String::new()
    }

    /// Extract methods from a class body.
    fn extract_class_methods(node: &tree_sitter::Node, source: &[u8]) -> Vec<Symbol> {
        let mut methods = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "class_body" || child.kind() == "body" || child.kind() == "block" {
                let mut inner_cursor = child.walk();
                for item in child.children(&mut inner_cursor) {
                    if item.kind() == "method_declaration"
                        || item.kind() == "function_definition"
                        || item.kind() == "method_definition"
                    {
                        let name = Self::extract_name(&item, source);
                        let visibility = Self::determine_visibility(&item, source);
                        let signature = Self::extract_fn_signature(&item, source);
                        let body = Self::extract_fn_body(&item, source);
                        let start_line = item.start_position().row + 1;
                        let end_line = item.end_position().row + 1;

                        if !name.is_empty() {
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
        }
        methods
    }
}

impl LanguageSupport for GroovyLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_groovy::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "groovy"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let mut imports: Vec<Import> = Vec::new();
        let mut exports: Vec<Export> = Vec::new();

        // Use stack to walk deeper into class bodies
        let mut stack: Vec<tree_sitter::Node> = root.children(&mut root.walk()).collect();

        while let Some(node) = stack.pop() {
            let kind = node.kind();

            match kind {
                "method_declaration" | "function_definition" | "method_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let visibility = Self::determine_visibility(&node, source_bytes);
                    let signature = Self::extract_fn_signature(&node, source_bytes);
                    let body = Self::extract_fn_body(&node, source_bytes);
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if !name.is_empty() {
                        let sym_kind = SymbolKind::Function;
                        if visibility == Visibility::Public {
                            exports.push(Export {
                                name: name.clone(),
                                kind: sym_kind.clone(),
                            });
                        }
                        symbols.push(Symbol {
                            name,
                            kind: sym_kind,
                            visibility,
                            signature,
                            body,
                            start_line,
                            end_line,
                        });
                    }
                }

                "class_definition" | "class_declaration" => {
                    let name = Self::extract_class_name(&node, source_bytes);
                    let visibility = Self::determine_visibility(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if !name.is_empty() {
                        if visibility == Visibility::Public {
                            exports.push(Export {
                                name: name.clone(),
                                kind: SymbolKind::Class,
                            });
                        }
                        symbols.push(Symbol {
                            name,
                            kind: SymbolKind::Class,
                            visibility,
                            signature,
                            body,
                            start_line,
                            end_line,
                        });

                        // Extract methods from class body
                        let methods = Self::extract_class_methods(&node, source_bytes);
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
                }

                "import_declaration" | "import_statement" => {
                    if let Some(imp) = Self::extract_import(&node, source_bytes) {
                        imports.push(imp);
                    }
                }

                _ => {
                    // Check text for declarations under other node kinds
                    let text = Self::node_text(&node, source_bytes);
                    let trimmed = text.trim();

                    if trimmed.starts_with("import ") {
                        if let Some(imp) = Self::extract_import(&node, source_bytes) {
                            if !imports.iter().any(|i| i.source == imp.source) {
                                imports.push(imp);
                            }
                        }
                    }

                    if trimmed.starts_with("class ") || trimmed.contains("class ") {
                        let name = Self::extract_class_name(&node, source_bytes);
                        if !name.is_empty() && !symbols.iter().any(|s| s.name == name) {
                            let visibility = Self::determine_visibility(&node, source_bytes);
                            let signature = Self::first_line(&node, source_bytes);
                            let body = text.to_string();
                            let start_line = node.start_position().row + 1;
                            let end_line = node.end_position().row + 1;

                            if visibility == Visibility::Public {
                                exports.push(Export {
                                    name: name.clone(),
                                    kind: SymbolKind::Class,
                                });
                            }
                            symbols.push(Symbol {
                                name,
                                kind: SymbolKind::Class,
                                visibility,
                                signature,
                                body,
                                start_line,
                                end_line,
                            });

                            // Try to extract methods
                            let methods = Self::extract_class_methods(&node, source_bytes);
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
                    }

                    // Also try to match function definitions under other node kinds
                    if (trimmed.starts_with("def ")
                        || trimmed.starts_with("void ")
                        || trimmed.starts_with("public ")
                        || trimmed.starts_with("private "))
                        && (trimmed.contains("(") && trimmed.contains("{"))
                    {
                        // Looks like a method/function
                        let name = Self::extract_name(&node, source_bytes);
                        if !name.is_empty() && !symbols.iter().any(|s| s.name == name) {
                            let visibility = Self::determine_visibility(&node, source_bytes);
                            let signature = Self::extract_fn_signature(&node, source_bytes);
                            let body = Self::extract_fn_body(&node, source_bytes);
                            let start_line = node.start_position().row + 1;
                            let end_line = node.end_position().row + 1;

                            if visibility == Visibility::Public {
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
                    }

                    // Push children to continue scanning
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        stack.push(child);
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::language::{SymbolKind, Visibility};

    fn make_parser() -> tree_sitter::Parser {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_groovy::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_function() {
        let source = r#"def greet(String name) {
    println "Hello, ${name}!"
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GroovyLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| {
                (s.kind == SymbolKind::Function || s.kind == SymbolKind::Method)
                    && s.name == "greet"
            })
            .collect();
        assert!(
            !funcs.is_empty(),
            "expected function 'greet', got symbols: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
        assert_eq!(funcs[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_class() {
        let source = r#"class Animal {
    String name

    def speak() {
        return "..."
    }
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GroovyLanguage;
        let result = lang.extract(source, &tree);

        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert!(
            !classes.is_empty(),
            "expected class symbol, got: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"import groovy.json.JsonSlurper
import java.util.*
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GroovyLanguage;
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
        let lang = GroovyLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_groovy_class() {
        let source = r#"import groovy.transform.ToString

@ToString
class Person {
    String name
    int age

    def greet() {
        return "Hello, I'm ${name}"
    }

    private void helper() {
        // internal logic
    }
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GroovyLanguage;
        let result = lang.extract(source, &tree);

        // Should have class and possibly methods
        assert!(
            !result.symbols.is_empty(),
            "expected symbols from Groovy class"
        );

        // Should have import
        assert!(
            !result.imports.is_empty(),
            "expected import from groovy.transform"
        );
    }

    #[test]
    fn test_wildcard_import() {
        let source = "import java.util.*\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GroovyLanguage;
        let result = lang.extract(source, &tree);

        assert!(!result.imports.is_empty(), "expected wildcard import");
        let util_import = result
            .imports
            .iter()
            .find(|i| i.source.contains("java.util"));
        assert!(
            util_import.is_some(),
            "expected java.util import, got: {:?}",
            result.imports
        );
        if let Some(imp) = util_import {
            assert!(
                imp.names.contains(&"*".to_string()),
                "expected wildcard name, got: {:?}",
                imp.names
            );
        }
    }
}
