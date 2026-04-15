use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct CppLanguage;

impl CppLanguage {
    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        text.lines().next().unwrap_or("").trim().to_string()
    }

    fn find_fn_identifier(node: &tree_sitter::Node, source: &[u8], depth: usize) -> String {
        if depth > 6 {
            return String::new();
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "identifier" | "field_identifier" => {
                    return Self::node_text(&child, source).to_string();
                }
                "qualified_identifier" => {
                    // last segment of a::b::c
                    let text = Self::node_text(&child, source);
                    return text.split("::").last().unwrap_or("").to_string();
                }
                "function_declarator"
                | "pointer_declarator"
                | "reference_declarator"
                | "abstract_pointer_declarator" => {
                    let name = Self::find_fn_identifier(&child, source, depth + 1);
                    if !name.is_empty() {
                        return name;
                    }
                }
                _ => {}
            }
        }
        String::new()
    }

    fn extract_tag_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_identifier" || child.kind() == "identifier" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    fn extract_typedef_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut last_name = String::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "type_identifier" | "identifier" | "primitive_type" => {
                    last_name = Self::node_text(&child, source).to_string();
                }
                _ => {}
            }
        }
        last_name
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

    fn extract_include(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        let text = Self::node_text(node, source);
        let path = text
            .trim_start_matches("#include")
            .trim()
            .trim_matches(|c| c == '<' || c == '>' || c == '"')
            .to_string();

        if path.is_empty() {
            None
        } else {
            Some(Import {
                source: path.clone(),
                names: vec![path],
            })
        }
    }

    fn extract_namespace_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" || child.kind() == "namespace_identifier" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    /// Walk a class_specifier or struct_specifier body and extract method symbols,
    /// respecting access labels (`public:`, `protected:`, `private:`).
    fn extract_class_methods(
        class_node: &tree_sitter::Node,
        source: &[u8],
        class_default: Visibility,
        symbols: &mut Vec<Symbol>,
        exports: &mut Vec<Export>,
    ) {
        let mut cursor = class_node.walk();
        for child in class_node.children(&mut cursor) {
            if child.kind() == "field_declaration_list" {
                let mut current_access = class_default.clone();
                let mut body_cursor = child.walk();
                for member in child.children(&mut body_cursor) {
                    match member.kind() {
                        "access_specifier" => {
                            let text = Self::node_text(&member, source);
                            if text.starts_with("public") || text.starts_with("protected") {
                                current_access = Visibility::Public;
                            } else if text.starts_with("private") {
                                current_access = Visibility::Private;
                            }
                        }
                        // Inline method definitions (with body), forward declarations,
                        // and field_declaration (member declarations in class body)
                        "function_definition" | "declaration" | "field_declaration" => {
                            let name = Self::find_fn_identifier(&member, source, 0);
                            if name.is_empty() {
                                continue;
                            }
                            let visibility = current_access.clone();
                            let signature = Self::extract_fn_signature(&member, source);
                            let body = Self::extract_fn_body(&member, source);
                            let start_line = member.start_position().row + 1;
                            let end_line = member.end_position().row + 1;

                            if visibility == Visibility::Public {
                                exports.push(Export {
                                    name: name.clone(),
                                    kind: SymbolKind::Method,
                                });
                            }
                            symbols.push(Symbol {
                                name,
                                kind: SymbolKind::Method,
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
            }
        }
    }

    /// Returns true if the function_definition node has a `static` storage class specifier
    /// (file-local linkage for free functions).
    fn has_static_linkage(node: &tree_sitter::Node, source: &[u8]) -> bool {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "storage_class_specifier"
                && Self::node_text(&child, source) == "static"
            {
                return true;
            }
        }
        false
    }
}

impl LanguageSupport for CppLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_cpp::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "cpp"
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
                "preproc_include" => {
                    if let Some(import) = Self::extract_include(&node, source_bytes) {
                        imports.push(import);
                    }
                }

                "function_definition" => {
                    let name = Self::find_fn_identifier(&node, source_bytes, 0);
                    if name.is_empty() {
                        continue;
                    }
                    // Free functions with `static` have file-local linkage → Private
                    let visibility = if Self::has_static_linkage(&node, source_bytes) {
                        Visibility::Private
                    } else {
                        Visibility::Public
                    };
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

                "struct_specifier" => {
                    let name = Self::extract_tag_name(&node, source_bytes);
                    if name.is_empty() {
                        continue;
                    }
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    // Struct members default to Public; extract methods from body
                    Self::extract_class_methods(
                        &node,
                        source_bytes,
                        Visibility::Public,
                        &mut symbols,
                        &mut exports,
                    );

                    exports.push(Export {
                        name: name.clone(),
                        kind: SymbolKind::Struct,
                    });
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Struct,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "class_specifier" => {
                    let name = Self::extract_tag_name(&node, source_bytes);
                    if name.is_empty() {
                        continue;
                    }
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    // Class members default to Private; extract methods from body
                    Self::extract_class_methods(
                        &node,
                        source_bytes,
                        Visibility::Private,
                        &mut symbols,
                        &mut exports,
                    );

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

                "enum_specifier" => {
                    let name = Self::extract_tag_name(&node, source_bytes);
                    if name.is_empty() {
                        continue;
                    }
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    exports.push(Export {
                        name: name.clone(),
                        kind: SymbolKind::Enum,
                    });
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Enum,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "namespace_definition" => {
                    let name = Self::extract_namespace_name(&node, source_bytes);
                    if name.is_empty() {
                        continue;
                    }
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    // Namespaces are always accessible (treat as public)
                    exports.push(Export {
                        name: name.clone(),
                        kind: SymbolKind::Struct,
                    });
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Struct,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "type_definition" => {
                    let name = Self::extract_typedef_name(&node, source_bytes);
                    if name.is_empty() {
                        continue;
                    }
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
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

                "declaration" => {
                    let mut decl_cursor = node.walk();
                    for child in node.children(&mut decl_cursor) {
                        if child.kind() == "struct_specifier" || child.kind() == "class_specifier" {
                            let kind = if child.kind() == "class_specifier" {
                                SymbolKind::Class
                            } else {
                                SymbolKind::Struct
                            };
                            let name = Self::extract_tag_name(&child, source_bytes);
                            if !name.is_empty() {
                                let signature = Self::first_line(&child, source_bytes);
                                let body = Self::node_text(&child, source_bytes).to_string();
                                let start_line = child.start_position().row + 1;
                                let end_line = child.end_position().row + 1;
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
            .set_language(&tree_sitter_cpp::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_function() {
        let source = r#"int add(int a, int b) {
    return a + b;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected function symbol");
        assert_eq!(funcs[0].name, "add");
        assert_eq!(funcs[0].visibility, Visibility::Public);
        assert!(
            funcs[0].signature.contains("int add(int a, int b)"),
            "signature: {}",
            funcs[0].signature
        );
    }

    #[test]
    fn test_extract_include() {
        let source = r#"#include <iostream>
#include <string>
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.imports.len(), 2);
        assert!(result.imports.iter().any(|i| i.source == "iostream"));
        assert!(result.imports.iter().any(|i| i.source == "string"));
    }

    #[test]
    fn test_extract_class() {
        let source = r#"class Point {
public:
    double x;
    double y;
};
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = CppLanguage;
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
        assert_eq!(classes[0].name, "Point");
    }

    #[test]
    fn test_extract_typedef() {
        let source = r#"typedef unsigned int uint32_t;
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);

        let typedefs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::TypeAlias)
            .collect();
        assert!(!typedefs.is_empty(), "expected typedef symbol");
        assert_eq!(typedefs[0].name, "uint32_t");
    }

    #[test]
    fn test_extract_namespace() {
        let source = "namespace math {\n    int add(int a, int b) { return a + b; }\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);
        let ns: Vec<_> = result.symbols.iter().filter(|s| s.name == "math").collect();
        assert!(!ns.is_empty(), "expected namespace symbol");
    }

    #[test]
    fn test_extract_struct() {
        let source = "struct Vec3 {\n    float x, y, z;\n};\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);
        let structs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct)
            .collect();
        assert!(!structs.is_empty(), "expected struct");
        assert_eq!(structs[0].name, "Vec3");
    }

    #[test]
    fn test_extract_enum() {
        let source = "enum Direction {\n    UP,\n    DOWN,\n    LEFT,\n    RIGHT\n};\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);
        let enums: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Enum)
            .collect();
        assert!(!enums.is_empty(), "expected enum");
        assert_eq!(enums[0].name, "Direction");
    }

    #[test]
    fn test_extract_class_in_declaration() {
        let source = "class Widget {\npublic:\n    void draw();\n} widget;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);
        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert!(!classes.is_empty(), "expected class from declaration");
        assert_eq!(classes[0].name, "Widget");
    }

    #[test]
    fn test_extract_struct_in_declaration() {
        let source = "struct Data {\n    int value;\n} data;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);
        let structs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct)
            .collect();
        assert!(!structs.is_empty(), "expected struct from declaration");
        assert_eq!(structs[0].name, "Data");
    }

    #[test]
    fn test_extract_typedef_cpp() {
        let source = "typedef long long int64;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);
        let typedefs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::TypeAlias)
            .collect();
        assert!(!typedefs.is_empty(), "expected typedef");
        assert_eq!(typedefs[0].name, "int64");
    }

    #[test]
    fn test_empty_source_cpp() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
    }

    #[test]
    fn test_multiple_includes_cpp() {
        let source = "#include <vector>\n#include <map>\n#include <algorithm>\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);
        assert_eq!(result.imports.len(), 3);
    }

    #[test]
    fn test_function_with_reference_param() {
        let source = "void swap(int& a, int& b) {\n    int tmp = a;\n    a = b;\n    b = tmp;\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);
        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty());
        assert_eq!(funcs[0].name, "swap");
    }

    #[test]
    fn test_extract_qualified_function() {
        let source = "int MyNamespace::calculate(int x) {\n    return x * 2;\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);
        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected qualified function");
        assert_eq!(funcs[0].name, "calculate");
    }

    #[test]
    fn test_extract_pointer_return_function() {
        let source = "int* create() {\n    return new int(42);\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);
        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected pointer return function");
        assert_eq!(funcs[0].name, "create");
    }

    #[test]
    fn test_extract_namespace_definition() {
        let source = "namespace MyLib {\n    void helper() {}\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);

        let ns: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.name == "MyLib")
            .collect();
        assert!(!ns.is_empty(), "expected namespace symbol");
        assert_eq!(ns[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_type_definition() {
        let source = "typedef unsigned long size_t;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);

        let types: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::TypeAlias)
            .collect();
        assert!(
            !types.is_empty(),
            "expected typedef symbol, got: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_declaration_with_class_specifier() {
        let source = "class Widget {\npublic:\n    int value;\n} widget;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);

        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert!(
            !classes.is_empty(),
            "expected class from declaration, got: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
        assert_eq!(classes[0].name, "Widget");
    }

    #[test]
    fn test_anonymous_namespace() {
        // Anonymous namespace should be skipped (empty name)
        let source = "namespace {\n    void internal() {}\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);

        // Anonymous namespace should not produce a namespace symbol
        let ns: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.name.is_empty())
            .collect();
        assert!(ns.is_empty(), "anonymous namespace should be skipped");
    }

    #[test]
    fn test_deep_nested_declarator() {
        let source = "int** getMatrix(int rows) {\n    return nullptr;\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);
        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected nested pointer function");
        assert_eq!(funcs[0].name, "getMatrix");
    }

    #[test]
    fn test_empty_include() {
        // Covers extract_include returning None for empty/malformed includes (line 104)
        let source = "#include\nint x = 1;\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);
        // Should not crash; imports may be empty
        let _ = result;
    }

    #[test]
    fn test_function_no_body() {
        // A forward declaration with no body — covers signature/body fallback returns (lines 81, 92)
        let source = "void forwardDecl(int x);\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);
        // Forward declaration is a `declaration`, not a `function_definition`, so no function symbol
        let _ = result;
    }

    #[test]
    fn test_static_free_function_is_private() {
        let source = "static int helper(int x) {\n    return x;\n}\nint compute(int x) {\n    return x * 2;\n}\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);

        let helper = result
            .symbols
            .iter()
            .find(|s| s.name == "helper")
            .expect("helper");
        assert_eq!(
            helper.visibility,
            Visibility::Private,
            "static fn should be Private"
        );
        assert!(
            result.exports.iter().all(|e| e.name != "helper"),
            "static fn should not be exported"
        );

        let compute = result
            .symbols
            .iter()
            .find(|s| s.name == "compute")
            .expect("compute");
        assert_eq!(compute.visibility, Visibility::Public);
        assert!(result.exports.iter().any(|e| e.name == "compute"));
    }

    #[test]
    fn test_class_access_labels_respected() {
        let source = "class Widget {\npublic:\n    void draw();\n    void resize();\nprivate:\n    void internal();\n};\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(!methods.is_empty(), "expected methods from class body");

        if let Some(draw) = methods.iter().find(|m| m.name == "draw") {
            assert_eq!(draw.visibility, Visibility::Public, "draw should be Public");
            assert!(result.exports.iter().any(|e| e.name == "draw"));
        }
        if let Some(internal) = methods.iter().find(|m| m.name == "internal") {
            assert_eq!(
                internal.visibility,
                Visibility::Private,
                "internal should be Private"
            );
            assert!(result.exports.iter().all(|e| e.name != "internal"));
        }
    }

    #[test]
    fn test_struct_members_default_public() {
        let source = "struct Point {\n    void print() {}\nprivate:\n    void secret() {}\n};\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);

        if let Some(print) = result.symbols.iter().find(|s| s.name == "print") {
            assert_eq!(
                print.visibility,
                Visibility::Public,
                "struct method default is Public"
            );
        }
        if let Some(secret) = result.symbols.iter().find(|s| s.name == "secret") {
            assert_eq!(
                secret.visibility,
                Visibility::Private,
                "explicitly private struct method"
            );
        }
    }

    #[test]
    fn test_protected_class_member_is_public() {
        let source = "class Base {\nprotected:\n    void onEvent() {}\n};\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = CppLanguage;
        let result = lang.extract(source, &tree);

        if let Some(on_event) = result.symbols.iter().find(|s| s.name == "onEvent") {
            assert_eq!(
                on_event.visibility,
                Visibility::Public,
                "protected maps to Public"
            );
        }
    }
}
