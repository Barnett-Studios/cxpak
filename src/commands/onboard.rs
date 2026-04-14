use crate::budget::counter::TokenCounter;
use crate::cache;
use crate::cli::OutputFormat;
use crate::index::CodebaseIndex;
use crate::scanner::Scanner;
use crate::visual::onboard::{
    compute_onboarding_map, render_onboarding_json, render_onboarding_markdown, OnboardingMap,
};
use std::io::Write;
use std::path::Path;

fn build_index(path: &Path) -> Result<CodebaseIndex, Box<dyn std::error::Error>> {
    let counter = TokenCounter::new();
    let scanner = Scanner::new(path)?;
    let files = scanner.scan_workspace(None)?;
    if files.is_empty() {
        return Err("no source files found".into());
    }
    let (parse_results, content_map) =
        cache::parse::parse_with_cache(&files, path, &counter, false);
    let mut index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
    index.conventions = crate::conventions::build_convention_profile(&index, path);
    index.co_changes = index.conventions.git_health.co_changes.clone();
    Ok(index)
}

fn render_onboarding(map: &OnboardingMap, format: &OutputFormat) -> String {
    match format {
        OutputFormat::Markdown => render_onboarding_markdown(map),
        OutputFormat::Json => render_onboarding_json(map),
        OutputFormat::Xml => render_onboarding_xml(map),
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn render_onboarding_xml(map: &OnboardingMap) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<onboarding>\n");
    out.push_str(&format!(
        "  <total_files>{}</total_files>\n",
        map.total_files
    ));
    out.push_str(&format!(
        "  <estimated_reading_time>{}</estimated_reading_time>\n",
        xml_escape(&map.estimated_reading_time)
    ));
    for (i, phase) in map.phases.iter().enumerate() {
        out.push_str(&format!(
            "  <phase index=\"{}\" name=\"{}\" module=\"{}\">\n",
            i,
            xml_escape(&phase.name),
            xml_escape(&phase.module)
        ));
        out.push_str(&format!(
            "    <rationale>{}</rationale>\n",
            xml_escape(&phase.rationale)
        ));
        for file in &phase.files {
            out.push_str(&format!(
                "    <file path=\"{}\" pagerank=\"{:.3}\" tokens=\"{}\"/>\n",
                xml_escape(&file.path),
                file.pagerank,
                file.estimated_tokens
            ));
        }
        out.push_str("  </phase>\n");
    }
    out.push_str("</onboarding>\n");
    out
}

pub fn run(
    path: &Path,
    focus: Option<&str>,
    format: &OutputFormat,
    out: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let index = build_index(path)?;
    let map = compute_onboarding_map(&index, focus);
    let content = render_onboarding(&map, format);

    match out {
        Some(out_path) => {
            std::fs::write(out_path, &content)?;
            eprintln!("cxpak: wrote {}", out_path.display());
        }
        None => {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            handle.write_all(content.as_bytes())?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::index::CodebaseIndex;
    use crate::parser::language::{Import, ParseResult, Symbol, SymbolKind, Visibility};
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
                size_bytes: 100,
            },
            ScannedFile {
                relative_path: "src/lib.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/lib.rs"),
                language: Some("rust".to_string()),
                size_bytes: 200,
            },
            ScannedFile {
                relative_path: "src/commands/mod.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/commands/mod.rs"),
                language: Some("rust".to_string()),
                size_bytes: 50,
            },
        ];
        let mut parse_results = HashMap::new();
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
        content_map.insert(
            "src/lib.rs".to_string(),
            "use std;\npub fn run() {}".to_string(),
        );
        content_map.insert("src/commands/mod.rs".to_string(), String::new());
        CodebaseIndex::build_with_content(files, parse_results, &counter, content_map)
    }

    #[test]
    fn test_onboard_markdown_has_phases() {
        let index = make_test_index();
        let map = compute_onboarding_map(&index, None);
        let output = render_onboarding_markdown(&map);
        assert!(output.contains("# Codebase Onboarding Map"));
        assert!(output.contains("Phase"));
    }

    #[test]
    fn test_onboard_json_has_phases() {
        let index = make_test_index();
        let map = compute_onboarding_map(&index, None);
        let output = render_onboarding_json(&map);
        let parsed: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert!(parsed["phases"].is_array());
        assert!(parsed["total_files"].as_u64().unwrap() > 0);
        assert!(parsed["estimated_reading_time"].is_string());
    }

    #[test]
    fn test_onboard_xml_has_phases() {
        let index = make_test_index();
        let map = compute_onboarding_map(&index, None);
        let output = render_onboarding_xml(&map);
        assert!(output.contains("<onboarding>"));
        assert!(output.contains("<estimated_reading_time>"));
        assert!(output.contains("</onboarding>"));
    }

    #[test]
    fn test_onboard_dispatches_all_formats() {
        let index = make_test_index();
        let map = compute_onboarding_map(&index, None);
        let md = render_onboarding(&map, &OutputFormat::Markdown);
        let json = render_onboarding(&map, &OutputFormat::Json);
        let xml = render_onboarding(&map, &OutputFormat::Xml);
        assert!(!md.is_empty());
        assert!(!json.is_empty());
        assert!(!xml.is_empty());
    }

    #[test]
    fn test_onboard_empty_index() {
        let counter = TokenCounter::new();
        let index =
            CodebaseIndex::build_with_content(vec![], HashMap::new(), &counter, HashMap::new());
        let map = compute_onboarding_map(&index, None);
        assert_eq!(map.total_files, 0);
        assert!(map.phases.is_empty());
    }

    /// xml_escape must convert `<`, `&`, and `>` to their XML entities.
    #[test]
    fn test_xml_escape_converts_all_special_chars() {
        let input = "<a&b>";
        let escaped = xml_escape(input);
        assert_eq!(escaped, "&lt;a&amp;b&gt;");
    }

    /// The XML output from render_onboarding_xml must not contain raw `<` or `&`
    /// characters inside attribute values or text content supplied by the OnboardingMap.
    /// We build a map containing characters that require escaping and verify the output.
    #[test]
    fn test_render_onboarding_xml_escapes_special_chars_in_content() {
        let counter = TokenCounter::new();
        // Build a minimal index so compute_onboarding_map returns at least one phase.
        let files = vec![ScannedFile {
            relative_path: "src/a<b>&c.rs".to_string(),
            absolute_path: PathBuf::from("/tmp/src/a<b>&c.rs"),
            language: Some("rust".to_string()),
            size_bytes: 100,
        }];
        let mut content_map = HashMap::new();
        content_map.insert("src/a<b>&c.rs".to_string(), "fn foo() {}".to_string());
        let index = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);
        let map = compute_onboarding_map(&index, None);

        let xml = render_onboarding_xml(&map);

        // The XML skeleton tags themselves use < legitimately — so we check that
        // the raw unescaped sequences "&>" and "<a" from our crafted filename
        // do NOT appear verbatim in the XML attributes/text (they must be escaped).
        // We look at the file path attributes specifically.
        for line in xml.lines() {
            if line.contains("path=") {
                assert!(
                    !line.contains("a<b>"),
                    "unescaped '<' must not appear in path attribute: {line}"
                );
                assert!(
                    !line.contains("b>&"),
                    "unescaped '&' must not appear in path attribute: {line}"
                );
            }
        }
    }
}
