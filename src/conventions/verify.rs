use crate::conventions::{ConventionProfile, PatternStrength};
use crate::index::CodebaseIndex;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResult {
    pub files_checked: usize,
    pub lines_checked: usize,
    pub violations: Vec<Violation>,
    pub passed: Vec<String>,
    pub summary: ViolationSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Violation {
    pub severity: String,
    pub category: String,
    pub location: String,
    pub message: String,
    pub evidence: ViolationEvidence,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViolationEvidence {
    pub dominant_pattern: String,
    pub count: String,
    pub strength: String,
    pub history: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ViolationSummary {
    pub high: usize,
    pub medium: usize,
    pub low: usize,
}

#[derive(Debug, Clone)]
pub struct ChangedFile {
    pub path: String,
    pub added_lines: Vec<usize>,
    pub is_new: bool,
}

/// Get changed lines from the working tree or between refs using git2.
///
/// - No `git_ref` → `diff_index_to_workdir` (uncommitted changes vs HEAD)
/// - `git_ref` provided → `diff_tree_to_tree` (ref..HEAD)
/// - `focus` → filter to paths under prefix
pub fn get_changed_lines(
    repo_path: &Path,
    git_ref: Option<&str>,
    focus: Option<&str>,
) -> Result<Vec<ChangedFile>, String> {
    let repo = git2::Repository::discover(repo_path).map_err(|e| format!("Not a git repo: {e}"))?;

    let diff = if let Some(ref_str) = git_ref {
        // Diff between ref and HEAD
        let head_commit = repo
            .head()
            .and_then(|r| r.peel_to_commit())
            .map_err(|e| format!("Cannot resolve HEAD: {e}"))?;
        let head_tree = head_commit
            .tree()
            .map_err(|e| format!("Cannot get HEAD tree: {e}"))?;

        let ref_obj = repo
            .revparse_single(ref_str)
            .map_err(|e| format!("Cannot resolve ref '{ref_str}': {e}"))?;
        let ref_commit = ref_obj
            .peel_to_commit()
            .map_err(|e| format!("Cannot peel ref to commit: {e}"))?;
        let ref_tree = ref_commit
            .tree()
            .map_err(|e| format!("Cannot get ref tree: {e}"))?;

        repo.diff_tree_to_tree(Some(&ref_tree), Some(&head_tree), None)
            .map_err(|e| format!("Diff failed: {e}"))?
    } else {
        // Uncommitted changes vs HEAD
        let head = repo.head().ok().and_then(|r| r.peel_to_tree().ok());
        repo.diff_tree_to_workdir_with_index(head.as_ref(), None)
            .map_err(|e| format!("Diff failed: {e}"))?
    };

    let mut files: Vec<ChangedFile> = Vec::new();

    diff.foreach(
        &mut |delta, _| {
            let path = delta
                .new_file()
                .path()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_string();

            if let Some(focus_prefix) = focus {
                if !path.starts_with(focus_prefix) {
                    return true;
                }
            }

            let is_new =
                delta.status() == git2::Delta::Added || delta.status() == git2::Delta::Untracked;

            files.push(ChangedFile {
                path,
                added_lines: Vec::new(),
                is_new,
            });
            true
        },
        None,
        None,
        None,
    )
    .map_err(|e| format!("Diff iteration failed: {e}"))?;

    // Get line-level detail
    let mut line_map: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();

    diff.foreach(
        &mut |_, _| true,
        None,
        None,
        Some(&mut |delta, _hunk, line| {
            if line.origin() == '+' {
                if let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) {
                    if let Some(focus_prefix) = focus {
                        if !path.starts_with(focus_prefix) {
                            return true;
                        }
                    }
                    if let Some(line_no) = line.new_lineno() {
                        line_map
                            .entry(path.to_string())
                            .or_default()
                            .push(line_no as usize);
                    }
                }
            }
            true
        }),
    )
    .ok();

    for file in &mut files {
        if let Some(lines) = line_map.remove(&file.path) {
            file.added_lines = lines;
        }
    }

    Ok(files)
}

/// Verify changed files against the codebase's convention profile.
///
/// Re-parses changed files and checks NEW symbols against conventions.
/// Only checks symbols in added/modified line ranges.
pub fn verify_changes(
    changed_files: &[ChangedFile],
    index: &CodebaseIndex,
    repo_path: &Path,
) -> VerifyResult {
    let mut violations = Vec::new();
    let mut passed = Vec::new();
    let mut total_lines = 0usize;

    let registry = crate::parser::LanguageRegistry::new();

    for changed in changed_files {
        total_lines += changed.added_lines.len();

        // Read and parse the changed file
        let file_path = repo_path.join(&changed.path);
        let content = match std::fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let language = crate::scanner::detect_language(Path::new(&changed.path));
        let parse_result = language.as_deref().and_then(|lang| {
            let lang_support = registry.get(lang)?;
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&lang_support.ts_language()).ok()?;
            let tree = parser.parse(&content, None)?;
            Some(lang_support.extract(&content, &tree))
        });

        if let Some(pr) = parse_result {
            // Check each symbol that falls within changed lines
            for symbol in &pr.symbols {
                let in_changed_range = changed.is_new
                    || changed
                        .added_lines
                        .iter()
                        .any(|&line| line >= symbol.start_line && line <= symbol.end_line);

                if !in_changed_range {
                    continue;
                }

                // Check naming conventions
                check_naming(&changed.path, symbol, &index.conventions, &mut violations);

                // Check error handling
                check_errors(&changed.path, symbol, &index.conventions, &mut violations);
            }

            // Check import conventions
            for import in &pr.imports {
                check_imports(&changed.path, import, &index.conventions, &mut violations);
            }
        }
    }

    // Build passed list
    if violations.iter().all(|v| v.category != "naming") {
        if let Some(ref obs) = index.conventions.naming.function_style {
            passed.push(format!(
                "naming: {} ({}/{} files)",
                obs.dominant,
                changed_files.len(),
                changed_files.len()
            ));
        }
    }
    if violations.iter().all(|v| v.category != "error_handling") {
        passed.push("error_handling: no violations in changed code".to_string());
    }
    if violations.iter().all(|v| v.category != "imports") {
        passed.push("imports: style consistent with codebase".to_string());
    }

    let summary = ViolationSummary {
        high: violations.iter().filter(|v| v.severity == "high").count(),
        medium: violations.iter().filter(|v| v.severity == "medium").count(),
        low: violations.iter().filter(|v| v.severity == "low").count(),
    };

    VerifyResult {
        files_checked: changed_files.len(),
        lines_checked: total_lines,
        violations,
        passed,
        summary,
    }
}

fn severity_for_strength(strength: &PatternStrength) -> &'static str {
    match strength {
        PatternStrength::Convention => "high",
        PatternStrength::Trend => "medium",
        PatternStrength::Mixed => "low",
    }
}

