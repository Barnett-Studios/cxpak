use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

use tree_sitter::Node;

pub struct LuauLanguage;

impl LuauLanguage {
    fn node_text<'a>(node: &Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn first_line(node: &Node, source: &[u8]) -> String {
        Self::node_text(node, source)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string()
    }

    /// `local function`, `export type`: the modifier is the node's first token.
    fn starts_with_token(node: &Node, token: &str) -> bool {
        node.child(0).is_some_and(|c| c.kind() == token)
    }

    /// `type X = …` names the alias directly; `type X<T> = …` names a `generic_type` wrapping it.
    fn type_name(node: &Node, source: &[u8]) -> String {
        let name = node.child_by_field_name("name").and_then(|n| {
            if n.kind() == "generic_type" {
                n.named_child(0)
            } else {
                Some(n)
            }
        });
        name.map(|n| Self::node_text(&n, source).to_string())
            .unwrap_or_default()
    }

    /// A static `require(...)` call as an import. The argument is a string path (`"@lune/fs"`) or
    /// an instance path (`script.Parent.Foo`), recorded verbatim; anything computed, such as
    /// `"dir/" .. name`, names no module and yields nothing.
    fn require_import(call: &Node, source: &[u8]) -> Option<Import> {
        let callee = call.child_by_field_name("name")?;
        if callee.kind() != "identifier" || Self::node_text(&callee, source) != "require" {
            return None;
        }
        let arg = call.child_by_field_name("arguments")?.named_child(0)?;
        let path = match arg.kind() {
            "string" => Self::node_text(&arg.child_by_field_name("content")?, source),
            "identifier" | "dot_index_expression" | "function_call" => {
                Self::node_text(&arg, source)
            }
            _ => return None,
        };
        let name = path
            .trim_end_matches(['"', '\'', ')'])
            .rsplit(['/', '.', ':', '"', '\'', '('])
            .next()
            .unwrap_or(path);
        if name.is_empty() {
            return None;
        }
        Some(Import {
            source: path.to_string(),
            names: vec![name.to_string()],
        })
    }

    /// `local a, f = 0, function() end`: each name paired with its value.
    fn function_valued_locals(node: &Node, source: &[u8], symbols: &mut Vec<Symbol>) {
        let mut cursor = node.walk();
        let Some(assignment) = node
            .named_children(&mut cursor)
            .find(|c| c.kind() == "assignment_statement")
        else {
            return;
        };
        let mut cursor = assignment.walk();
        let lists: Vec<Node> = assignment.named_children(&mut cursor).collect();
        let (Some(names), Some(values)) = (
            lists.iter().find(|c| c.kind() == "variable_list"),
            lists.iter().find(|c| c.kind() == "expression_list"),
        ) else {
            return;
        };
        let mut name_cursor = names.walk();
        let mut value_cursor = values.walk();
        for (name, value) in names
            .named_children(&mut name_cursor)
            .zip(values.named_children(&mut value_cursor))
        {
            if value.kind() == "function_definition" {
                symbols.push(Symbol {
                    name: Self::node_text(&name, source).to_string(),
                    kind: SymbolKind::Variable,
                    visibility: Visibility::Private,
                    signature: Self::first_line(node, source),
                    body: String::new(),
                    start_line: node.start_position().row + 1,
                    end_line: node.end_position().row + 1,
                });
            }
        }
    }
}

