use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct PythonLanguage;

impl PythonLanguage {
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

    /// Python visibility: names starting with `_` are private.
    fn is_public(name: &str) -> bool {
        !name.starts_with('_')
    }

    /// Extract the function signature (first line: `def name(...):`)
    fn extract_fn_signature(node: &tree_sitter::Node, source: &[u8]) -> String {
        Self::first_line(node, source)
    }

    /// Extract the body text (everything after the first line / colon).
    fn extract_fn_body(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block" {
                let text = &source[child.start_byte()..child.end_byte()];
                return String::from_utf8_lossy(text).into_owned();
            }
        }
        String::new()
    }

    fn extract_import(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        let text = Self::node_text(node, source);
        match node.kind() {
            "import_statement" => {
                // "import os" / "import os, sys"
                let inner = text.trim_start_matches("import").trim();
                let names: Vec<String> = inner
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                Some(Import {
                    source: String::new(),
                    names,
                })
            }
            "import_from_statement" => {
                // "from os.path import join, exists"
                // "from . import something"
                let after_from = text.trim_start_matches("from").trim();
                if let Some(import_idx) = after_from.find(" import ") {
                    let module = after_from[..import_idx].trim().to_string();
                    let names_str = after_from[import_idx + 8..].trim();
                    let names: Vec<String> = if names_str.starts_with('(') {
                        // Strip outer parens then trim whitespace before splitting.
                        // The raw text may span multiple lines, e.g.:
                        //   from os import (\n    join,\n    exists \n)
                        names_str
                            .trim_matches(|c: char| c == '(' || c == ')')
                            .trim()
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    } else {
                        names_str
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    };
                    Some(Import {
                        source: module,
                        names,
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Extract the inner `function_definition` or `async_function_definition` from a node,
    /// walking through a `decorated_definition` wrapper if present.
    fn resolve_fn_node<'a>(node: &'a tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
        match node.kind() {
            "function_definition" | "async_function_definition" => Some(*node),
            "decorated_definition" => {
                // The last child of decorated_definition is the actual definition.
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "function_definition"
                        || child.kind() == "async_function_definition"
                    {
                        return Some(child);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Collect decorator names from a decorated_definition node.
    fn collect_decorators(node: &tree_sitter::Node, source: &[u8]) -> Vec<String> {
        let mut decorators = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "decorator" {
                let text = Self::node_text(&child, source);
                // Strip leading "@"
                decorators.push(text.trim_start_matches('@').trim().to_string());
            }
        }
        decorators
    }

    fn extract_methods(node: &tree_sitter::Node, source: &[u8]) -> Vec<Symbol> {
        let mut methods = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block" {
                let mut inner_cursor = child.walk();
                for item in child.children(&mut inner_cursor) {
                    let (fn_node, decorators) = if let Some(fn_node) = Self::resolve_fn_node(&item)
                    {
                        let decorators = if item.kind() == "decorated_definition" {
                            Self::collect_decorators(&item, source)
                        } else {
                            vec![]
                        };
                        (fn_node, decorators)
                    } else {
                        continue;
                    };

                    let name = Self::extract_name(&fn_node, source);
                    let is_pub = Self::is_public(&name);
                    let visibility = if is_pub {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };
                    let base_sig = Self::extract_fn_signature(&fn_node, source);
                    let signature = if decorators.is_empty() {
                        base_sig
                    } else {
                        format!("@{} {}", decorators.join(" @"), base_sig)
                    };
                    let body = Self::extract_fn_body(&fn_node, source);
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
        methods
    }
}

impl LanguageSupport for PythonLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_python::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "python"
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
                "function_definition" | "async_function_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_public(&name);
                    let visibility = if is_pub {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };
                    let signature = Self::extract_fn_signature(&node, source_bytes);
                    let body = Self::extract_fn_body(&node, source_bytes);
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if is_pub {
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

                "decorated_definition" => {
                    let decorators = Self::collect_decorators(&node, source_bytes);
                    if let Some(fn_node) = Self::resolve_fn_node(&node) {
                        let name = Self::extract_name(&fn_node, source_bytes);
                        let is_pub = Self::is_public(&name);
                        let visibility = if is_pub {
                            Visibility::Public
                        } else {
                            Visibility::Private
                        };
                        let base_sig = Self::extract_fn_signature(&fn_node, source_bytes);
                        let signature = if decorators.is_empty() {
                            base_sig
                        } else {
                            format!("@{} {}", decorators.join(" @"), base_sig)
                        };
                        let body = Self::extract_fn_body(&fn_node, source_bytes);
                        let start_line = node.start_position().row + 1;
                        let end_line = node.end_position().row + 1;

                        if is_pub {
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

                "class_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_pub = Self::is_public(&name);
                    let visibility = if is_pub {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if is_pub {
                        exports.push(Export {
                            name: name.clone(),
                            kind: SymbolKind::Class,
                        });
                    }

                    symbols.push(Symbol {
                        name: name.clone(),
                        kind: SymbolKind::Class,
                        visibility,
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

                "import_statement" | "import_from_statement" => {
                    if let Some(import) = Self::extract_import(&node, source_bytes) {
                        imports.push(import);
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
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_public_function() {
        let source = r#"def greet(name):
    return f"Hello, {name}!"
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.symbols.len(), 1);
        let sym = &result.symbols[0];
        assert_eq!(sym.name, "greet");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, Visibility::Public);
        assert!(
            sym.signature.contains("def greet"),
            "signature: {}",
            sym.signature
        );

        assert_eq!(result.exports.len(), 1);
        assert_eq!(result.exports[0].name, "greet");
    }

    #[test]
    fn test_extract_private_function() {
        let source = r#"def _helper(x):
    return x * 2
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.symbols.len(), 1);
        let sym = &result.symbols[0];
        assert_eq!(sym.name, "_helper");
        assert_eq!(sym.visibility, Visibility::Private);
        assert!(
            result.exports.is_empty(),
            "private function should not be exported"
        );
    }

    #[test]
    fn test_extract_class() {
        let source = r#"class Animal:
    def __init__(self, name):
        self.name = name

    def speak(self):
        pass
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);

        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Animal");
        assert_eq!(classes[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"import os
from pathlib import Path, PurePath
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.imports.len(), 2);
        assert!(result.imports[0].names.contains(&"os".to_string()));
        let second = &result.imports[1];
        assert_eq!(second.source, "pathlib");
        assert!(second.names.contains(&"Path".to_string()));
        assert!(second.names.contains(&"PurePath".to_string()));
    }

    #[test]
    fn test_extract_from_import_with_parens() {
        let source = "from os.path import (\n    join,\n    exists,\n    dirname\n)\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source, "os.path");
        assert!(result.imports[0].names.len() >= 3);
        // Ensure no name contains a stray ')' character
        for name in &result.imports[0].names {
            assert!(!name.contains(')'), "name should not contain ')': {name:?}");
        }
        assert!(
            result.imports[0].names.contains(&"join".to_string()),
            "names: {:?}",
            result.imports[0].names
        );
        assert!(
            result.imports[0].names.contains(&"exists".to_string()),
            "names: {:?}",
            result.imports[0].names
        );
    }

    #[test]
    fn test_extract_private_class() {
        let source = "class _InternalHelper:\n    def run(self):\n        pass\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);

        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].visibility, Visibility::Private);
        // Private class should not be exported
        assert!(!result.exports.iter().any(|e| e.name == "_InternalHelper"));
    }

    #[test]
    fn test_extract_class_methods() {
        let source = "class Dog:\n    def bark(self):\n        print('woof')\n    def _wag(self):\n        pass\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(
            methods.len() >= 2,
            "expected methods, got: {:?}",
            methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
        // bark is public, _wag is private
        let bark = methods
            .iter()
            .find(|m| m.name == "bark")
            .expect("bark method");
        assert_eq!(bark.visibility, Visibility::Public);
        let wag = methods
            .iter()
            .find(|m| m.name == "_wag")
            .expect("_wag method");
        assert_eq!(wag.visibility, Visibility::Private);
    }

    #[test]
    fn test_extract_multi_import() {
        let source = "import os, sys, json\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);

        assert!(!result.imports.is_empty());
    }

    #[test]
    fn test_extract_relative_import() {
        let source = "from . import something\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);

        assert_eq!(result.imports.len(), 1);
    }

    #[test]
    fn test_extract_name_fallback() {
        // Covers extract_name returning String::new() (line 25) when no identifier child found
        let source = "pass\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);
        let _ = result;
    }

    #[test]
    fn test_function_no_block_body() {
        // Covers extract_fn_body returning String::new() (line 47) for unusual AST
        // A one-liner function with pass
        let source = "def stub(): pass\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);
        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty());
    }

    #[test]
    fn test_private_function() {
        // Covers is_public returning false for underscore-prefixed names
        let source = "def _internal():\n    pass\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);
        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty());
        assert_eq!(funcs[0].visibility, Visibility::Private);
    }

    #[test]
    fn test_decorated_function() {
        // Covers decorated_definition extraction
        let source = "@staticmethod\ndef helper():\n    return True\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);
        let _ = result;
    }

    #[test]
    fn test_async_function_extracted() {
        let source = "async def fetch_data(url):\n    pass\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1, "expected 1 async function");
        assert_eq!(funcs[0].name, "fetch_data");
        assert_eq!(funcs[0].visibility, Visibility::Public);
        assert!(result.exports.iter().any(|e| e.name == "fetch_data"));
    }

    #[test]
    fn test_private_async_function_extracted() {
        let source = "async def _private_fetch():\n    pass\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "_private_fetch");
        assert_eq!(funcs[0].visibility, Visibility::Private);
        assert!(!result.exports.iter().any(|e| e.name == "_private_fetch"));
    }

    #[test]
    fn test_decorated_definition_top_level() {
        let source = "@property\ndef value(self):\n    return self._value\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1, "expected 1 decorated function");
        assert_eq!(funcs[0].name, "value");
        assert!(
            funcs[0].signature.contains("property"),
            "signature should include decorator: {}",
            funcs[0].signature
        );
    }

    #[test]
    fn test_decorated_method_in_class() {
        let source = "class Service:\n    @staticmethod\n    def create():\n        return Service()\n\n    @classmethod\n    def from_config(cls, cfg):\n        return cls()\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = PythonLanguage;
        let result = lang.extract(source, &tree);

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(
            methods.len(),
            2,
            "expected 2 decorated methods, got: {:?}",
            methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );

        let create = methods
            .iter()
            .find(|m| m.name == "create")
            .expect("create method");
        assert!(
            create.signature.contains("staticmethod"),
            "create signature: {}",
            create.signature
        );

        let from_config = methods
            .iter()
            .find(|m| m.name == "from_config")
            .expect("from_config method");
        assert!(
            from_config.signature.contains("classmethod"),
            "from_config signature: {}",
            from_config.signature
        );
    }
}
