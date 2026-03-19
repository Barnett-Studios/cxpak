use crate::parser::language::{
    Export, Import, LanguageSupport, ParseResult, Symbol, SymbolKind, Visibility,
};
use tree_sitter::Language as TsLanguage;

pub struct MakefileLanguage;

impl MakefileLanguage {
    fn node_text<'a>(node: &tree_sitter::Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    fn first_line(node: &tree_sitter::Node, source: &[u8]) -> String {
        let text = Self::node_text(node, source);
        text.lines().next().unwrap_or("").trim().to_string()
    }

    /// Extract the target name(s) from a rule node.
    /// The target is the part before the colon.
    fn extract_target_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        // Try to find target children directly
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "targets" || child.kind() == "target" {
                return Self::node_text(&child, source).trim().to_string();
            }
        }
        // Fallback: parse from the first line (text before colon)
        let first = Self::first_line(node, source);
        if let Some(colon_idx) = first.find(':') {
            let target = first[..colon_idx].trim();
            if !target.is_empty() {
                return target.to_string();
            }
        }
        // Last fallback: use first word
        let text = Self::node_text(node, source);
        text.split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches(':')
            .to_string()
    }

    /// Extract the variable name from a variable_assignment node.
    fn extract_variable_name(node: &tree_sitter::Node, source: &[u8]) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "word" || child.kind() == "variable_name" || child.kind() == "NAME" {
                return Self::node_text(&child, source).to_string();
            }
        }
        // Fallback: parse from text (e.g., "CC = gcc" or "CC := gcc")
        let text = Self::node_text(node, source);
        for delim in &[":=", "?=", "+=", "="] {
            if let Some(idx) = text.find(delim) {
                let name = text[..idx].trim();
                if !name.is_empty() {
                    return name.to_string();
                }
            }
        }
        String::new()
    }

    /// Extract include path from an include directive.
    fn extract_include_path(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        let text = Self::node_text(node, source).trim().to_string();
        // Strip the "include" or "-include" prefix
        let after = text
            .strip_prefix("-include")
            .or_else(|| text.strip_prefix("include"))
            .unwrap_or("")
            .trim();
        if after.is_empty() {
            // Try child nodes
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "word" || child.kind() == "list" {
                    let path = Self::node_text(&child, source).trim().to_string();
                    if !path.is_empty() && path != "include" && path != "-include" {
                        return Some(path);
                    }
                }
            }
            None
        } else {
            Some(after.to_string())
        }
    }
}

impl LanguageSupport for MakefileLanguage {
    fn ts_language(&self) -> TsLanguage {
        tree_sitter_make::LANGUAGE.into()
    }

    fn name(&self) -> &str {
        "makefile"
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
                "rule" => {
                    let name = Self::extract_target_name(&node, source_bytes);
                    let signature = Self::first_line(&node, source_bytes);
                    let body = Self::node_text(&node, source_bytes).to_string();
                    let start_line = node.start_position().row + 1;
                    let end_line = node.end_position().row + 1;

                    if !name.is_empty() {
                        symbols.push(Symbol {
                            name,
                            kind: SymbolKind::Target,
                            visibility: Visibility::Public,
                            signature,
                            body,
                            start_line,
                            end_line,
                        });
                    }
                }

                "variable_assignment" => {
                    let name = Self::extract_variable_name(&node, source_bytes);
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

                "include_directive" => {
                    if let Some(path) = Self::extract_include_path(&node, source_bytes) {
                        let short_name = path.rsplit('/').next().unwrap_or(&path).to_string();
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
            .set_language(&tree_sitter_make::LANGUAGE.into())
            .expect("failed to set language");
        parser
    }

    #[test]
    fn test_extract_rules() {
        let source = "build:\n\tcargo build\n\ntest:\n\tcargo test\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = MakefileLanguage;
        let result = lang.extract(source, &tree);

        let targets: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Target)
            .collect();
        assert!(
            targets.len() >= 2,
            "expected at least 2 targets, got: {:?}",
            targets.iter().map(|t| &t.name).collect::<Vec<_>>()
        );
        assert_eq!(targets[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_variables() {
        let source = "CC = gcc\nCFLAGS = -Wall -O2\n\nall:\n\t$(CC) $(CFLAGS) main.c\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = MakefileLanguage;
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
    }

    #[test]
    fn test_empty_source() {
        let source = "";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).unwrap();
        let lang = MakefileLanguage;
        let result = lang.extract(source, &tree);
        assert!(result.symbols.is_empty());
        assert!(result.imports.is_empty());
    }

    #[test]
    fn test_complex_makefile() {
        let source = "CC = gcc\nCFLAGS = -Wall\nSRC = main.c utils.c\n\n.PHONY: all clean\n\nall: $(SRC)\n\t$(CC) $(CFLAGS) -o app $(SRC)\n\nclean:\n\trm -f app\n\ninstall: all\n\tcp app /usr/local/bin/\n";
        let mut parser = make_parser();
        let tree = parser.parse(source, None).expect("parse failed");
        let lang = MakefileLanguage;
        let result = lang.extract(source, &tree);

        let targets: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Target)
            .collect();
        assert!(
            targets.len() >= 2,
            "expected multiple targets, got: {:?}",
            targets.iter().map(|t| &t.name).collect::<Vec<_>>()
        );

        let vars: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .collect();
        assert!(
            vars.len() >= 2,
            "expected multiple variables, got: {:?}",
            vars.iter().map(|v| &v.name).collect::<Vec<_>>()
        );
    }
}
