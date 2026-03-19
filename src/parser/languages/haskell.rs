use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct HaskellLanguage;

impl HaskellLanguage {
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
            match child.kind() {
                "variable"
                | "name"
                | "constructor"
                | "type"
                | "identifier"
                | "constructor_identifier"
                | "variable_identifier"
                | "type_name" => {
                    return Self::node_text(&child, source).to_string();
                }
                _ => {}
            }
        }
        String::new()
    }

    /// Extract function/binding name from a function or bind node.
    fn extract_bind_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        // Try to find the first variable or name child
        let name = Self::extract_name(node, source);
        if !name.is_empty() {
            return name;
        }

        // Fallback: extract from text
        let text = Self::node_text(node, source);
        let trimmed = text.trim();
        let name: String = trimmed
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '\'')
            .collect();
        name
    }

    /// Extract import module name and optional import list.
    fn extract_import_info(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        let text = Self::node_text(node, source);
        let trimmed = text.trim();

        // "import qualified Data.Map as Map"
        // "import Data.List (sort, nub)"
        // "import Data.Maybe"
        if !trimmed.starts_with("import") {
            return None;
        }

        let after_import = trimmed.strip_prefix("import").unwrap_or(trimmed).trim();
        let after_qualified = if let Some(rest) = after_import.strip_prefix("qualified") {
            rest.trim()
        } else {
            after_import
        };

        // Extract module name (capitalized identifier with dots)
        let module: String = after_qualified
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '.' || *c == '_')
            .collect();

        if module.is_empty() {
            return None;
        }

        // Check for explicit import list in parens
        let after_module = after_qualified[module.len()..].trim();
        let names = if after_module.starts_with('(') {
            let inner = after_module
                .trim_start_matches('(')
                .split(')')
                .next()
                .unwrap_or("");
            inner
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else if let Some(rest) = after_module.strip_prefix("as ") {
            // import qualified Foo as F
            let alias = rest.trim();
            let alias_name: String = alias
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            vec![alias_name]
        } else {
            let short = module.rsplit('.').next().unwrap_or(&module).to_string();
            vec![short]
        };

        Some(Import {
            source: module,
            names,
        })
    }

    /// Extract the type name from type/data/newtype/class declarations.
    fn extract_type_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        // First try: look for constructor or type child
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "name"
                | "type"
                | "constructor"
                | "type_name"
                | "constructor_identifier"
                | "simple_type" => {
                    let text = Self::node_text(&child, source);
                    // For simple_type "Maybe a", extract just the type constructor name
                    let name: String = text
                        .trim()
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() {
                        return name;
                    }
                }
                _ => {}
            }
        }

        // Fallback: parse from text
        let text = Self::node_text(node, source);
        let trimmed = text.trim();

        // Strip leading keywords
        let after_keyword = if let Some(rest) = trimmed.strip_prefix("data ") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("newtype ") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("type ") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("class ") {
            rest
        } else {
            trimmed
        };

        let name: String = after_keyword
            .trim()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        name
    }
}

