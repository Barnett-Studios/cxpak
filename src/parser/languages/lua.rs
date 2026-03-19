use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct LuaLanguage;

impl LuaLanguage {
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
            // Handle dotted names like M.func
            if child.kind() == "dot_index_expression" || child.kind() == "method_index_expression" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

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

    /// Check if a function_declaration is local (i.e., preceded by `local` keyword).
    fn is_local_function(node: &tree_sitter::Node, source: &[u8]) -> bool {
        let text = Self::node_text(node, source);
        text.trim_start().starts_with("local")
    }

    /// Extract `require` calls as imports from a function_call or variable_declaration node.
    fn extract_require_import(node: &tree_sitter::Node, source: &[u8]) -> Option<Import> {
        let text = Self::node_text(node, source);

        // Match patterns like: require("module"), require "module", require 'module'
        if !text.contains("require") {
            return None;
        }

        // Try to extract the module path from the require call
        let path = if let Some(start) = text.find("require") {
            let after_require = &text[start + 7..];
            let trimmed = after_require
                .trim()
                .trim_start_matches('(')
                .trim()
                .trim_matches('"')
                .trim_matches('\'');
            let end = trimmed.find(['"', '\'', ')']).unwrap_or(trimmed.len());
            trimmed[..end].to_string()
        } else {
            return None;
        };

        if path.is_empty() {
            return None;
        }

        let name = path.rsplit('.').next().unwrap_or(&path).to_string();

        Some(Import {
            source: path,
            names: vec![name],
        })
    }
}

impl LanguageSupport for LuaLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_lua::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "lua"
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
                "function_declaration" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let is_local = Self::is_local_function(&node, source_bytes);
                    let visibility = if is_local {
                        Visibility::Private
                    } else {
                        Visibility::Public
                    };
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::extract_fn_body(&node, source_bytes);
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if !is_local {
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

                "function_call" => {
                    if let Some(imp) = Self::extract_require_import(&node, source_bytes) {
                        imports.push(imp);
                    }
                }

                "variable_declaration" => {
                    // Check for require calls in variable assignments:
                    // local json = require("json")
                    if let Some(imp) = Self::extract_require_import(&node, source_bytes) {
                        imports.push(imp);
                    }

                    // Also extract local variables assigned to functions as symbols
                    let text = Self::node_text(&node, source_bytes);
                    if text.contains("function") && !text.contains("require") {
                        // local function-like variable: local f = function(...) end
                        let mut inner_cursor = node.walk();
                        for child in node.children(&mut inner_cursor) {
                            if child.kind() == "assignment_statement"
                                || child.kind() == "variable_list"
                            {
                                let name = Self::extract_name(&child, source_bytes);
                                if !name.is_empty() {
                                    let start_line = node.start_position().row + 1;
                                    let end_line = node.end_position().row + 1;
                                    symbols.push(Symbol {
                                        name,
                                        kind: SymbolKind::Variable,
                                        visibility: Visibility::Private,
                                        signature: Self::first_line(&node, source_bytes),
                                        body: String::new(),
                                        start_line,
                                        end_line,
                                    });
                                }
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
            .set_language(&tree_sitter_lua::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_global_function() {
        let source = r#"function greet(name)
    print("Hello, " .. name)
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = LuaLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected function symbol");
        assert_eq!(funcs[0].name, "greet");
        assert_eq!(funcs[0].visibility, Visibility::Public);

        assert!(
            result.exports.iter().any(|e| e.name == "greet"),
            "greet should be exported"
        );
    }

    #[test]
    fn test_extract_local_function() {
        let source = r#"local function helper(x)
    return x * 2
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = LuaLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected local function symbol");
        assert_eq!(funcs[0].name, "helper");
        assert_eq!(funcs[0].visibility, Visibility::Private);

        assert!(
            result.exports.is_empty(),
            "local function should not be exported"
        );
    }

    #[test]
    fn test_extract_require_import() {
        let source = "local json = require(\"json\")\nlocal utils = require('utils')\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = LuaLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.imports.is_empty(),
            "expected require imports, got: {:?}",
            result.imports
        );
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = LuaLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
    }

    #[test]
    fn test_complex_lua() {
        let source = r#"local http = require("socket.http")

local function log(msg)
    print("[LOG] " .. msg)
end

function setup()
    log("Setting up...")
end

function run(config)
    setup()
    log("Running with config: " .. tostring(config))
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = LuaLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(
            funcs.len() >= 3,
            "expected at least 3 functions, got: {:?}",
            funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
        );

        // log is local/private, setup and run are public
        let log_fn = funcs.iter().find(|f| f.name == "log");
        if let Some(log_fn) = log_fn {
            assert_eq!(log_fn.visibility, Visibility::Private);
        }

        let setup_fn = funcs.iter().find(|f| f.name == "setup");
        if let Some(setup_fn) = setup_fn {
            assert_eq!(setup_fn.visibility, Visibility::Public);
        }

        assert!(!result.imports.is_empty(), "expected require import");
    }

    #[test]
    fn test_standalone_require() {
        let source = "require(\"lfs\")\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = LuaLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.imports.is_empty(),
            "expected standalone require import"
        );
    }
}
