use crate::conventions::{FileContribution, PatternObservation};
use crate::index::{CodebaseIndex, IndexedFile};
use crate::parser::language::SymbolKind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamingStyle {
    SnakeCase,
    CamelCase,
    PascalCase,
    ScreamingSnake,
    KebabCase,
    Other,
}

impl std::fmt::Display for NamingStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NamingStyle::SnakeCase => write!(f, "snake_case"),
            NamingStyle::CamelCase => write!(f, "camelCase"),
            NamingStyle::PascalCase => write!(f, "PascalCase"),
            NamingStyle::ScreamingSnake => write!(f, "SCREAMING_SNAKE_CASE"),
            NamingStyle::KebabCase => write!(f, "kebab-case"),
            NamingStyle::Other => write!(f, "other"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NamingConventions {
    pub function_style: Option<PatternObservation>,
    pub type_style: Option<PatternObservation>,
    pub file_style: Option<PatternObservation>,
    pub constant_style: Option<PatternObservation>,
    pub additional: Vec<PatternObservation>,
    #[serde(skip)]
    pub file_contributions: HashMap<String, FileContribution>,
}

/// Classify a name into a naming style based on character-class analysis.
pub fn classify_name(name: &str) -> NamingStyle {
    if name.is_empty() {
        return NamingStyle::Other;
    }

    let has_underscore = name.contains('_');
    let has_hyphen = name.contains('-');
    let has_upper = name.chars().any(|c| c.is_uppercase());
    let has_lower = name.chars().any(|c| c.is_lowercase());
    let starts_upper = name.starts_with(|c: char| c.is_uppercase());

    if has_hyphen && !has_underscore {
        return NamingStyle::KebabCase;
    }

    if has_underscore {
        if has_upper && !has_lower {
            return NamingStyle::ScreamingSnake;
        }
        if !has_upper || !name.chars().any(|c| c.is_uppercase() && c != '_') {
            return NamingStyle::SnakeCase;
        }
        // mixed with underscores — treat as snake_case if mostly lowercase
        let upper_count = name.chars().filter(|c| c.is_uppercase()).count();
        let alpha_count = name.chars().filter(|c| c.is_alphabetic()).count();
        if alpha_count > 0 && (upper_count as f64 / alpha_count as f64) < 0.5 {
            return NamingStyle::SnakeCase;
        }
        return NamingStyle::ScreamingSnake;
    }

    // No underscore, no hyphen
    if starts_upper && has_lower {
        return NamingStyle::PascalCase;
    }
    if !starts_upper && has_upper && has_lower {
        return NamingStyle::CamelCase;
    }
    if has_upper && !has_lower {
        // All uppercase, no underscore.
        // Short names (≤6 chars) are likely acronyms (e.g. "API", "HTTP") — classify
        // as Other to avoid inflating the ScreamingSnake count with ambiguous names.
        if name.len() <= 6 {
            return NamingStyle::Other;
        }
        return NamingStyle::ScreamingSnake;
    }

    // All lowercase, no separators — single word, treat as snake_case
    NamingStyle::SnakeCase
}

/// Classify a filename (without extension) into a naming style.
pub fn classify_filename(filename: &str) -> NamingStyle {
    // Strip extension
    let name = filename.rsplit('/').next().unwrap_or(filename);
    let name = name.split('.').next().unwrap_or(name);
    if name.is_empty() {
        return NamingStyle::Other;
    }

    let has_hyphen = name.contains('-');
    let has_underscore = name.contains('_');
    let has_upper = name.chars().any(|c| c.is_uppercase());

    if has_hyphen {
        return NamingStyle::KebabCase;
    }
    if has_underscore {
        return NamingStyle::SnakeCase;
    }
    if has_upper {
        return NamingStyle::PascalCase;
    }
    // single word lowercase
    NamingStyle::SnakeCase
}

fn count_styles(names: &[NamingStyle]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for style in names {
        *counts.entry(style.to_string()).or_insert(0) += 1;
    }
    counts
}

fn dominant_observation(
    name: &str,
    counts: &HashMap<String, usize>,
    total: usize,
) -> Option<PatternObservation> {
    if total == 0 {
        return None;
    }
    // Stable tie detection: if two or more styles share the maximum count,
    // there is no dominant style — return None rather than picking one
    // non-deterministically.
    let max_count = counts.values().copied().max()?;
    let winners: Vec<_> = counts.iter().filter(|(_, &v)| v == max_count).collect();
    if winners.len() > 1 {
        return None;
    }
    let (dominant_style, &dominant_count) = winners[0];
    let mut obs = PatternObservation::new(name, dominant_style, dominant_count, total)?;

    // Collect exceptions (non-dominant names)
    let exceptions: Vec<String> = counts
        .iter()
        .filter(|(style, _)| *style != dominant_style)
        .map(|(style, &count)| format!("{style} ({count})"))
        .collect();
    if !exceptions.is_empty() {
        obs = obs.with_exceptions(exceptions);
    }
    Some(obs)
}

/// Extract naming conventions from the codebase index.
pub fn extract_naming(index: &CodebaseIndex) -> NamingConventions {
    let mut function_styles = Vec::new();
    let mut type_styles = Vec::new();
    let mut constant_styles = Vec::new();
    let mut file_contributions: HashMap<String, FileContribution> = HashMap::new();

    for file in &index.files {
        let mut contribution = FileContribution::default();

        if let Some(pr) = &file.parse_result {
            for symbol in &pr.symbols {
                let style = classify_name(&symbol.name);
                let style_str = style.to_string();

                match symbol.kind {
                    SymbolKind::Function | SymbolKind::Method => {
                        function_styles.push(style);
                        *contribution
                            .counts
                            .entry(format!("fn:{style_str}"))
                            .or_insert(0) += 1;
                    }
                    SymbolKind::Struct
                    | SymbolKind::Class
                    | SymbolKind::Enum
                    | SymbolKind::Type
                    | SymbolKind::Interface
                    | SymbolKind::Trait => {
                        type_styles.push(style);
                        *contribution
                            .counts
                            .entry(format!("type:{style_str}"))
                            .or_insert(0) += 1;
                    }
                    SymbolKind::Constant => {
                        constant_styles.push(style);
                        *contribution
                            .counts
                            .entry(format!("const:{style_str}"))
                            .or_insert(0) += 1;
                    }
                    _ => {}
                }
            }
        }

        file_contributions.insert(file.relative_path.clone(), contribution);
    }

    // File naming
    let file_styles: Vec<NamingStyle> = index
        .files
        .iter()
        .map(|f| classify_filename(&f.relative_path))
        .collect();

    let fn_counts = count_styles(&function_styles);
    let type_counts = count_styles(&type_styles);
    let const_counts = count_styles(&constant_styles);
    let file_counts = count_styles(&file_styles);

    NamingConventions {
        function_style: dominant_observation("function_naming", &fn_counts, function_styles.len()),
        type_style: dominant_observation("type_naming", &type_counts, type_styles.len()),
        file_style: dominant_observation("file_naming", &file_counts, file_styles.len()),
        constant_style: dominant_observation(
            "constant_naming",
            &const_counts,
            constant_styles.len(),
        ),
        additional: Vec::new(),
        file_contributions,
    }
}

/// Remove a file's contribution from naming conventions.
pub fn remove_file_contribution(conventions: &mut NamingConventions, path: &str) {
    conventions.file_contributions.remove(path);
}

/// Update a file's contribution to naming conventions.
pub fn update_file_contribution(_conventions: &mut NamingConventions, _file: &IndexedFile) {
    // Full recompute from file_contributions is deferred to the orchestrator
    // since we need to rebuild the aggregate counts.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_snake_case() {
        assert_eq!(classify_name("handle_request"), NamingStyle::SnakeCase);
        assert_eq!(classify_name("rate_limit"), NamingStyle::SnakeCase);
    }

    #[test]
    fn test_classify_camel_case() {
        assert_eq!(classify_name("handleRequest"), NamingStyle::CamelCase);
        assert_eq!(classify_name("getUser"), NamingStyle::CamelCase);
    }

    #[test]
    fn test_classify_pascal_case() {
        assert_eq!(classify_name("HandleRequest"), NamingStyle::PascalCase);
        assert_eq!(classify_name("UserService"), NamingStyle::PascalCase);
    }

    #[test]
    fn test_classify_screaming_snake() {
        assert_eq!(classify_name("MAX_RETRIES"), NamingStyle::ScreamingSnake);
        assert_eq!(classify_name("API_KEY"), NamingStyle::ScreamingSnake);
    }

    #[test]
    fn test_classify_acronym_short_all_caps() {
        // Short all-caps names without underscore are acronyms → Other.
        assert_eq!(classify_name("API"), NamingStyle::Other);
        assert_eq!(classify_name("HTTP"), NamingStyle::Other);
        assert_eq!(classify_name("ID"), NamingStyle::Other);
        // Long all-caps without underscore → ScreamingSnake (e.g. old code that
        // uses all-caps single words for constants).
        assert_eq!(
            classify_name("VERY_LONG_CONSTANT"),
            NamingStyle::ScreamingSnake
        );
    }

    #[test]
    fn test_classify_kebab_case() {
        assert_eq!(classify_name("my-component"), NamingStyle::KebabCase);
    }

    #[test]
    fn test_classify_single_word() {
        assert_eq!(classify_name("main"), NamingStyle::SnakeCase);
        assert_eq!(classify_name("Main"), NamingStyle::PascalCase);
    }

    #[test]
    fn test_classify_empty() {
        assert_eq!(classify_name(""), NamingStyle::Other);
    }

    #[test]
    fn test_classify_filename_snake() {
        assert_eq!(classify_filename("my_module.rs"), NamingStyle::SnakeCase);
    }

    #[test]
    fn test_classify_filename_kebab() {
        assert_eq!(
            classify_filename("my-component.tsx"),
            NamingStyle::KebabCase
        );
    }

    #[test]
    fn test_classify_filename_pascal() {
        assert_eq!(
            classify_filename("MyComponent.tsx"),
            NamingStyle::PascalCase
        );
    }

    #[test]
    fn test_classify_filename_single_word() {
        assert_eq!(classify_filename("main.rs"), NamingStyle::SnakeCase);
    }

    #[test]
    fn test_extract_naming_all_snake() {
        use crate::budget::counter::TokenCounter;
        use crate::conventions::PatternStrength;
        use crate::parser::language::{ParseResult, Symbol, Visibility};
        use crate::scanner::ScannedFile;

        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("test.rs");
        std::fs::write(&fp, "fn a() {} fn b() {} fn c() {}").unwrap();

        let files = vec![ScannedFile {
            relative_path: "test.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 28,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "test.rs".into(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "handle_request".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn handle_request()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "parse_input".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn parse_input()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "rate_limit".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn rate_limit()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let naming = extract_naming(&index);

        let fn_style = naming.function_style.unwrap();
        assert_eq!(fn_style.dominant, "snake_case");
        assert!(matches!(fn_style.strength, PatternStrength::Convention));
        assert_eq!(fn_style.count, 3);
        assert_eq!(fn_style.total, 3);
    }

    #[test]
    fn test_extract_naming_mixed() {
        use crate::budget::counter::TokenCounter;
        use crate::conventions::PatternStrength;
        use crate::parser::language::{ParseResult, Symbol, Visibility};
        use crate::scanner::ScannedFile;

        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("test.rs");
        std::fs::write(&fp, "x").unwrap();

        let files = vec![ScannedFile {
            relative_path: "test.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 1,
        }];

        let mut symbols = Vec::new();
        // 8 snake_case, 2 camelCase = 80% → Trend
        for i in 0..8 {
            symbols.push(Symbol {
                name: format!("func_{i}"),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: format!("fn func_{i}()"),
                body: "{}".into(),
                start_line: 1,
                end_line: 1,
            });
        }
        for i in 0..2 {
            symbols.push(Symbol {
                name: format!("funcCamel{i}"),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: format!("fn funcCamel{i}()"),
                body: "{}".into(),
                start_line: 1,
                end_line: 1,
            });
        }

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "test.rs".into(),
            ParseResult {
                symbols,
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let naming = extract_naming(&index);

        let fn_style = naming.function_style.unwrap();
        assert_eq!(fn_style.dominant, "snake_case");
        assert!(matches!(fn_style.strength, PatternStrength::Trend));
        assert_eq!(fn_style.count, 8);
        assert_eq!(fn_style.total, 10);
    }

    #[test]
    fn test_extract_naming_types_separate() {
        use crate::budget::counter::TokenCounter;
        use crate::parser::language::{ParseResult, Symbol, Visibility};
        use crate::scanner::ScannedFile;

        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("test.rs");
        std::fs::write(&fp, "x").unwrap();

        let files = vec![ScannedFile {
            relative_path: "test.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 1,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "test.rs".into(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "my_func".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn my_func()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "MyStruct".into(),
                        kind: SymbolKind::Struct,
                        visibility: Visibility::Public,
                        signature: "struct MyStruct".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let naming = extract_naming(&index);

        let fn_style = naming.function_style.unwrap();
        assert_eq!(fn_style.dominant, "snake_case");

        let type_style = naming.type_style.unwrap();
        assert_eq!(type_style.dominant, "PascalCase");
    }

    #[test]
    fn test_file_contributions_populated_after_extract() {
        use crate::budget::counter::TokenCounter;
        use crate::parser::language::{ParseResult, Symbol, Visibility};
        use crate::scanner::ScannedFile;

        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("lib.rs");
        std::fs::write(&fp, "x").unwrap();

        let files = vec![ScannedFile {
            relative_path: "src/lib.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 1,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/lib.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "my_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn my_fn()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let naming = extract_naming(&index);

        // file_contributions must contain the file
        assert!(naming.file_contributions.contains_key("src/lib.rs"));
        let contrib = &naming.file_contributions["src/lib.rs"];
        // snake_case function was counted
        assert_eq!(contrib.counts.get("fn:snake_case").copied().unwrap_or(0), 1);
    }

    #[test]
    fn test_remove_file_contribution_removes_entry() {
        use crate::budget::counter::TokenCounter;
        use crate::scanner::ScannedFile;

        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("lib.rs");
        std::fs::write(&fp, "x").unwrap();

        let files = vec![ScannedFile {
            relative_path: "src/lib.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 1,
        }];

        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let mut naming = extract_naming(&index);

        assert!(naming.file_contributions.contains_key("src/lib.rs"));
        remove_file_contribution(&mut naming, "src/lib.rs");
        assert!(!naming.file_contributions.contains_key("src/lib.rs"));
    }

    #[test]
    fn test_update_file_contribution_is_noop() {
        use crate::budget::counter::TokenCounter;
        use crate::scanner::ScannedFile;

        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("lib.rs");
        std::fs::write(&fp, "x").unwrap();

        let files = vec![ScannedFile {
            relative_path: "src/lib.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 1,
        }];

        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let mut naming = extract_naming(&index);
        let before_len = naming.file_contributions.len();

        let file = &index.files[0];
        update_file_contribution(&mut naming, file);

        assert_eq!(naming.file_contributions.len(), before_len);
    }
}