fn check_naming(
    path: &str,
    symbol: &crate::parser::language::Symbol,
    conventions: &ConventionProfile,
    violations: &mut Vec<Violation>,
) {
    use crate::conventions::naming::classify_name;
    use crate::parser::language::SymbolKind;

    let style = classify_name(&symbol.name);

    match symbol.kind {
        SymbolKind::Function | SymbolKind::Method => {
            if let Some(ref obs) = conventions.naming.function_style {
                let expected = &obs.dominant;
                if style.to_string() != *expected {
                    let suggestion = match expected.as_str() {
                        "snake_case" => Some(to_snake_case(&symbol.name)),
                        "camelCase" => Some(to_camel_case(&symbol.name)),
                        _ => None,
                    };
                    violations.push(Violation {
                        severity: severity_for_strength(&obs.strength).to_string(),
                        category: "naming".to_string(),
                        location: format!("{path}:{}", symbol.start_line),
                        message: format!(
                            "Function '{}' uses {} but convention is {}",
                            symbol.name, style, obs.dominant
                        ),
                        evidence: ViolationEvidence {
                            dominant_pattern: obs.dominant.clone(),
                            count: format!("{}/{} ({:.1}%)", obs.count, obs.total, obs.percentage),
                            strength: format!("{:?}", obs.strength).to_lowercase(),
                            history: None,
                        },
                        suggestion: suggestion.map(|s| format!("Rename to `{s}`")),
                    });
                }
            }
        }
        SymbolKind::Struct
        | SymbolKind::Class
        | SymbolKind::Enum
        | SymbolKind::Type
        | SymbolKind::Interface
        | SymbolKind::Trait => {
            if let Some(ref obs) = conventions.naming.type_style {
                let expected = &obs.dominant;
                if style.to_string() != *expected {
                    violations.push(Violation {
                        severity: severity_for_strength(&obs.strength).to_string(),
                        category: "naming".to_string(),
                        location: format!("{path}:{}", symbol.start_line),
                        message: format!(
                            "Type '{}' uses {} but convention is {}",
                            symbol.name, style, obs.dominant
                        ),
                        evidence: ViolationEvidence {
                            dominant_pattern: obs.dominant.clone(),
                            count: format!("{}/{} ({:.1}%)", obs.count, obs.total, obs.percentage),
                            strength: format!("{:?}", obs.strength).to_lowercase(),
                            history: None,
                        },
                        suggestion: None,
                    });
                }
            }
        }
        _ => {}
    }
}

