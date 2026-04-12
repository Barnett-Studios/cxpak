//! Onboarding map for navigating unfamiliar codebases.
//!
//! The onboard module generates guided learning maps that help developers
//! understand a codebase by walking through entry points, dependencies,
//! and patterns in a structured reading order.
//!
//! Tasks 16-20 will implement the full pipeline:
//! - Task 16: OnboardingMap module scaffold
//! - Task 17: OnboardingMap types
//! - Task 18: Topological sort for reading order
//! - Task 19: Phase grouping with 7±2 constraint
//! - Task 20: Estimated reading time calculation

// Types live in the intelligence module; re-export here for backward compatibility.
pub use crate::intelligence::onboarding::{OnboardingFile, OnboardingMap, OnboardingPhase};

/// Identify the module prefix (first two path components) for a file path.
#[cfg(test)]
fn module_prefix(path: &str) -> String {
    let parts: Vec<&str> = path.splitn(3, '/').collect();
    if parts.len() >= 2 {
        format!("{}/{}", parts[0], parts[1])
    } else {
        parts[0].to_string()
    }
}

/// Returns true if a path is a conventional entry point.
fn is_entry_point(path: &str) -> bool {
    path == "src/main.rs"
        || path == "src/lib.rs"
        || path == "main.py"
        || path == "index.ts"
        || path == "index.js"
        || path.ends_with("/main.rs")
        || path.ends_with("/lib.rs")
        || path.ends_with("/main.py")
        || path.ends_with("/index.ts")
        || path.ends_with("/index.js")
}

