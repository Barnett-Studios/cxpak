use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct XmlLanguage;

impl XmlLanguage {
    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        text.lines().next().unwrap_or("").trim().to_string()
    }

    /// Extract the tag name from an element node.
    fn extract_tag_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "STag" | "EmptyElemTag" | "start_tag" | "self_closing_tag" => {
                    let mut inner_cursor = child.walk();
                    for inner in child.children(&mut inner_cursor) {
                        if inner.kind() == "Name" || inner.kind() == "tag_name" {
                            return Self::node_text(&inner, source).to_string();
                        }
                    }
                }
                "Name" | "tag_name" => {
                    return Self::node_text(&child, source).to_string();
                }
                _ => {}
            }
        }
        String::new()
    }

    /// Recursively extract elements from the tree.
    /// `depth` controls how deep we go (0 = top-level, 1 = children of top-level, etc.)
    fn extract_elements(
        node: &tree_sitter::Node,
        source: &[u8],
        symbols: &mut Vec<Symbol>,
        max_depth: usize,
        current_depth: usize,
    ) {
        if current_depth > max_depth {
            return;
        }

        match node.kind() {
            "element" => {
                let tag_name = Self::extract_tag_name(node, source);
                let signature = Self::first_line(node, source);
                let body = Self::node_text(node, source).to_string();
                let start_line = node.start_position().row + 1;
                let end_line = node.end_position().row + 1;

                if !tag_name.is_empty() {
                    symbols.push(Symbol {
                        name: tag_name,
                        kind: SymbolKind::Element,
                        visibility: Visibility::Public,
                        signature,
                        body,
                        start_line,
                        end_line,
                    });
                }

                // Recurse into child elements (content children)
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "content" {
                        let mut content_cursor = child.walk();
                        for content_child in child.children(&mut content_cursor) {
                            Self::extract_elements(
                                &content_child,
                                source,
                                symbols,
                                max_depth,
                                current_depth + 1,
                            );
                        }
                    } else {
                        Self::extract_elements(
                            &child,
                            source,
                            symbols,
                            max_depth,
                            current_depth + 1,
                        );
                    }
                }
            }

            _ => {
                // For non-element nodes, recurse into children
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    Self::extract_elements(&child, source, symbols, max_depth, current_depth);
                }
            }
        }
    }
}

impl LanguageSupport for XmlLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_xml::LANGUAGE_XML.into()
    }

    fn name(&self) -> &str {
        "xml"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let imports: Vec<Import> = Vec::new();
        let exports: Vec<Export> = Vec::new();

        // Extract elements up to 2 levels deep (top-level + one level of nesting)
        Self::extract_elements(&root, source_bytes, &mut symbols, 2, 0);

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
            .set_language(&tree_sitter_xml::LANGUAGE_XML.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_top_level_elements() {
        let source = r#"<?xml version="1.0"?>
<root>
  <item>Hello</item>
  <item>World</item>
</root>
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = XmlLanguage;
        let result = lang.extract(source, &tree);

        let elements: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Element)
            .collect();
        assert!(
            !elements.is_empty(),
            "expected at least root element, got symbols: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
        assert_eq!(elements[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_nested_elements() {
        let source = r#"<?xml version="1.0"?>
<project>
  <dependencies>
    <dependency>junit</dependency>
  </dependencies>
  <build>
    <plugins>
      <plugin>maven-compiler</plugin>
    </plugins>
  </build>
</project>
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = XmlLanguage;
        let result = lang.extract(source, &tree);

        let elements: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Element)
            .collect();
        assert!(
            elements.len() >= 3,
            "expected at least project, dependencies, build, got: {:?}",
            elements.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = XmlLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_xml() {
        let source = r#"<?xml version="1.0" encoding="UTF-8"?>
<configuration>
  <appSettings>
    <add key="DatabaseHost" value="localhost"/>
    <add key="DatabasePort" value="5432"/>
  </appSettings>
  <connectionStrings>
    <add name="Default" connectionString="Server=localhost;Database=mydb"/>
  </connectionStrings>
  <system.web>
    <compilation debug="true"/>
  </system.web>
</configuration>
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = XmlLanguage;
        let result = lang.extract(source, &tree);

        let elements: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Element)
            .collect();
        assert!(
            elements.len() >= 3,
            "expected multiple elements, got: {:?}",
            elements.iter().map(|e| &e.name).collect::<Vec<_>>()
        );

        // The root element should be "configuration"
        assert!(
            elements.iter().any(|e| e.name == "configuration"),
            "expected 'configuration' element, got: {:?}",
            elements.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
    }
}
