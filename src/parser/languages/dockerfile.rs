use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct DockerfileLanguage;

impl DockerfileLanguage {
    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        text.lines().next().unwrap_or("").trim().to_string()
    }

    fn full_text(node: &tree_sitter::Node, source: &[u8]) -> String {
        Self::node_text(node, source).to_string()
    }

    /// Extract the image name from a FROM instruction.
    fn extract_from_image(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            if kind == "image_spec" {
                // image_spec may contain image_name, image_tag, image_digest
                let mut spec_cursor = child.walk();
                for spec_child in child.children(&mut spec_cursor) {
                    if spec_child.kind() == "image_name" {
                        return Self::node_text(&spec_child, source).trim().to_string();
                    }
                }
                // Fallback: use the full image_spec text
                return Self::node_text(&child, source).trim().to_string();
            }
            // Some grammars directly put the image name
            if kind == "image_name" {
                return Self::node_text(&child, source).trim().to_string();
            }
        }
        // Fallback: parse from text
        let text = Self::node_text(node, source);
        let after_from = text.split_whitespace().nth(1).unwrap_or("").to_string();
        // Strip the alias (AS name)
        after_from
            .split_whitespace()
            .next()
            .unwrap_or(&after_from)
            .to_string()
    }

    /// Extract the alias from a FROM instruction (e.g., `FROM node:18 AS builder` -> `builder`).
    fn extract_from_alias(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        let mut cursor = node.walk();
        let mut found_as = false;
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            if kind == "as_instruction" || kind == "image_alias" {
                return Some(Self::node_text(&child, source).trim().to_string());
            }
            let text = Self::node_text(&child, source);
            if text.eq_ignore_ascii_case("as") {
                found_as = true;
                continue;
            }
            if found_as {
                return Some(text.trim().to_string());
            }
        }
        None
    }

    /// Extract a descriptive name for an instruction (e.g., the command or key).
    fn extract_instruction_summary(node: &tree_sitter::Node, source: &[u8]) -> String {
        Self::first_line(node, source)
    }
}

impl LanguageSupport for DockerfileLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_dockerfile_updated::language()
    }

    fn name(&self) -> &str {
        "dockerfile"
    }

    fn extract(&self, source: &str, tree: &tree_sitter::Tree) -> ParseResult {
        let source_bytes = source.as_bytes();
        let root = tree.root_node();

        let mut symbols: Vec<Symbol> = Vec::new();
        let imports: Vec<Import> = Vec::new();
        let exports: Vec<Export> = Vec::new();

        let mut cursor = root.walk();

        for node in root.children(&mut cursor) {
            match node.kind() {
                "from_instruction" => {
                    let image = Self::extract_from_image(&node, source_bytes);
                    let alias = Self::extract_from_alias(&node, source_bytes);
                    let name = if let Some(ref a) = alias {
                        format!("{} (as {})", image, a)
                    } else {
                        image
                    };
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    symbols.push(Symbol {
                        name,
                        kind: SymbolKind::Section,
                        visibility: Visibility::Public,
                        signature: Self::first_line(&node, source_bytes),
                        body: Self::full_text(&node, source_bytes),
                        start_line,
                        end_line,
                    });
                }

                "run_instruction"
                | "copy_instruction"
                | "env_instruction"
                | "expose_instruction"
                | "cmd_instruction"
                | "entrypoint_instruction"
                | "workdir_instruction"
                | "arg_instruction"
                | "label_instruction"
                | "add_instruction"
                | "volume_instruction"
                | "user_instruction"
                | "healthcheck_instruction"
                | "shell_instruction"
                | "stopsignal_instruction"
                | "onbuild_instruction"
                | "maintainer_instruction" => {
                    let summary = Self::extract_instruction_summary(&node, source_bytes);
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    symbols.push(Symbol {
                        name: summary.clone(),
                        kind: SymbolKind::Instruction,
                        visibility: Visibility::Public,
                        signature: summary,
                        body: Self::full_text(&node, source_bytes),
                        start_line,
                        end_line,
                    });
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
            .set_language(&tree_sitter_dockerfile_updated::language())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_from_instruction() {
        let source = r#"FROM node:18-alpine
RUN npm install
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = DockerfileLanguage;
        let result = lang.extract(source, &tree);

        let sections: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Section)
            .collect();
        assert!(
            !sections.is_empty(),
            "expected FROM as Section, got: {:?}",
            result
                .symbols
                .iter()
                .map(|s| (&s.name, &s.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_instructions() {
        let source = r#"FROM ubuntu:22.04
RUN apt-get update
COPY . /app
WORKDIR /app
ENV NODE_ENV=production
EXPOSE 8080
CMD ["node", "server.js"]
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = DockerfileLanguage;
        let result = lang.extract(source, &tree);

        let instructions: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Instruction)
            .collect();
        assert!(
            instructions.len() >= 5,
            "expected at least 5 instructions, got: {:?}",
            instructions.iter().map(|i| &i.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = DockerfileLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.exports.is_empty());
    }

    #[test]
    fn test_complex_dockerfile() {
        let source = r#"FROM node:18 AS builder
WORKDIR /app
COPY package*.json ./
RUN npm ci
COPY . .
RUN npm run build

FROM node:18-alpine
WORKDIR /app
COPY --from=builder /app/dist ./dist
COPY --from=builder /app/node_modules ./node_modules
EXPOSE 3000
ENV NODE_ENV=production
CMD ["node", "dist/server.js"]
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = DockerfileLanguage;
        let result = lang.extract(source, &tree);

        let sections: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Section)
            .collect();
        assert!(
            sections.len() >= 2,
            "expected at least 2 FROM sections, got: {:?}",
            sections.iter().map(|s| &s.name).collect::<Vec<_>>()
        );

        let instructions: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Instruction)
            .collect();
        assert!(
            instructions.len() >= 8,
            "expected at least 8 instructions, got: {:?}",
            instructions.iter().map(|i| &i.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_symbol_kinds() {
        let source = "FROM alpine:latest\nRUN echo hello\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = DockerfileLanguage;
        let result = lang.extract(source, &tree);

        let has_section = result.symbols.iter().any(|s| s.kind == SymbolKind::Section);
        let has_instruction = result
            .symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Instruction);
        assert!(has_section, "expected Section symbol kind for FROM");
        assert!(has_instruction, "expected Instruction symbol kind");

        assert!(
            result
                .symbols
                .iter()
                .all(|s| s.visibility == Visibility::Public),
            "all Dockerfile symbols should be public"
        );
    }

    #[test]
    fn test_no_imports_exports() {
        let source = "FROM alpine\nRUN echo test\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = DockerfileLanguage;
        let result = lang.extract(source, &tree);
        assert!(
            result.imports.is_empty(),
            "dockerfile should have no imports"
        );
        assert!(
            result.exports.is_empty(),
            "dockerfile should have no exports"
        );
    }

    #[test]
    fn test_arg_instruction() {
        let source = r#"ARG BASE_IMAGE=node:18
FROM ${BASE_IMAGE}
ARG APP_VERSION=1.0
LABEL version=${APP_VERSION}
"#;
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = DockerfileLanguage;
        let result = lang.extract(source, &tree);

        assert!(
            !result.symbols.is_empty(),
            "expected symbols from ARG Dockerfile"
        );
    }
}