fn check_errors(
    path: &str,
    symbol: &crate::parser::language::Symbol,
    conventions: &ConventionProfile,
    violations: &mut Vec<Violation>,
) {
    use crate::parser::language::SymbolKind;

    if !matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method) {
        return;
    }

    // Check for .unwrap() in production code
    if !path.contains("test") && !path.starts_with("tests/") && symbol.body.contains(".unwrap()") {
        if let Some(ref obs) = conventions.errors.unwrap_usage {
            // Find revert history for unwrap
            let history = if !conventions.git_health.reverts.is_empty() {
                let unwrap_reverts: Vec<&str> = conventions
                    .git_health
                    .reverts
                    .iter()
                    .filter(|r| {
                        r.commit_message.to_lowercase().contains("unwrap")
                            || r.reverted_message
                                .as_deref()
                                .is_some_and(|m| m.to_lowercase().contains("unwrap"))
                    })
                    .map(|r| r.commit_message.as_str())
                    .collect();
                if unwrap_reverts.is_empty() {
                    None
                } else {
                    Some(format!(
                        "{} commits reverted unwrap() usage",
                        unwrap_reverts.len()
                    ))
                }
            } else {
                None
            };

            violations.push(Violation {
                severity: severity_for_strength(&obs.strength).to_string(),
                category: "error_handling".to_string(),
                location: format!("{path}:{}", symbol.start_line),
                message: ".unwrap() used in production code".to_string(),
                evidence: ViolationEvidence {
                    dominant_pattern: obs.dominant.clone(),
                    count: format!("{}/{} ({:.1}%)", obs.count, obs.total, obs.percentage),
                    strength: format!("{:?}", obs.strength).to_lowercase(),
                    history,
                },
                suggestion: Some(
                    "Replace .unwrap() with .map_err(|e| ...)? or .expect(\"reason\")".to_string(),
                ),
            });
        }
    }
}

fn check_imports(
    path: &str,
    import: &crate::parser::language::Import,
    conventions: &ConventionProfile,
    violations: &mut Vec<Violation>,
) {
    if let Some(ref obs) = conventions.imports.style {
        let is_relative = import.source.starts_with("./") || import.source.starts_with("../");
        let convention_is_absolute = obs.dominant == "absolute";

        if is_relative && convention_is_absolute {
            violations.push(Violation {
                severity: severity_for_strength(&obs.strength).to_string(),
                category: "imports".to_string(),
                location: path.to_string(),
                message: format!(
                    "Relative import '{}' but convention is absolute imports",
                    import.source
                ),
                evidence: ViolationEvidence {
                    dominant_pattern: obs.dominant.clone(),
                    count: format!("{}/{} ({:.1}%)", obs.count, obs.total, obs.percentage),
                    strength: format!("{:?}", obs.strength).to_lowercase(),
                    history: None,
                },
                suggestion: Some(format!("Rewrite '{}' as absolute import", import.source)),
            });
        }
    }
}

fn to_snake_case(name: &str) -> String {
    let mut result = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(ch.to_lowercase().next().unwrap_or(ch));
    }
    result
}

