use super::OutputSections;

pub fn render(sections: &OutputSections) -> String {
    let mut out = String::from("<cxpak>\n");
    emit_section(&mut out, "metadata", &sections.metadata);
    emit_section(&mut out, "directory-tree", &sections.directory_tree);
    emit_section(&mut out, "module-map", &sections.module_map);
    emit_section(&mut out, "dependency-graph", &sections.dependency_graph);
    emit_section(&mut out, "key-files", &sections.key_files);
    emit_section(&mut out, "signatures", &sections.signatures);
    emit_section(&mut out, "git-context", &sections.git_context);
    out.push_str("</cxpak>\n");
    out
}

fn emit_section(out: &mut String, tag: &str, content: &str) {
    if !content.is_empty() {
        out.push_str(&format!("  <{tag}>\n"));
        for line in content.lines() {
            out.push_str(&format!("    {}\n", escape_xml(line)));
        }
        out.push_str(&format!("  </{tag}>\n"));
    }
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
