use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct GraphqlLanguage;

impl GraphqlLanguage {
    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        text.lines().next().unwrap_or("").trim().to_string()
    }

    /// Extract the name from a node by looking for a `name` field or `name` child.
    fn extract_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        if let Some(name_node) = node.child_by_field_name("name") {
            return Self::node_text(&name_node, source).to_string();
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "name" || child.kind() == "identifier" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }

    /// Detect the operation type for an operation_definition node.
    fn detect_operation_type(node: &tree_sitter::Node, source: &[u8]) -> SymbolKind {
        let text = Self::node_text(node, source);
        let lower = text.to_lowercase();
        if lower.starts_with("mutation") {
            SymbolKind::Mutation
        } else if lower.starts_with("subscription") {
            SymbolKind::Query // Subscriptions map to Query
        } else {
            // "query" or anonymous (default to Query)
            SymbolKind::Query
        }
    }
}

impl LanguageSupport for GraphqlLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_graphql::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "graphql"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let imports: Vec<Import> = Vec::new();
        let exports: Vec<Export> = Vec::new();

        // tree-sitter-graphql wraps everything:
        // document -> definition -> type_system_definition/executable_definition -> actual node.
        // We use a stack-based walk to drill through wrapper nodes.
        let mut stack: Vec<tree_sitter::Node> = Vec::new();
        {
            let mut cursor = root.walk();
            for child in root.children(&mut cursor) {
                stack.push(child);
            }
        }

        while let Some(node) = stack.pop() {
            let kind_str = node.kind();
            match kind_str {
                // Wrapper nodes — drill into children
                "document"
                | "definition"
                | "type_system_definition"
                | "type_system_extension"
                | "executable_definition"
                | "type_extension"
                | "type_definition" => {
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        stack.push(child);
                    }
                }

                "object_type_definition" => {
                    // Direct type definition
                    Self::extract_type_definitions(&node, source_bytes, &mut symbols);
                }

                "query_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    symbols.push(Symbol {
                        name: if name.is_empty() {
                            "anonymous_query".to_string()
                        } else {
                            name
                        },
                        kind: SymbolKind::Query,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "mutation_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    symbols.push(Symbol {
                        name: if name.is_empty() {
                            "anonymous_mutation".to_string()
                        } else {
                            name
                        },
                        kind: SymbolKind::Mutation,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "subscription_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    symbols.push(Symbol {
                        name: if name.is_empty() {
                            "anonymous_subscription".to_string()
                        } else {
                            name
                        },
                        kind: SymbolKind::Query,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "operation_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let kind = Self::detect_operation_type(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    symbols.push(Symbol {
                        name: if name.is_empty() {
                            "anonymous_operation".to_string()
                        } else {
                            name
                        },
                        kind,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "enum_type_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    symbols.push(Symbol {
                        name: if name.is_empty() {
                            "anonymous_enum".to_string()
                        } else {
                            name
                        },
                        kind: SymbolKind::Enum,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "interface_type_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    symbols.push(Symbol {
                        name: if name.is_empty() {
                            "anonymous_interface".to_string()
                        } else {
                            name
                        },
                        kind: SymbolKind::Interface,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                "scalar_type_definition"
                | "union_type_definition"
                | "input_object_type_definition" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if !name.is_empty() {
                        symbols.push(Symbol {
                            name,
                            kind: SymbolKind::Type,
                            visibility: Visibility::Public,
                            signature,
                            body,
                            start_line,
                            end_line,
                        });
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

impl GraphqlLanguage {
    /// Extract type definitions that may be nested inside type_system_definition or similar wrappers.
    fn extract_type_definitions(
        node: &tree_sitter::Node,
        source: &[u8],
        symbols: &mut Vec<Symbol>,
    ) {
        let kind_str = node.kind();
        match kind_str {
            "object_type_definition" => {
                let name = Self::extract_name(node, source);
                let signature = Self::first_line(node, source);
                let body = Self::node_text(node, source).to_string();
                let start_line = node.start_position().row + 1;
                let end_line = node.end_position().row + 1;

                if !name.is_empty() {
                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Type,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }
            }

            "type_definition" | "type_system_definition" => {
                // Check if this node itself has a name (some grammars put the name directly)
                let name = Self::extract_name(node, source);
                if !name.is_empty() {
                    let signature = Self::first_line(node, source);
                    let body = Self::node_text(node, source).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Type,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                // Also recurse into children for nested definitions
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    Self::extract_type_definitions(&child, source, symbols);
                }
            }

            _ => {
                // Recurse into children
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    Self::extract_type_definitions(&child, source, symbols);
                }
            }
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
            .set_language(&tree_sitter_graphql::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_type_definitions() {
        let source = r#"type User {
  id: ID!
  name: String!
  email: String
}

type Post {
  id: ID!
  title: String!
  author: User!
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GraphqlLanguage;
        let result = lang.extract(source, &tree);

        let types: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Type)
            .collect();
        assert!(
            types.len() >= 2,
            "expected at least 2 types (User, Post), got: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
        assert_eq!(types[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_operations() {
        let source = r#"query GetUser($id: ID!) {
  user(id: $id) {
    name
    email
  }
}

mutation CreateUser($input: CreateUserInput!) {
  createUser(input: $input) {
    id
    name
  }
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GraphqlLanguage;
        let result = lang.extract(source, &tree);

        let queries: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Query)
            .collect();
        assert!(
            !queries.is_empty(),
            "expected query operation, got symbols: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );

        let mutations: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Mutation)
            .collect();
        assert!(
            !mutations.is_empty(),
            "expected mutation operation, got symbols: {:?}",
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
        let lang = GraphqlLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_schema() {
        let source = r#"type Query {
  users: [User!]!
  user(id: ID!): User
}

type Mutation {
  createUser(input: CreateUserInput!): User!
  deleteUser(id: ID!): Boolean!
}

enum Role {
  ADMIN
  USER
  GUEST
}

interface Node {
  id: ID!
}

type User implements Node {
  id: ID!
  name: String!
  role: Role!
}

input CreateUserInput {
  name: String!
  email: String!
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = GraphqlLanguage;
        let result = lang.extract(source, &tree);

        // Should have multiple types, enum, interface
        assert!(
            result.symbols.len() >= 4,
            "expected multiple symbols (types, enum, interface, input), got: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );

        let enums: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Enum)
            .collect();
        assert!(
            !enums.is_empty(),
            "expected enum type, got symbols: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
    }
}