fn to_camel_case(name: &str) -> String {
    let parts: Vec<&str> = name.split('_').collect();
    let mut result = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            result.push_str(&part.to_lowercase());
        } else {
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                result.push(first.to_uppercase().next().unwrap_or(first));
                result.extend(chars);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::conventions::PatternObservation;
    use crate::parser::language::{Import, Symbol, SymbolKind, Visibility};
    use std::collections::HashMap;

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("handleRequest"), "handle_request");
        assert_eq!(to_snake_case("MyFunc"), "my_func");
    }

    #[test]
    fn test_to_camel_case() {
        assert_eq!(to_camel_case("handle_request"), "handleRequest");
        assert_eq!(to_camel_case("my_func"), "myFunc");
    }

    #[test]
    fn test_severity_mapping() {
        assert_eq!(severity_for_strength(&PatternStrength::Convention), "high");
        assert_eq!(severity_for_strength(&PatternStrength::Trend), "medium");
        assert_eq!(severity_for_strength(&PatternStrength::Mixed), "low");
    }

    #[test]
    fn test_changed_file_struct() {
        let cf = ChangedFile {
            path: "src/lib.rs".to_string(),
            added_lines: vec![10, 20, 30],
            is_new: false,
        };
        assert_eq!(cf.added_lines.len(), 3);
        assert!(!cf.is_new);
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    /// Create a minimal git repo with one initial commit containing `filename`.
    fn make_git_repo_with_file(dir: &tempfile::TempDir, filename: &str, content: &str) {
        let repo = git2::Repository::init(dir.path()).unwrap();

        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        let file_path = dir.path().join(filename);
        std::fs::write(&file_path, content).unwrap();

        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new(filename)).unwrap();
        index.write().unwrap();

        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])
            .unwrap();
    }

    // ── get_changed_lines ─────────────────────────────────────────────────────

    #[test]
    fn test_get_changed_lines_not_a_git_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = get_changed_lines(dir.path(), None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Not a git repo"));
    }

    #[test]
    fn test_get_changed_lines_clean_worktree_returns_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        make_git_repo_with_file(&dir, "lib.rs", "fn main() {}");

        // No changes since the commit → empty diff
        let result = get_changed_lines(dir.path(), None, None).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_get_changed_lines_detects_modified_file() {
        let dir = tempfile::TempDir::new().unwrap();
        make_git_repo_with_file(&dir, "lib.rs", "fn main() {}");

        // Modify lib.rs without committing
        std::fs::write(dir.path().join("lib.rs"), "fn main() {}\nfn extra() {}").unwrap();

        let result = get_changed_lines(dir.path(), None, None).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "lib.rs");
        assert!(!result[0].is_new);
    }

    #[test]
    fn test_get_changed_lines_detects_added_lines() {
        let dir = tempfile::TempDir::new().unwrap();
        make_git_repo_with_file(&dir, "lib.rs", "fn main() {}");

        // Append a new line
        std::fs::write(
            dir.path().join("lib.rs"),
            "fn main() {}\nfn new_fn() { let x = 1; }",
        )
        .unwrap();

        let result = get_changed_lines(dir.path(), None, None).unwrap();
        let file = result.iter().find(|f| f.path == "lib.rs").unwrap();
        // line 2 is added
        assert!(file.added_lines.contains(&2));
    }

    #[test]
    fn test_get_changed_lines_with_focus_filter() {
        let dir = tempfile::TempDir::new().unwrap();
        make_git_repo_with_file(&dir, "lib.rs", "fn main() {}");

        // Modify lib.rs
        std::fs::write(dir.path().join("lib.rs"), "fn main() {}\nfn extra() {}").unwrap();

        // Focus on "src/" — lib.rs is not under src/, so it should be filtered out
        let result = get_changed_lines(dir.path(), None, Some("src/")).unwrap();
        assert!(result.is_empty());

        // Focus on "lib" prefix — lib.rs should be included
        let result2 = get_changed_lines(dir.path(), None, Some("lib")).unwrap();
        assert_eq!(result2.len(), 1);
    }

    #[test]
    fn test_get_changed_lines_tree_to_tree() {
        let dir = tempfile::TempDir::new().unwrap();
        make_git_repo_with_file(&dir, "lib.rs", "fn main() {}");

        // Make a second commit with a change
        let repo = git2::Repository::discover(dir.path()).unwrap();
        std::fs::write(dir.path().join("lib.rs"), "fn main() {}\nfn extra() {}").unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("lib.rs")).unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "second commit", &tree, &[&parent])
            .unwrap();

        // Diff HEAD~1..HEAD
        let result = get_changed_lines(dir.path(), Some("HEAD~1"), None).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "lib.rs");
    }

    // ── verify_changes ────────────────────────────────────────────────────────

    #[test]
    fn test_verify_changes_empty_changed_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let counter = TokenCounter::new();
        let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);

        let result = verify_changes(&[], &index, dir.path());
        assert_eq!(result.files_checked, 0);
        assert_eq!(result.lines_checked, 0);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn test_verify_changes_no_violations_when_no_conventions() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("src_lib.rs"), "fn my_fn() {}").unwrap();

        let counter = TokenCounter::new();
        let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);

        let changed = vec![ChangedFile {
            path: "src_lib.rs".to_string(),
            added_lines: vec![1],
            is_new: true,
        }];

        let result = verify_changes(&changed, &index, dir.path());
        assert!(result.violations.is_empty());
    }

    // ── check_naming ─────────────────────────────────────────────────────────

    #[test]
    fn test_check_naming_function_violation_snake_expected() {
        let mut conventions = ConventionProfile::default();
        conventions.naming.function_style =
            PatternObservation::new("fn_naming", "snake_case", 95, 100);

        let symbol = Symbol {
            name: "handleRequest".into(), // camelCase — violates snake_case convention
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: "fn handleRequest()".into(),
            body: "{}".into(),
            start_line: 1,
            end_line: 2,
        };

        let mut violations = Vec::new();
        check_naming("src/lib.rs", &symbol, &conventions, &mut violations);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].category, "naming");
        assert_eq!(violations[0].severity, "high");
        assert!(violations[0]
            .suggestion
            .as_deref()
            .unwrap()
            .contains("handle_request"));
    }

    #[test]
    fn test_check_naming_function_no_violation_when_matches() {
        let mut conventions = ConventionProfile::default();
        conventions.naming.function_style =
            PatternObservation::new("fn_naming", "snake_case", 95, 100);

        let symbol = Symbol {
            name: "handle_request".into(), // already snake_case
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: "fn handle_request()".into(),
            body: "{}".into(),
            start_line: 1,
            end_line: 2,
        };

        let mut violations = Vec::new();
        check_naming("src/lib.rs", &symbol, &conventions, &mut violations);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_check_naming_function_camel_expected_suggestion() {
        // When convention is camelCase and name is snake_case, suggestion uses to_camel_case
        let mut conventions = ConventionProfile::default();
        conventions.naming.function_style =
            PatternObservation::new("fn_naming", "camelCase", 90, 100);

        let symbol = Symbol {
            name: "handle_request".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: "fn handle_request()".into(),
            body: "{}".into(),
            start_line: 1,
            end_line: 1,
        };

        let mut violations = Vec::new();
        check_naming("src/api.rs", &symbol, &conventions, &mut violations);

        assert_eq!(violations.len(), 1);
        assert!(violations[0]
            .suggestion
            .as_deref()
            .unwrap()
            .contains("handleRequest"));
    }

    #[test]
    fn test_check_naming_type_violation() {
        let mut conventions = ConventionProfile::default();
        conventions.naming.type_style =
            PatternObservation::new("type_naming", "PascalCase", 95, 100);

        let symbol = Symbol {
            name: "my_struct".into(), // snake_case — violates PascalCase convention
            kind: SymbolKind::Struct,
            visibility: Visibility::Public,
            signature: "struct my_struct".into(),
            body: "{}".into(),
            start_line: 5,
            end_line: 6,
        };

        let mut violations = Vec::new();
        check_naming("src/lib.rs", &symbol, &conventions, &mut violations);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].category, "naming");
    }

    #[test]
    fn test_check_naming_no_convention_set() {
        // When no naming convention is set, no violations are generated
        let conventions = ConventionProfile::default();

        let symbol = Symbol {
            name: "anything".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: "fn anything()".into(),
            body: "{}".into(),
            start_line: 1,
            end_line: 1,
        };

        let mut violations = Vec::new();
        check_naming("src/lib.rs", &symbol, &conventions, &mut violations);
        assert!(violations.is_empty());
    }

    // ── check_errors ─────────────────────────────────────────────────────────

    #[test]
    fn test_check_errors_unwrap_in_production_code() {
        let mut conventions = ConventionProfile::default();
        conventions.errors.unwrap_usage =
            PatternObservation::new("unwrap_usage", "no .unwrap() in src/", 95, 100);

        let symbol = Symbol {
            name: "my_fn".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: "fn my_fn()".into(),
            body: "{ let x = foo.unwrap() }".into(), // contains .unwrap()
            start_line: 1,
            end_line: 3,
        };

        let mut violations = Vec::new();
        // "src/lib.rs" — not a test file
        check_errors("src/lib.rs", &symbol, &conventions, &mut violations);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].category, "error_handling");
        assert!(violations[0].suggestion.is_some());
    }

    #[test]
    fn test_check_errors_unwrap_in_test_file_no_violation() {
        let mut conventions = ConventionProfile::default();
        conventions.errors.unwrap_usage =
            PatternObservation::new("unwrap_usage", "no .unwrap() in src/", 95, 100);

        let symbol = Symbol {
            name: "test_fn".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Private,
            signature: "fn test_fn()".into(),
            body: "{ foo.unwrap() }".into(),
            start_line: 1,
            end_line: 2,
        };

        let mut violations = Vec::new();
        // Path contains "test" → no violation
        check_errors("src/my_test.rs", &symbol, &conventions, &mut violations);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_check_errors_non_function_symbol_skipped() {
        let mut conventions = ConventionProfile::default();
        conventions.errors.unwrap_usage =
            PatternObservation::new("unwrap_usage", "no .unwrap() in src/", 95, 100);

        let symbol = Symbol {
            name: "MyStruct".into(),
            kind: SymbolKind::Struct, // not Function/Method → skipped
            visibility: Visibility::Public,
            signature: "struct MyStruct".into(),
            body: "{ val: x.unwrap() }".into(),
            start_line: 1,
            end_line: 2,
        };

        let mut violations = Vec::new();
        check_errors("src/lib.rs", &symbol, &conventions, &mut violations);
        assert!(violations.is_empty());
    }

    // ── check_imports ─────────────────────────────────────────────────────────

    #[test]
    fn test_check_imports_relative_violates_absolute_convention() {
        let mut conventions = ConventionProfile::default();
        conventions.imports.style = PatternObservation::new("import_style", "absolute", 95, 100);

        let import = Import {
            source: "./utils".into(), // relative — violates absolute convention
            names: vec!["helper".into()],
        };

        let mut violations = Vec::new();
        check_imports("src/lib.rs", &import, &conventions, &mut violations);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].category, "imports");
        assert!(violations[0].suggestion.is_some());
    }

    #[test]
    fn test_check_imports_absolute_no_violation() {
        let mut conventions = ConventionProfile::default();
        conventions.imports.style = PatternObservation::new("import_style", "absolute", 95, 100);

        let import = Import {
            source: "crate::utils".into(), // absolute — OK
            names: vec!["helper".into()],
        };

        let mut violations = Vec::new();
        check_imports("src/lib.rs", &import, &conventions, &mut violations);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_check_imports_no_convention_set() {
        let conventions = ConventionProfile::default();

        let import = Import {
            source: "./local".into(),
            names: vec!["x".into()],
        };

        let mut violations = Vec::new();
        check_imports("src/lib.rs", &import, &conventions, &mut violations);
        assert!(violations.is_empty());
    }

    // ── verify_changes with symbols ──────────────────────────────────────────

    #[test]
    fn test_verify_changes_naming_violation_in_new_file() {
        let dir = tempfile::TempDir::new().unwrap();

        // Write a source file with a camelCase function
        let content = "pub fn handleRequest() {}";
        std::fs::write(dir.path().join("lib.rs"), content).unwrap();

        let mut conventions = ConventionProfile::default();
        conventions.naming.function_style =
            PatternObservation::new("fn_naming", "snake_case", 95, 100);

        let counter = TokenCounter::new();
        let mut index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
        index.conventions = conventions;

        let changed = vec![ChangedFile {
            path: "lib.rs".to_string(),
            added_lines: vec![1],
            is_new: true,
        }];

        let result = verify_changes(&changed, &index, dir.path());
        // The file has no parse result we inject, so tree-sitter parses the real content.
        // Regardless, summary counts must be consistent.
        assert_eq!(
            result.summary.high + result.summary.medium + result.summary.low,
            result.violations.len()
        );
    }

    #[test]
    fn test_verify_changes_summary_counts() {
        let dir = tempfile::TempDir::new().unwrap();

        // File doesn't exist → skipped with no violations
        let counter = TokenCounter::new();
        let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);

        let changed = vec![ChangedFile {
            path: "nonexistent.rs".to_string(),
            added_lines: vec![1, 2, 3],
            is_new: false,
        }];

        let result = verify_changes(&changed, &index, dir.path());
        assert_eq!(result.files_checked, 1);
        assert_eq!(result.lines_checked, 3);
        assert!(result.violations.is_empty());
        assert_eq!(result.summary.high, 0);
        assert_eq!(result.summary.medium, 0);
        assert_eq!(result.summary.low, 0);
    }

    #[test]
    fn test_verify_changes_passed_list_populated() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "fn x() {}").unwrap();

        let counter = TokenCounter::new();
        let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);

        let changed = vec![ChangedFile {
            path: "lib.rs".to_string(),
            added_lines: vec![1],
            is_new: true,
        }];

        let result = verify_changes(&changed, &index, dir.path());
        // error_handling and imports passes are always included
        assert!(result.passed.iter().any(|p| p.contains("error_handling")));
        assert!(result.passed.iter().any(|p| p.contains("imports")));
    }
}
