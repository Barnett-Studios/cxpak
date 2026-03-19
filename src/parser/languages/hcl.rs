use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct HclLanguage;

impl HclLanguage {
    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        text.lines().next().unwrap_or("").trim().to_string()
    }

    /// Build the block name from block type + labels.
    /// e.g., `resource "aws_instance" "web"` → "resource aws_instance web"
    fn extract_block_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    parts.push(Self::node_text(&child, source).to_string());
                }
                "string_lit" => {
                    let text = Self::node_text(&child, source);
                    // string_lit may use quoted_template_start/end with actual quotes
                    let unquoted = text.trim_matches('"');
                    parts.push(unquoted.to_string());
                }
                "body" | "block" | "block_start" | "block_end" => {
                    // Stop collecting name parts when we hit the body
                    break;
                }
                _ => {}
            }
        }
        parts.join(" ")
    }

    /// Extract the attribute name from an `attribute` node.
    fn extract_attribute_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                return Self::node_text(&child, source).to_string();
            }
        }
        String::new()
    }
}

impl LanguageSupport for HclLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_hcl::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "hcl"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let imports: Vec<Import> = Vec::new();
        let exports: Vec<Export> = Vec::new();

        // tree-sitter-hcl: root is `config_file` whose only child is `body`.
        // We need to find the `body` node and iterate its children.
        let body_node = {
            let mut found = root;
            let mut cursor = root.walk();
            for child in root.children(&mut cursor) {
                if child.kind() == "body" {
                    found = child;
                    break;
                }
            }
            found
        };

        let mut cursor = body_node.walk();
        for node in body_node.children(&mut cursor) {
            match node.kind() {
                "block" => {
                    let name = Self::extract_block_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if !name.is_empty() {
                        symbols.push(Symbol {
                            name,
                            kind: SymbolKind::Block,
                            visibility: Visibility::Public,
                            signature,
                            body,
                            start_line,
                            end_line,
                        });
                    }
                }

                "attribute" => {
                    let name = Self::extract_attribute_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if !name.is_empty() {
                        symbols.push(Symbol {
                            name,
                            kind: SymbolKind::Variable,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::language::{SymbolKind, Visibility};

    fn make_parser() -> tree_sitter::Parser {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_hcl::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_blocks() {
        let source = r#"resource "aws_instance" "web" {
  ami           = "ami-12345"
  instance_type = "t2.micro"
}

variable "region" {
  default = "us-east-1"
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = HclLanguage;
        let result = lang.extract(source, &tree);

        let blocks: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Block)
            .collect();
        assert!(
            blocks.len() >= 2,
            "expected at least 2 blocks, got: {:?}",
            blocks.iter().map(|b| &b.name).collect::<Vec<_>>()
        );
        assert_eq!(blocks[0].visibility, Visibility::Public);
        assert!(blocks[0].name.contains("resource"));
        assert!(blocks[0].name.contains("aws_instance"));
        assert!(blocks[0].name.contains("web"));
    }

    #[test]
    fn test_extract_top_level_attributes() {
        let source = r#"region = "us-east-1"
project = "my-app"
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = HclLanguage;
        let result = lang.extract(source, &tree);

        let vars: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .collect();
        assert!(
            vars.len() >= 2,
            "expected at least 2 variables, got: {:?}",
            vars.iter().map(|v| &v.name).collect::<Vec<_>>()
        );
        assert_eq!(vars[0].name, "region");
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = HclLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_terraform() {
        let source = r#"terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 4.0"
    }
  }
}

provider "aws" {
  region = "us-east-1"
}

resource "aws_s3_bucket" "logs" {
  bucket = "my-logs-bucket"

  tags = {
    Environment = "production"
  }
}

output "bucket_arn" {
  value = aws_s3_bucket.logs.arn
}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = HclLanguage;
        let result = lang.extract(source, &tree);

        let blocks: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Block)
            .collect();
        assert!(
            blocks.len() >= 4,
            "expected at least 4 blocks (terraform, provider, resource, output), got: {:?}",
            blocks.iter().map(|b| &b.name).collect::<Vec<_>>()
        );
    }
}