impl LanguageSupport for HaskellLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_haskell::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "haskell"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let mut imports: Vec<Import> = Vec::new();
        let mut exports: Vec<Export> = Vec::new();

        // tree-sitter-haskell wraps top-level items in `declarations` nodes.
        // We use a stack to drill through them.
        let mut stack: Vec<tree_sitter::Node> = Vec::new();
        {
            let mut cursor = root.walk();
            for child in root.children(&mut cursor) {
                stack.push(child);
            }
        }

        while let Some(node) = stack.pop() {
            let kind = node.kind();

            match kind {
                // Wrapper nodes — drill into children
                "declarations" => {
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        stack.push(child);
                    }
                    continue;
                }

                // Function/value bindings
                "function" | "bind" | "function_declaration" | "value_binding" | "top_splice" => {
                    let text = Self::node_text(&node, source_bytes);
                    // Skip type signatures (lines with ::)
                    let first_line_text = text.lines().next().unwrap_or("");
                    if first_line_text.contains("::") && !first_line_text.contains("=") {
                        continue;
                    }

                    let name = Self::extract_bind_name(&node, source_bytes);
                    if name.is_empty() || name.starts_with("--") {
                        continue;
                    }

                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    // In Haskell, all module-level bindings are public
                    // (visibility is controlled through export lists)
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

                // Type aliases
                "type_alias" | "type_synonym_declaration" | "type_synomym" => {
                    let name = Self::extract_type_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if !name.is_empty() {
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

                // Data types and newtypes
                "data_type" | "newtype" | "adt" | "data_declaration" | "newtype_declaration" => {
                    let name = Self::extract_type_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if !name.is_empty() {
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
                }

                // Type classes
                "class" | "class_declaration" | "type_class_declaration" => {
                    let name = Self::extract_type_name(&node, source_bytes);
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
                }

                // Import declarations
                "import" | "import_declaration" => {
                    if let Some(imp) = Self::extract_import_info(&node, source_bytes) {
                        imports.push(imp);
                    }
                }

                // Signature declarations (type annotations) — skip, but don't error
                "signature" | "type_signature" => {}

                _ => {
                    // Check text for declarations that might be under different node kinds
                    let text = Self::node_text(&node, source_bytes);
                    let trimmed = text.trim();

                    if trimmed.starts_with("import ") {
                        if let Some(imp) = Self::extract_import_info(&node, source_bytes) {
                            if !imports.iter().any(|i| i.source == imp.source) {
                                imports.push(imp);
                            }
                        }
                    }

                    if trimmed.starts_with("data ") || trimmed.starts_with("newtype ") {
                        let name = Self::extract_type_name(&node, source_bytes);
                        if !name.is_empty() && !symbols.iter().any(|s| s.name == name) {
                            let sym_kind = SymbolKind::Struct;
                            let signature = Self::first_line(&node, source_bytes);
                            let body = text.to_string();
                            let start_line = node.start_position().row + 1;
                            let end_line = node.end_position().row + 1;

                            exports.push(Export {
                                name: name.clone(),
                                kind: sym_kind.clone(),
                            });
                            symbols.push(Symbol {
                                name,
                                kind: sym_kind,
                                visibility: Visibility::Public,
                                signature,
                                body,
                                start_line,
                                end_line,
                            });
                        }
                    }

                    if trimmed.starts_with("type ") && !trimmed.starts_with("type instance") {
                        let name = Self::extract_type_name(&node, source_bytes);
                        if !name.is_empty() && !symbols.iter().any(|s| s.name == name) {
                            let signature = Self::first_line(&node, source_bytes);
                            let body = text.to_string();
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

                    if trimmed.starts_with("class ") {
                        let name = Self::extract_type_name(&node, source_bytes);
                        if !name.is_empty() && !symbols.iter().any(|s| s.name == name) {
                            let signature = Self::first_line(&node, source_bytes);
                            let body = text.to_string();
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
            .set_language(&tree_sitter_haskell::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_function() {
        let source = r#"greet :: String -> String
greet name = "Hello, " ++ name
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = HaskellLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function && s.name == "greet")
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

        let exported: Vec<_> = result
            .exports
            .iter()
            .filter(|e| e.name == "greet")
            .collect();
        assert!(!exported.is_empty(), "function should be exported");
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"import Data.List (sort, nub)
import qualified Data.Map as Map
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = HaskellLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.imports.is_empty(),
            "expected imports, got: {:?}",
            result.imports
        );
    }

    #[test]
    fn test_extract_data_type() {
        let source = r#"data Color = Red | Green | Blue
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = HaskellLanguage;
        let result = lang.extract(source, &tree);

        let structs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct)
            .collect();
        assert!(
            !structs.is_empty(),
            "expected data type as Struct, got symbols: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = HaskellLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_haskell_module() {
        let source = r#"module Main where

import Data.Maybe

data Tree a = Leaf a | Branch (Tree a) (Tree a)

type Name = String

fmap :: (a -> b) -> Tree a -> Tree b
fmap f (Leaf x) = Leaf (f x)
fmap f (Branch l r) = Branch (fmap f l) (fmap f r)

main :: IO ()
main = putStrLn "Hello"
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = HaskellLanguage;
        let result = lang.extract(source, &tree);

        // Should have data type, type alias, and functions
        assert!(
            !result.symbols.is_empty(),
            "expected symbols from Haskell module"
        );

        // Imports
        assert!(
            !result.imports.is_empty(),
            "expected import from Data.Maybe"
        );
    }

    #[test]
    fn test_qualified_import() {
        let source = "import qualified Data.Map as Map\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = HaskellLanguage;
        let result = lang.extract(source, &tree);

        assert!(!result.imports.is_empty(), "expected qualified import");
        let map_import = result
            .imports
            .iter()
            .find(|i| i.source.contains("Data.Map"));
        assert!(
            map_import.is_some(),
            "expected Data.Map import, got: {:?}",
            result.imports
        );
    }
}
