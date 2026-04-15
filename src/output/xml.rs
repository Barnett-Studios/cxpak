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

pub fn render_single_section(title: &str, content: &str) -> String {
    let raw: String = title
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    let raw = raw.trim_matches('-').to_string();
    let raw = if raw.is_empty() {
        "section".to_string()
    } else {
        raw
    };
    // XML NCName: first char must be letter or '_'; prefix '_' if it's a digit.
    let tag = if raw
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
    {
        format!("_{raw}")
    } else {
        raw
    };
    let mut out = String::from("<cxpak>\n");
    emit_section(&mut out, &tag, content);
    out.push_str("</cxpak>\n");
    out
}

fn emit_section(out: &mut String, tag: &str, content: &str) {
    if !content.is_empty() {
        out.push_str(&format!("  <{tag}>\n"));
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("<!-- ") && trimmed.ends_with(" -->") {
                // Omission pointer — emit as XML element instead of escaped comment
                let inner = &trimmed[5..trimmed.len() - 4];
                out.push_str(&format!(
                    "    <detail-ref>{}</detail-ref>\n",
                    escape_xml(inner)
                ));
            } else {
                out.push_str(&format!("    {}\n", escape_xml(line)));
            }
        }
        out.push_str(&format!("  </{tag}>\n"));
    }
}

fn escape_xml(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .filter(|&c| !matches!(c as u32, 0x0..=0x8 | 0xB..=0xC | 0xE..=0x1F))
        .collect();
    cleaned
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
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
    fn test_render_xml() {
        let sections = make_sections();
        let output = render(&sections);
        assert!(output.starts_with("<cxpak>"));
        assert!(output.contains("<metadata>"));
        assert!(output.contains("name: test"));
        assert!(output.ends_with("</cxpak>\n"));
    }

    #[test]
    fn test_render_single_section_xml() {
        let output = render_single_section("Key Files", "main.rs");
        assert!(output.contains("<key-files>"));
        assert!(output.contains("main.rs"));
        assert!(output.contains("</key-files>"));
    }

    #[test]
    fn test_escape_xml_special_chars() {
        let escaped = escape_xml("a & b < c > d \"e\"");
        assert_eq!(escaped, "a &amp; b &lt; c &gt; d &quot;e&quot;");
    }

    #[test]
    fn test_escape_xml_handles_apostrophe() {
        assert_eq!(escape_xml("it's"), "it&apos;s");
    }

    #[test]
    fn test_render_single_section_safe_tag_from_injection() {
        let output = render_single_section("<inject>", "content");
        // The injected angle brackets should be mapped to dashes, not kept as-is.
        assert!(
            !output.contains("<<inject>>"),
            "raw injection chars must not appear: {output}"
        );
        assert!(
            output.contains("<cxpak>"),
            "wrapper must still be present: {output}"
        );
    }

    #[test]
    fn test_escape_xml_strips_forbidden_control_chars() {
        let input = "\x00\x01\x08<>";
        let result = escape_xml(input);
        assert!(!result.contains('\x00'), "null must be stripped: {result}");
        assert!(!result.contains('\x01'), "SOH must be stripped: {result}");
        assert!(result.contains("&lt;"), "< must be escaped: {result}");
        assert!(result.contains("&gt;"), "> must be escaped: {result}");
    }

    #[test]
    fn test_xml_empty_sections_skipped() {
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
        assert!(output.contains("<metadata>"));
        assert!(!output.contains("<directory-tree>"));
    }

    #[test]
    fn test_xml_omission_pointer() {
        let sections = OutputSections {
            metadata: "<!-- signatures full content: .cxpak/sigs.md (~5k tokens) -->".to_string(),
            directory_tree: String::new(),
            module_map: String::new(),
            dependency_graph: String::new(),
            key_files: String::new(),
            signatures: String::new(),
            git_context: String::new(),
        };
        let output = render(&sections);
        assert!(output.contains("<detail-ref>"));
        assert!(!output.contains("<!--"));
    }
}
