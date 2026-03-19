use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct ObjcLanguage;

impl ObjcLanguage {
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
            if child.kind() == "identifier" || child.kind() == "name" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    /// Extract the class name from a class_interface or class_implementation node.
    fn extract_class_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    /// Extract the method name from a method_declaration or method_definition node.
    fn extract_method_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "selector" || child.kind() == "identifier" {
                let text = Self::node_text(&child, source);
                // For selectors like "initWithName:", take first keyword
                return text.split(':').next().unwrap_or("").to_string();
            }
            if child.kind() == "keyword_selector" {
                let mut inner = child.walk();
                for kw in child.children(&mut inner) {
                    if kw.kind() == "keyword_declarator" {
                        let mut kw_cursor = kw.walk();
                        for kw_child in kw.children(&mut kw_cursor) {
                            if kw_child.kind() == "identifier" {
                                return Self::node_text(&kw_child, source).to_string();
                            }
                        }
                    }
                }
            }
        }
        String::new()
    }

    fn extract_fn_body(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "compound_statement" || child.kind() == "block" {
                let text = &source[child.start_byte()..child.end_byte()];
                return String::from_utf8_lossy(text).into_owned();
            }
        }
        String::new()
    }

    /// Extract import path from #import or #include preprocessor directive.
    fn extract_import_path(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            if kind == "string_literal" || kind == "system_lib_string" {
                let path = Self::node_text(&child, source)
                    .trim_matches('"')
                    .trim_matches('<')
                    .trim_matches('>')
                    .to_string();
                if !path.is_empty() {
                    let name = path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&path)
                        .trim_end_matches(".h")
                        .to_string();
                    return Some(Import {
                        source: path,
                        names: vec![name],
                    });
                }
            }
        }
        None
    }
}

impl LanguageSupport for ObjcLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_objc::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "objc"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let mut imports: Vec<Import> = Vec::new();
        let mut exports: Vec<Export> = Vec::new();

        let mut stack: Vec<tree_sitter::Node> = root.children(&mut root.walk()).collect();

        while let Some(node) = stack.pop() {
            match node.kind() {
                "class_interface" => {
                    let name = Self::extract_class_name(&node, source_bytes);
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

                    // Recurse into the interface to find method declarations
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        stack.push(child);
                    }
                }

                "class_implementation" => {
                    let name = Self::extract_class_name(&node, source_bytes);
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

                    // Recurse into the implementation to find method definitions
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        stack.push(child);
                    }
                }

                "method_declaration" => {
                    let name = Self::extract_method_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = String::new();
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

                "method_definition" => {
                    let name = Self::extract_method_name(&node, source_bytes);
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

                "preproc_import" | "preproc_include" => {
                    if let Some(imp) = Self::extract_import_path(&node, source_bytes) {
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
            .set_language(&tree_sitter_objc::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_function() {
        let source = r#"void greet(const char *name) {
    printf("Hello, %s!\n", name);
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ObjcLanguage;
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
        let source = r#"#import <Foundation/Foundation.h>
#include "MyHeader.h"
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ObjcLanguage;
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
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ObjcLanguage;
        let result = lang.extract(source, &tree);

        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_snippet() {
        let source = r#"#import <Foundation/Foundation.h>

@interface MyClass : NSObject
- (void)doSomething;
@end

@implementation MyClass
- (void)doSomething {
    NSLog(@"doing something");
}
@end

void helperFunction(int x) {
    printf("%d\n", x);
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ObjcLanguage;
        let result = lang.extract(source, &tree);

        // Should find classes and/or methods and the helper function
        assert!(
            !result.symbols.is_empty(),
            "expected symbols in complex snippet"
        );
        assert!(
            !result.imports.is_empty(),
            "expected import from #import directive"
        );
    }

    #[test]
    fn test_extract_class_interface() {
        let source = r#"@interface Dog : NSObject
- (void)bark;
@end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ObjcLanguage;
        let result = lang.extract(source, &tree);

        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert!(!classes.is_empty(), "expected class from @interface");
    }
}
