pub mod markdown;
pub mod xml;
pub mod json;

use crate::cli::OutputFormat;

#[derive(Debug, Clone)]
pub struct OutputSections {
    pub metadata: String,
    pub directory_tree: String,
    pub module_map: String,
    pub dependency_graph: String,
    pub key_files: String,
    pub signatures: String,
    pub git_context: String,
}

pub fn render(sections: &OutputSections, format: &OutputFormat) -> String {
    match format {
        OutputFormat::Markdown => markdown::render(sections),
        OutputFormat::Xml => xml::render(sections),
        OutputFormat::Json => json::render(sections),
    }
}
