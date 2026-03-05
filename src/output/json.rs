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
    serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".into())
}
