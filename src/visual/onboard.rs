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

/// A file included in an onboarding phase with focus guidance.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct OnboardingFile {
    /// Relative path to the file within the repository.
    pub path: String,
    /// PageRank importance score for this file (0.0–1.0).
    pub pagerank: f64,
    /// Key symbols a new developer should focus on when reading this file.
    pub symbols_to_focus_on: Vec<String>,
    /// Approximate token count for reading-time estimation.
    pub estimated_tokens: usize,
}

/// A logical grouping of files that a developer should read together.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct OnboardingPhase {
    /// Human-readable phase name (e.g. "Entry Points", "Core Logic").
    pub name: String,
    /// Module or directory prefix for this phase (e.g. "src/commands").
    pub module: String,
    /// Why this phase should be read at this point in the learning journey.
    pub rationale: String,
    /// Files to read in this phase, ordered by reading priority.
    pub files: Vec<OnboardingFile>,
}

/// A guided onboarding map for navigating an unfamiliar codebase.
///
/// Produced by [`build_onboarding_map`] and consumed by the MCP tool
/// and the interactive onboarding UI (Task 7 dashboard).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct OnboardingMap {
    /// Total number of files included across all phases.
    pub total_files: usize,
    /// Human-readable estimate of reading time (e.g. "~4 hours").
    pub estimated_reading_time: String,
    /// Phases in recommended reading order.
    pub phases: Vec<OnboardingPhase>,
}

/// Identify the module prefix (first two path components) for a file path.
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

/// Build a basic onboarding map from the index.
///
/// Groups files by module prefix, sorts each group by PageRank, and creates
/// a phase per module. Entry points always come first as their own phase.
///
/// Tasks 16-20 will replace this with the full topological-sort pipeline
/// including 7±2 phase grouping and precise reading-time calculation.
pub fn build_onboarding_map(
    index: &crate::index::CodebaseIndex,
    _focus: Option<&str>,
) -> OnboardingMap {
    use std::collections::HashMap;

    // Separate entry points from the rest.
    let mut entry_files: Vec<OnboardingFile> = Vec::new();
    let mut module_files: HashMap<String, Vec<OnboardingFile>> = HashMap::new();

    for file in &index.files {
        let path = &file.relative_path;
        let pagerank = *index.pagerank.get(path.as_str()).unwrap_or(&0.0);

        // Collect public symbol names to suggest as focus points.
        let symbols_to_focus_on: Vec<String> = file
            .parse_result
            .as_ref()
            .map(|pr| {
                pr.symbols
                    .iter()
                    .filter(|s| matches!(s.visibility, crate::parser::language::Visibility::Public))
                    .map(|s| s.name.clone())
                    .take(3)
                    .collect()
            })
            .unwrap_or_default();

        let onboard_file = OnboardingFile {
            path: path.clone(),
            pagerank,
            symbols_to_focus_on,
            estimated_tokens: file.token_count,
        };

        if is_entry_point(path) {
            entry_files.push(onboard_file);
        } else {
            let module = module_prefix(path);
            module_files.entry(module).or_default().push(onboard_file);
        }
    }

    // Sort entry files by pagerank descending.
    entry_files.sort_by(|a, b| {
        b.pagerank
            .partial_cmp(&a.pagerank)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Build phases: entry points first, then modules sorted by total pagerank.
    let mut phases: Vec<OnboardingPhase> = Vec::new();

    if !entry_files.is_empty() {
        let total_entry_tokens: usize = entry_files.iter().map(|f| f.estimated_tokens).sum();
        phases.push(OnboardingPhase {
            name: "Entry Points".to_string(),
            module: "entry".to_string(),
            rationale: "Start here to understand how the codebase is structured and where execution begins. These files define the public interface and top-level flow.".to_string(),
            files: entry_files,
        });
        let _ = total_entry_tokens;
    }

    // Sort module groups by cumulative pagerank (most important modules first).
    let mut module_vec: Vec<(String, Vec<OnboardingFile>)> = module_files.into_iter().collect();
    module_vec.sort_by(|(_, files_a), (_, files_b)| {
        let rank_a: f64 = files_a.iter().map(|f| f.pagerank).sum();
        let rank_b: f64 = files_b.iter().map(|f| f.pagerank).sum();
        rank_b
            .partial_cmp(&rank_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for (module, mut files) in module_vec {
        // Sort files within the module by pagerank descending.
        files.sort_by(|a, b| {
            b.pagerank
                .partial_cmp(&a.pagerank)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let name = module
            .split('/')
            .next_back()
            .map(|s| {
                let mut chars = s.chars();
                match chars.next() {
                    None => String::new(),
                    Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .unwrap_or_else(|| module.clone());
        phases.push(OnboardingPhase {
            name,
            module: module.clone(),
            rationale: format!("Understand the `{module}` module, which contains related functionality grouped together."),
            files,
        });
    }

    // Calculate total tokens across all phases for reading-time estimation.
    let total_tokens: usize = phases
        .iter()
        .flat_map(|p| p.files.iter())
        .map(|f| f.estimated_tokens)
        .sum();

    // Estimate reading time: ~200 tokens per minute for code.
    let minutes = (total_tokens as f64 / 200.0).ceil() as usize;
    let estimated_reading_time = if minutes < 60 {
        format!("~{minutes} minutes")
    } else {
        let hours = minutes / 60;
        let remaining = minutes % 60;
        if remaining == 0 {
            format!("~{hours} hour{}", if hours == 1 { "" } else { "s" })
        } else {
            format!("~{hours}h {remaining}m")
        }
    };

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
        let map = build_onboarding_map(&index, None);
        assert!(!map.phases.is_empty(), "map should have at least one phase");
    }

    #[test]
    fn test_onboarding_map_entry_points_first() {
        let index = make_test_index();
        let map = build_onboarding_map(&index, None);
        // Entry points phase should be first
        assert_eq!(
            map.phases[0].name, "Entry Points",
            "first phase should be Entry Points"
        );
    }

    #[test]
    fn test_onboarding_map_total_files() {
        let index = make_test_index();
        let map = build_onboarding_map(&index, None);
        // total_files must equal sum of files across all phases
        let sum: usize = map.phases.iter().map(|p| p.files.len()).sum();
        assert_eq!(map.total_files, sum);
    }

    #[test]
    fn test_onboarding_map_serialization() {
        let index = make_test_index();
        let map = build_onboarding_map(&index, None);
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
        let map = build_onboarding_map(&index, None);
        let md = render_onboarding_markdown(&map);
        assert!(
            md.contains("# Codebase Onboarding Map"),
            "should have h1 title"
        );
        assert!(md.contains("Phase 1:"), "should have Phase 1");
        assert!(
            md.contains("estimated_reading_time") || md.contains("minutes") || md.contains("hour"),
            "should mention reading time"
        );
    }

    #[test]
    fn test_render_onboarding_json() {
        let index = make_test_index();
        let map = build_onboarding_map(&index, None);
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
        let map = build_onboarding_map(&index, None);
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
        // A tiny index: reading time should be in minutes
        let files = vec![ScannedFile {
            relative_path: "src/tiny.rs".to_string(),
            absolute_path: PathBuf::from("/tmp/src/tiny.rs"),
            language: Some("rust".to_string()),
            size_bytes: 10,
        }];
        let mut content_map = HashMap::new();
        content_map.insert("src/tiny.rs".to_string(), "fn x() {}".to_string());
        let index = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);
        let map = build_onboarding_map(&index, None);
        // Should be a very small reading time
        assert!(
            map.estimated_reading_time.contains("minute"),
            "small index should show minutes, got: {}",
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
