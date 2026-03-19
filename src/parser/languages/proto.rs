use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct ProtoLanguage;

impl ProtoLanguage {
    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        text.lines().next().unwrap_or("").trim().to_string()
    }

    /// Extract the name from a node by looking for specific child kinds.
    fn extract_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        // Try named children first: message_name, service_name, enum_name, rpc_name
        for field in &[
            "name",
            "message_name",
            "service_name",
            "enum_name",
            "rpc_name",
        ] {
            if let Some(child) = node.child_by_field_name(field) {
                return Self::node_text(&child, source).to_string();
            }
        }
        // Fallback: look for identifier children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "message_name" | "service_name" | "enum_name" | "rpc_name" | "identifier" => {
                    return Self::node_text(&child, source).to_string();
                }
                _ => {}
            }
        }
        String::new()
    }

    /// Extract the import path from an import statement.
    fn extract_import_path(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let text = Self::node_text(&child, source);
            if child.kind() == "string" || child.kind() == "string_literal" {
                let path = text.trim_matches('"').trim_matches('\'').to_string();
                if !path.is_empty() {
                    return Some(path);
                }
            }
        }
        // Fallback: parse from text
        let text = Self::node_text(node, source);
        if let Some(start) = text.find('"') {
            if let Some(end) = text[start + 1..].find('"') {
                let path = &text[start + 1..start + 1 + end];
                if !path.is_empty() {
                    return Some(path.to_string());
                }
            }
        }
        None
    }
}

impl LanguageSupport for ProtoLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_proto::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "proto"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let mut imports: Vec<Import> = Vec::new();
        let exports: Vec<Export> = Vec::new();

        let mut cursor = root.walk();

        for node in root.children(&mut cursor) {
            match node.kind() {
                "message" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if !name.is_empty() {
                        symbols.push(Symbol {
                            name,
                            kind: SymbolKind::Message,
                            visibility: Visibility::Public,
                            signature,
                            body,
                            start_line,
                            end_line,
                        });
                    }
                }

                "service" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if !name.is_empty() {
                        // Extract RPC methods from service body
                        let mut inner_cursor = node.walk();
                        for child in node.children(&mut inner_cursor) {
                            if child.kind() == "rpc" {
                                let rpc_name = Self::extract_name(&child, source_bytes);
                                let rpc_sig = Self::first_line(&child, source_bytes);
                                let rpc_body = Self::node_text(&child, source_bytes).to_string();
                                let rpc_start = child.start_position().row + 1;
                                let rpc_end = child.end_position().row + 1;

                                if !rpc_name.is_empty() {
                                    symbols.push(Symbol {
                                        name: rpc_name,
                                        kind: SymbolKind::Method,
                                        visibility: Visibility::Public,
                                        signature: rpc_sig,
                                        body: rpc_body,
                                        start_line: rpc_start,
                                        end_line: rpc_end,
                                    });
                                }
                            }
                        }

                        symbols.push(Symbol {
                            name,
                            kind: SymbolKind::Service,
                            visibility: Visibility::Public,
                            signature,
                            body,
                            start_line,
                            end_line,
                        });
                    }
                }

                "enum" => {
                    let name = Self::extract_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if !name.is_empty() {
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
                }

                "import" => {
                    if let Some(path) = Self::extract_import_path(&node, source_bytes) {
                        let short_name = path
                            .rsplit('/')
                            .next()
                            .unwrap_or(&path)
                            .trim_end_matches(".proto")
                            .to_string();
                        imports.push(Import {
                            source: path,
                            names: vec![short_name],
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::language::{SymbolKind, Visibility};

    fn make_parser() -> tree_sitter::Parser {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_proto::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_messages_and_enums() {
        let source = r#"syntax = "proto3";

message SearchRequest {
  string query = 1;
  int32 page_number = 2;
  int32 result_per_page = 3;
}

enum Status {
  UNKNOWN = 0;
  ACTIVE = 1;
  INACTIVE = 2;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ProtoLanguage;
        let result = lang.extract(source, &tree);

        let messages: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Message)
            .collect();
        assert!(
            !messages.is_empty(),
            "expected message, got symbols: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
        assert_eq!(messages[0].name, "SearchRequest");
        assert_eq!(messages[0].visibility, Visibility::Public);

        let enums: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Enum)
            .collect();
        assert!(!enums.is_empty(), "expected enum");
        assert_eq!(enums[0].name, "Status");
    }

    #[test]
    fn test_extract_service_with_rpcs() {
        let source = r#"syntax = "proto3";

service Greeter {
  rpc SayHello (HelloRequest) returns (HelloReply);
  rpc SayGoodbye (GoodbyeRequest) returns (GoodbyeReply);
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ProtoLanguage;
        let result = lang.extract(source, &tree);

        let services: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Service)
            .collect();
        assert!(!services.is_empty(), "expected service");
        assert_eq!(services[0].name, "Greeter");

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(
            methods.len() >= 2,
            "expected at least 2 RPC methods, got: {:?}",
            methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = ProtoLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"syntax = "proto3";

import "google/protobuf/timestamp.proto";
import "other/messages.proto";

message Event {
  string name = 1;
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ProtoLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            result.imports.len() >= 2,
            "expected at least 2 imports, got: {:?}",
            result.imports
        );
    }

    #[test]
    fn test_complex_proto() {
        let source = r#"syntax = "proto3";

package example.api;

import "google/protobuf/empty.proto";

message User {
  string id = 1;
  string name = 2;
  repeated string tags = 3;
}

message CreateUserRequest {
  User user = 1;
}

enum Role {
  ADMIN = 0;
  USER = 1;
}

service UserService {
  rpc CreateUser (CreateUserRequest) returns (User);
  rpc DeleteUser (User) returns (google.protobuf.Empty);
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = ProtoLanguage;
        let result = lang.extract(source, &tree);

        let messages: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Message)
            .collect();
        assert!(
            messages.len() >= 2,
            "expected at least 2 messages, got: {:?}",
            messages.iter().map(|m| &m.name).collect::<Vec<_>>()
        );

        let services: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Service)
            .collect();
        assert!(!services.is_empty(), "expected service");

        assert!(!result.imports.is_empty(), "expected imports");
    }
}
