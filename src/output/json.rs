use super::OutputSections;
use serde::Serialize;

#[derive(Serialize)]
struct JsonOutput {
    #[serde(skip_serializing_if = "String::is_empty")]
    metadata: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    directory_tree: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    module_map: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    dependency_graph: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    key_files: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    signatures: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    git_context: String,
}

pub fn render_single_section(title: &str, content: &str) -> String {
    let key = title.to_lowercase().replace([' ', '/'], "_");
    let mut map = serde_json::Map::new();
    map.insert(key, serde_json::Value::String(content.to_string()));
    serde_json::to_string_pretty(&serde_json::Value::Object(map))
        .expect("serde_json::to_string_pretty of a Value is infallible")
}

pub fn render(sections: &OutputSections) -> String {
    let output = JsonOutput {
        metadata: sections.metadata.clone(),
        directory_tree: sections.directory_tree.clone(),
        module_map: sections.module_map.clone(),
        dependency_graph: sections.dependency_graph.clone(),
        key_files: sections.key_files.clone(),
        signatures: sections.signatures.clone(),
        git_context: sections.git_context.clone(),
    };
    serde_json::to_string_pretty(&output)
        .expect("serde_json::to_string_pretty of a Value is infallible")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sections() -> OutputSections {
        OutputSections {
            metadata: "name: test".to_string(),
            directory_tree: "src/".to_string(),
            module_map: "mod a".to_string(),
            dependency_graph: "a -> b".to_string(),
            key_files: "main.rs".to_string(),
            signatures: "fn main()".to_string(),
            git_context: "branch: main".to_string(),
        }
    }

    #[test]
    fn test_render_json() {
        let sections = make_sections();
        let output = render(&sections);
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["metadata"], "name: test");
        assert_eq!(parsed["directory_tree"], "src/");
    }

    #[test]
    fn test_render_single_section_json() {
        let output = render_single_section("Key Files", "main.rs");
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["key_files"], "main.rs");
    }

    #[test]
    fn test_render_json_empty_sections_skipped() {
        let sections = OutputSections {
            metadata: "test".to_string(),
            directory_tree: String::new(),
            module_map: String::new(),
            dependency_graph: String::new(),
            key_files: String::new(),
            signatures: String::new(),
            git_context: String::new(),
        };
        let output = render(&sections);
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert!(parsed.get("directory_tree").is_none());
        assert_eq!(parsed["metadata"], "test");
    }
}