/// Compute an onboarding map from the index using the intelligence pipeline.
///
/// Entry points are placed first as their own phase. The remaining files are
/// sorted topologically (dependency-first) and grouped into phases of ≤9
/// files per module, ordered by aggregate PageRank. Reading time is
/// estimated via [`crate::intelligence::onboarding::format_reading_time`].
pub fn compute_onboarding_map(
    index: &crate::index::CodebaseIndex,
    _focus: Option<&str>,
) -> OnboardingMap {
    use crate::intelligence::onboarding::{
        format_reading_time, group_into_phases, topological_sort_files,
    };
    use std::collections::HashMap;

    // Build exclusion set: test files (from test_map keys) and
    // generated/vendored paths (noise filter blocklist).
    let excluded: std::collections::HashSet<&str> = index
        .test_map
        .keys()
        .map(|k| k.as_str())
        .chain(
            index
                .files
                .iter()
                .map(|f| f.relative_path.as_str())
                .filter(|p| crate::auto_context::noise::is_blocklisted(p)),
        )
        .collect();

    // Build per-file metadata maps for the intelligence functions.
    let mut file_tokens: HashMap<String, usize> = HashMap::new();
    let mut file_symbols: HashMap<String, Vec<String>> = HashMap::new();
    let mut entry_files: Vec<OnboardingFile> = Vec::new();
    let mut non_entry_paths: Vec<String> = Vec::new();

    for file in &index.files {
        if excluded.contains(file.relative_path.as_str()) {
            continue;
        }
        let path = &file.relative_path;
        let pagerank = *index.pagerank.get(path.as_str()).unwrap_or(&0.0);

        file_tokens.insert(path.clone(), file.token_count);

        let symbols: Vec<String> = file
            .parse_result
            .as_ref()
            .map(|pr| {
                pr.symbols
                    .iter()
                    .filter(|s| matches!(s.visibility, crate::parser::language::Visibility::Public))
                    .map(|s| s.name.clone())
                    .take(5)
                    .collect()
            })
            .unwrap_or_default();
        file_symbols.insert(path.clone(), symbols.clone());

        if is_entry_point(path) {
            entry_files.push(OnboardingFile {
                path: path.clone(),
                pagerank,
                symbols_to_focus_on: symbols,
                estimated_tokens: file.token_count,
            });
        } else {
            non_entry_paths.push(path.clone());
        }
    }

    // Sort entry files by pagerank descending.
    entry_files.sort_by(|a, b| {
        b.pagerank
            .partial_cmp(&a.pagerank)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Build phases: entry points first.
    let mut phases: Vec<OnboardingPhase> = Vec::new();

    if !entry_files.is_empty() {
        phases.push(OnboardingPhase {
            name: "Entry Points".to_string(),
            module: "entry".to_string(),
            rationale: "Start here to understand how the codebase is structured and where execution begins. These files define the public interface and top-level flow.".to_string(),
            files: entry_files,
        });
    }

    // Sort remaining files topologically and group into phases.
    let non_entry_refs: Vec<&str> = non_entry_paths.iter().map(|s| s.as_str()).collect();
    let sorted = topological_sort_files(&non_entry_refs, &index.graph);
    let mut module_phases = group_into_phases(
        &sorted,
        &index.pagerank,
        &index.graph,
        &file_tokens,
        &file_symbols,
    );
    phases.append(&mut module_phases);

    // Calculate total tokens and reading time.
    let total_tokens: usize = phases
        .iter()
        .flat_map(|p| p.files.iter())
        .map(|f| f.estimated_tokens)
        .sum();
    let estimated_reading_time = format_reading_time(total_tokens);
    let total_files: usize = phases.iter().map(|p| p.files.len()).sum();

    OnboardingMap {
        total_files,
        estimated_reading_time,
        phases,
    }
}

/// Render an [`OnboardingMap`] as a Markdown document.
pub fn render_onboarding_markdown(map: &OnboardingMap) -> String {
    let mut out = String::new();

    out.push_str("# Codebase Onboarding Map\n\n");
    out.push_str(&format!(
        "> **{} files** — Estimated reading time: {}\n\n",
        map.total_files, map.estimated_reading_time
    ));

    for (i, phase) in map.phases.iter().enumerate() {
        out.push_str(&format!("## Phase {}: {}\n\n", i + 1, phase.name));
        out.push_str(&format!("_{}_\n\n", phase.rationale));
        for file in &phase.files {
            out.push_str(&format!(
                "- `{}` (pagerank: {:.3}, ~{} tokens)",
                file.path, file.pagerank, file.estimated_tokens
            ));
            if !file.symbols_to_focus_on.is_empty() {
                out.push_str(&format!(
                    " — focus on: {}",
                    file.symbols_to_focus_on.join(", ")
                ));
            }
            out.push('\n');
        }
        out.push('\n');
    }

    out
}

/// Render an [`OnboardingMap`] as a pretty-printed JSON string.
pub fn render_onboarding_json(map: &OnboardingMap) -> String {
    serde_json::to_string_pretty(map).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::index::CodebaseIndex;
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn make_test_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let files = vec![
            ScannedFile {
                relative_path: "src/main.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/main.rs"),
                language: Some("rust".to_string()),
                size_bytes: 200,
            },
            ScannedFile {
                relative_path: "src/lib.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/lib.rs"),
                language: Some("rust".to_string()),
                size_bytes: 400,
            },
            ScannedFile {
                relative_path: "src/commands/mod.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/commands/mod.rs"),
                language: Some("rust".to_string()),
                size_bytes: 100,
            },
            ScannedFile {
                relative_path: "src/parser/mod.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/parser/mod.rs"),
                language: Some("rust".to_string()),
                size_bytes: 150,
            },
        ];
        let mut parse_results = HashMap::new();
        use crate::parser::language::{Import, ParseResult, Symbol, SymbolKind, Visibility};
        parse_results.insert(
            "src/lib.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "run".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "pub fn run()".to_string(),
                    body: "{}".to_string(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![Import {
                    source: "std".to_string(),
                    names: vec![],
                }],
                exports: vec![],
            },
        );
        let mut content_map = HashMap::new();
        content_map.insert("src/main.rs".to_string(), "fn main() {}".to_string());
        content_map.insert("src/lib.rs".to_string(), "pub fn run() {}".to_string());
        content_map.insert(
            "src/commands/mod.rs".to_string(),
            "pub fn cmd() {}".to_string(),
        );
        content_map.insert(
            "src/parser/mod.rs".to_string(),
            "pub fn parse() {}".to_string(),
        );
        CodebaseIndex::build_with_content(files, parse_results, &counter, content_map)
    }

    #[test]
    fn test_onboarding_map_has_phases() {
        let index = make_test_index();
        let map = compute_onboarding_map(&index, None);
        assert!(!map.phases.is_empty(), "map should have at least one phase");
    }

    #[test]
    fn test_onboarding_map_entry_points_first() {
        let index = make_test_index();
        let map = compute_onboarding_map(&index, None);
        // Entry points phase should be first
        assert_eq!(
            map.phases[0].name, "Entry Points",
            "first phase should be Entry Points"
        );
    }

    #[test]
    fn test_onboarding_map_total_files() {
        let index = make_test_index();
        let map = compute_onboarding_map(&index, None);
        // total_files must equal sum of files across all phases
        let sum: usize = map.phases.iter().map(|p| p.files.len()).sum();
        assert_eq!(map.total_files, sum);
    }

    #[test]
    fn test_onboarding_map_serialization() {
        let index = make_test_index();
        let map = compute_onboarding_map(&index, None);
        let json = render_onboarding_json(&map);
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert!(parsed["total_files"].is_number());
        assert!(parsed["estimated_reading_time"].is_string());
        assert!(parsed["phases"].is_array());
        let phases = parsed["phases"].as_array().unwrap();
        assert!(!phases.is_empty());
        // Each phase must have name, module, rationale, files
        for phase in phases {
            assert!(phase["name"].is_string());
            assert!(phase["module"].is_string());
            assert!(phase["rationale"].is_string());
            assert!(phase["files"].is_array());
        }
    }

    #[test]
    fn test_render_onboarding_markdown() {
        let index = make_test_index();
        let map = compute_onboarding_map(&index, None);
        let md = render_onboarding_markdown(&map);
        assert!(
            md.contains("# Codebase Onboarding Map"),
            "should have h1 title"
        );
        assert!(md.contains("Phase 1:"), "should have Phase 1");
        assert!(
            md.contains("Estimated reading time:"),
            "should mention reading time"
        );
    }

    #[test]
    fn test_render_onboarding_json() {
        let index = make_test_index();
        let map = compute_onboarding_map(&index, None);
        let json = render_onboarding_json(&map);
        assert!(!json.is_empty());
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert!(parsed["phases"].is_array());
    }

    #[test]
    fn test_onboarding_map_empty_index() {
        let counter = TokenCounter::new();
        let index =
            CodebaseIndex::build_with_content(vec![], HashMap::new(), &counter, HashMap::new());
        let map = compute_onboarding_map(&index, None);
        assert_eq!(map.total_files, 0);
        assert!(map.phases.is_empty());
        // Reading time for 0 tokens
        assert!(
            map.estimated_reading_time.contains('~'),
            "reading time should start with ~"
        );
    }

    #[test]
    fn test_estimated_reading_time_minutes() {
        let counter = TokenCounter::new();
        // A tiny index: reading time should be a small number of minutes
        let files = vec![ScannedFile {
            relative_path: "src/tiny.rs".to_string(),
            absolute_path: PathBuf::from("/tmp/src/tiny.rs"),
            language: Some("rust".to_string()),
            size_bytes: 10,
        }];
        let mut content_map = HashMap::new();
        content_map.insert("src/tiny.rs".to_string(), "fn x() {}".to_string());
        let index = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);
        let map = compute_onboarding_map(&index, None);
        // format_reading_time returns "~Nm" for sub-hour durations
        assert!(
            map.estimated_reading_time.starts_with('~'),
            "reading time should start with ~, got: {}",
            map.estimated_reading_time
        );
        assert!(
            !map.estimated_reading_time.contains('h'),
            "small index should not show hours, got: {}",
            map.estimated_reading_time
        );
    }

    #[test]
    fn test_module_prefix_two_components() {
        assert_eq!(module_prefix("src/commands/serve.rs"), "src/commands");
        assert_eq!(module_prefix("src/lib.rs"), "src/lib.rs");
        assert_eq!(module_prefix("main.rs"), "main.rs");
    }

    #[test]
    fn test_is_entry_point() {
        assert!(is_entry_point("src/main.rs"));
        assert!(is_entry_point("src/lib.rs"));
        assert!(is_entry_point("main.py"));
        assert!(is_entry_point("index.ts"));
        assert!(is_entry_point("index.js"));
        assert!(is_entry_point("pkg/main.rs"));
        assert!(!is_entry_point("src/commands/serve.rs"));
        assert!(!is_entry_point("tests/integration.rs"));
    }
}