impl LanguageSupport for LuauLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_luau::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "luau"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source = source.as_bytes();
        let root = tree.root_node();
        let mut symbols = Vec::new();
        let mut exports = Vec::new();

        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            let (name, kind, public, body) = match node.kind() {
                "function_declaration" => {
                    let Some(name) = node.child_by_field_name("name") else {
                        continue;
                    };
                    let body = node
                        .child_by_field_name("body")
                        .map(|b| Self::node_text(&b, source).to_string())
                        .unwrap_or_default();
                    (
                        Self::node_text(&name, source).to_string(),
                        SymbolKind::Function,
                        !Self::starts_with_token(&node, "local"),
                        body,
                    )
                }
                "type_definition" => (
                    Self::type_name(&node, source),
                    SymbolKind::TypeAlias,
                    Self::starts_with_token(&node, "export"),
                    Self::node_text(&node, source).to_string(),
                ),
                "variable_declaration" => {
                    Self::function_valued_locals(&node, source, &mut symbols);
                    continue;
                }
                _ => continue,
            };
            if name.is_empty() {
                continue;
            }
            if public {
                exports.push(Export {
                    name: name.clone(),
                    kind: kind.clone(),
                });
            }
            symbols.push(Symbol {
                name,
                kind,
                visibility: if public {
                    Visibility::Public
                } else {
                    Visibility::Private
                },
                signature: Self::first_line(&node, source),
                body,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
            });
        }

        // A require anywhere is a dependency, including one deferred into a function body.
        let mut imports = Vec::new();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if node.kind() == "function_call" {
                if let Some(import) = Self::require_import(&node, source) {
                    imports.push(import);
                }
            }
            let mut cursor = node.walk();
            let children: Vec<Node> = node.named_children(&mut cursor).collect();
            stack.extend(children.into_iter().rev());
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

    fn extract(source: &str) -> ParseResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_luau::LANGUAGE.into())
            .expect("failed to set language");
        let tree = parser.parse(source, None).expect("parse failed");
        assert!(
            !tree.root_node().has_error(),
            "the Luau grammar must accept the fixture"
        );
        LuauLanguage.extract(source, &tree)
    }

    fn symbol<'a>(result: &'a ParseResult, name: &str) -> &'a Symbol {
        result
            .symbols
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no symbol {name} in {:?}", result.symbols))
    }

    fn exported(result: &ParseResult, name: &str) -> Option<SymbolKind> {
        result
            .exports
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.kind.clone())
    }

    #[test]
    fn test_global_function_is_public_and_exported() {
        let result = extract("function greet(name: string): string\n    return name\nend\n");
        let greet = symbol(&result, "greet");
        assert_eq!(greet.kind, SymbolKind::Function);
        assert_eq!(greet.visibility, Visibility::Public);
        assert_eq!(greet.signature, "function greet(name: string): string");
        assert!(greet.body.contains("return name"));
        assert_eq!((greet.start_line, greet.end_line), (1, 3));
        assert_eq!(exported(&result, "greet"), Some(SymbolKind::Function));
    }

    #[test]
    fn test_local_generic_function_is_private() {
        let result = extract("local function identity<T>(x: T): T\n    return x\nend\n");
        let identity = symbol(&result, "identity");
        assert_eq!(identity.kind, SymbolKind::Function);
        assert_eq!(identity.visibility, Visibility::Private);
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_module_and_method_functions_keep_their_qualified_names() {
        let result = extract(
            "local M = {}\nfunction M.add(a: number, b: number): number\n    return a + b\nend\nfunction M:reset()\nend\nreturn M\n",
        );
        assert_eq!(symbol(&result, "M.add").visibility, Visibility::Public);
        assert_eq!(symbol(&result, "M:reset").visibility, Visibility::Public);
        assert_eq!(exported(&result, "M.add"), Some(SymbolKind::Function));
    }

    #[test]
    fn test_export_type_is_an_exported_alias_and_bare_type_is_private() {
        let result = extract(
            "export type Entry = {\n    id: number,\n}\ntype Hidden = string\nexport type Box<T> = { value: T }\n",
        );
        let entry = symbol(&result, "Entry");
        assert_eq!(entry.kind, SymbolKind::TypeAlias);
        assert_eq!(entry.visibility, Visibility::Public);
        assert_eq!((entry.start_line, entry.end_line), (1, 3));
        assert_eq!(exported(&result, "Entry"), Some(SymbolKind::TypeAlias));

        let hidden = symbol(&result, "Hidden");
        assert_eq!(hidden.visibility, Visibility::Private);
        assert_eq!(exported(&result, "Hidden"), None);

        assert_eq!(symbol(&result, "Box").kind, SymbolKind::TypeAlias);
        assert_eq!(exported(&result, "Box"), Some(SymbolKind::TypeAlias));
    }

    #[test]
    fn test_requires_by_string_and_instance_path_are_imports() {
        let result = extract(
            r#"local fs = require("@lune/fs")
local Config = require("../src/shared/Config")
local Registry = require(script.Parent.Parent.ServiceRegistry)
local Logger = require((script :: any).Parent.Logger)
local Typed = require(Shared.Typed) :: any
local A, B = require(Shared.A), require(script.Parent:WaitForChild("B"))
"#,
        );
        let got: Vec<(&str, &str)> = result
            .imports
            .iter()
            .map(|i| (i.source.as_str(), i.names[0].as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("@lune/fs", "fs"),
                ("../src/shared/Config", "Config"),
                ("script.Parent.Parent.ServiceRegistry", "ServiceRegistry"),
                ("(script :: any).Parent.Logger", "Logger"),
                ("Shared.Typed", "Typed"),
                ("Shared.A", "A"),
                ("script.Parent:WaitForChild(\"B\")", "B"),
            ]
        );
    }

    #[test]
    fn test_a_require_inside_a_function_is_still_a_dependency() {
        let result = extract(
            "local function load()\n    local locale = require(\"../content/locale/en\")\n    return locale\nend\n",
        );
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].source, "../content/locale/en");
    }

    #[test]
    fn test_dynamic_and_lookalike_requires_are_not_imports() {
        let result = extract(
            "local requirements = {}\nlocal x = required(\"a\")\nlocal raw = require(\"../content/events/\" .. name)\nlocal y = obj.require(\"b\")\n",
        );
        assert!(
            result.imports.is_empty(),
            "no static require here, got {:?}",
            result.imports
        );
    }

    #[test]
    fn test_function_valued_local_is_a_private_variable() {
        let result = extract("local count, handler = 0, function(x: number)\n    return x\nend\n");
        let handler = symbol(&result, "handler");
        assert_eq!(handler.kind, SymbolKind::Variable);
        assert_eq!(handler.visibility, Visibility::Private);
        assert!(result.symbols.iter().all(|s| s.name != "count"));
    }

    #[test]
    fn test_empty_source() {
        let result = extract("");
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }
}
