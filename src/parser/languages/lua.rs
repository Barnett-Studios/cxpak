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
        let path = {
            let start = text.find("require")?;
            let after_require = &text[start + 7..];
            let trimmed = after_require
                .trim()
                .trim_start_matches('(')
                .trim()
                .trim_matches('"')
                .trim_matches('\'');
            let end = trimmed.find(['"', '\'', ')']).unwrap_or(trimmed.len());
            trimmed[..end].to_string()
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
                        // AST: variable_declaration -> assignment_statement -> variable_list -> identifier
                        let mut inner_cursor = node.walk();
                        for child in node.children(&mut inner_cursor) {
                            if child.kind() == "assignment_statement" {
                                // Look for variable_list inside assignment_statement
                                let mut assign_cursor = child.walk();
                                for assign_child in child.children(&mut assign_cursor) {
                                    if assign_child.kind() == "variable_list" {
                                        let name = Self::extract_name(&assign_child, source_bytes);
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

    #[test]
    fn test_coverage_multiple_requires() {
        let source = r#"local json = require("cjson")
local socket = require("socket")
local lfs = require("lfs")
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = LuaLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            result.imports.len() >= 3,
            "expected at least 3 require imports, got: {:?}",
            result.imports
        );
    }

    #[test]
    fn test_coverage_dotted_require() {
        let source = "local http = require(\"socket.http\")\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = LuaLanguage;
        let result = lang.extract(source, &tree);

        assert!(!result.imports.is_empty(), "expected dotted require import");
        let imp = &result.imports[0];
        assert_eq!(imp.source, "socket.http");
        assert!(
            imp.names.iter().any(|n| n == "http"),
            "expected short name 'http', got: {:?}",
            imp.names
        );
    }

    #[test]
    fn test_coverage_table_function() {
        let source = r#"local M = {}

function M.greet(name)
    print("Hello, " .. name)
end

function M.farewell(name)
    print("Goodbye, " .. name)
end

return M
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
            !funcs.is_empty(),
            "expected table-style function declarations, got: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
        // Table functions (M.greet) are global/public
        for f in &funcs {
            assert_eq!(
                f.visibility,
                Visibility::Public,
                "M.func should be public (not local)"
            );
        }
    }

    #[test]
    fn test_coverage_nested_function() {
        // Nested functions defined inside outer function
        // Only top-level functions are extracted
        let source = r#"function outer()
    local function inner()
        return 42
    end
    return inner()
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
        assert!(!funcs.is_empty(), "expected at least outer function");
        // outer should be public
        let outer = funcs.iter().find(|f| f.name == "outer");
        assert!(outer.is_some(), "expected 'outer' function");
        if let Some(f) = outer {
            assert_eq!(f.visibility, Visibility::Public);
        }
    }

    #[test]
    fn test_coverage_local_function_private() {
        let source = r#"local function private_helper(x)
    return x * 2
end

function public_api(x)
    return private_helper(x) + 1
end
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = LuaLanguage;
        let result = lang.extract(source, &tree);

        let private_fn = result.symbols.iter().find(|s| s.name == "private_helper");
        assert!(private_fn.is_some(), "expected private_helper function");
        if let Some(f) = private_fn {
            assert_eq!(f.visibility, Visibility::Private);
        }

        let public_fn = result.symbols.iter().find(|s| s.name == "public_api");
        assert!(public_fn.is_some(), "expected public_api function");
        if let Some(f) = public_fn {
            assert_eq!(f.visibility, Visibility::Public);
        }

        // Only public function should be exported
        assert!(
            result.exports.iter().any(|e| e.name == "public_api"),
            "public_api should be exported"
        );
        assert!(
            !result.exports.iter().any(|e| e.name == "private_helper"),
            "private_helper should not be exported"
        );
    }

    #[test]
    fn test_coverage_require_with_single_quotes() {
        let source = "local yaml = require('yaml')\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = LuaLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.imports.is_empty(),
            "expected require with single quotes"
        );
        assert_eq!(result.imports[0].source, "yaml");
    }

    #[test]
    fn test_method_index_expression() {
        // function M:method(x) uses method_index_expression
        let source = "function M:greet(name)\n    print(\"Hello\")\nend\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = LuaLanguage;
        let result = lang.extract(source, &tree);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(!funcs.is_empty(), "expected method-style function");
        assert!(
            funcs[0].name.contains("greet"),
            "name should contain greet, got: {}",
            funcs[0].name
        );
        assert_eq!(funcs[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_local_function_variable() {
        // local f = function(x) exercises the variable_declaration + function branch
        let source = "local f = function(x)\n    return x * 2\nend\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = LuaLanguage;
        let result = lang.extract(source, &tree);

        let vars: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .collect();
        assert!(
            !vars.is_empty(),
            "expected variable symbol for local function var, got symbols: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
        assert_eq!(vars[0].name, "f");
        assert_eq!(vars[0].visibility, Visibility::Private);
    }

    #[test]
    fn test_extract_require_import_no_require() {
        // A function call that isn't require should not produce import
        let source = "print(\"hello\")\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = LuaLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            result.imports.is_empty(),
            "print() should not produce import"
        );
    }

    #[test]
    fn test_extract_name_empty() {
        // Exercise extract_name on a node with no identifier/dot_index/method_index child
        let source = "function greet(name)\n    print(name)\nend\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        // Root (chunk) has no identifier child
        let name = LuaLanguage::extract_name(&root, source.as_bytes());
        assert!(name.is_empty(), "chunk should have no identifier child");
    }

    #[test]
    fn test_extract_fn_body_no_block() {
        // Exercise extract_fn_body on a node with no block child
        let source = "function greet(name)\n    print(name)\nend\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        // Call on root which has no block child
        let body = LuaLanguage::extract_fn_body(&root, source.as_bytes());
        assert!(body.is_empty(), "root should have no block child");
    }

    #[test]
    fn test_variable_declaration_without_function() {
        // local x = 42 should not produce a variable symbol (no function keyword)
        let source = "local x = 42\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = LuaLanguage;
        let result = lang.extract(source, &tree);

        // Should not have any symbols (not a function, not a require)
        let vars: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .collect();
        assert!(
            vars.is_empty(),
            "local x = 42 should not produce variable symbol"
        );
    }

    #[test]
    fn test_variable_declaration_with_require_not_function() {
        // local x = require("foo") should produce import but not variable
        let source = "local x = require(\"foo\")\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = LuaLanguage;
        let result = lang.extract(source, &tree);

        assert!(!result.imports.is_empty(), "require should produce import");
        // Should NOT produce a variable symbol (contains require, not pure function)
        let vars: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .collect();
        assert!(
            vars.is_empty(),
            "require assignment should not produce variable symbol"
        );
    }

    #[test]
    fn test_is_local_function_false() {
        // Global function should return false for is_local_function
        let source = "function greet()\n    print(\"hi\")\nend\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        let func = root.child(0).unwrap();
        assert_eq!(func.kind(), "function_declaration");
        assert!(
            !LuaLanguage::is_local_function(&func, source.as_bytes()),
            "global function should not be local"
        );
    }
}
