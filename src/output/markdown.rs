use super::{code_fence_for, OutputSections};

/// Emit one section. `fenced` wraps the body in a backtick run longer than any
/// run inside it; the other sections are cxpak-generated markdown whose
/// structure (`###` sub-headers, list items) must survive, so they are not
/// fenced — see the module note on why fencing is not the answer for those.
fn push_section(out: &mut String, title: &str, body: &str, fenced: bool) {
    if body.is_empty() {
        return;
    }
    // Every section body carries repo-derived text. #43: the XML renderer has
    // always filtered control characters and this one never did.
    let body = super::strip_control_chars(body);
    out.push_str("## ");
    out.push_str(title);
    out.push_str("\n\n");
    if fenced {
        let fence = code_fence_for(&body);
        out.push_str(&fence);
        out.push('\n');
        out.push_str(&body);
        out.push('\n');
        out.push_str(&fence);
        out.push_str("\n\n");
    } else {
        out.push_str(&body);
        out.push_str("\n\n");
    }
}

pub fn render(sections: &OutputSections) -> String {
    let mut out = String::new();
    push_section(&mut out, "Project Metadata", &sections.metadata, false);
    push_section(&mut out, "Directory Tree", &sections.directory_tree, true);
    push_section(
        &mut out,
        "Module / Component Map",
        &sections.module_map,
        false,
    );
    push_section(
        &mut out,
        "Dependency Graph",
        &sections.dependency_graph,
        false,
    );
    push_section(&mut out, "Key Files", &sections.key_files, false);
    push_section(
        &mut out,
        "Function / Type Signatures",
        &sections.signatures,
        false,
    );
    push_section(&mut out, "Git Context", &sections.git_context, false);
    out
}

pub fn render_single_section(title: &str, content: &str) -> String {
    format!("## {title}\n\n{}\n", super::strip_control_chars(content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_single_section() {
        let content = "### src/main.rs\n- pub Function: `main`\n";
        let output = render_single_section("Module / Component Map", content);
        assert!(output.starts_with("## Module / Component Map"));
        assert!(output.contains("pub Function"));
    }

    #[test]
    fn test_render_all_sections() {
        let sections = OutputSections {
            metadata: "name: test".to_string(),
            directory_tree: "src/".to_string(),
            module_map: "mod a".to_string(),
            dependency_graph: "a -> b".to_string(),
            key_files: "main.rs".to_string(),
            signatures: "fn main()".to_string(),
            git_context: "branch: main".to_string(),
        };
        let output = render(&sections);
        assert!(output.contains("## Project Metadata"));
        assert!(output.contains("## Directory Tree"));
        assert!(output.contains("## Module / Component Map"));
        assert!(output.contains("## Dependency Graph"));
        assert!(output.contains("## Key Files"));
        assert!(output.contains("## Function / Type Signatures"));
        assert!(output.contains("## Git Context"));
    }

    #[test]
    fn test_code_fence_escapes_triple_backtick_in_content() {
        // Content that contains a triple-backtick run.
        let content = "```\nsome code\n```";
        let fence = code_fence_for(content);
        // Fence must be longer than 3 backticks.
        assert!(
            fence.len() > 3,
            "fence should be longer than longest backtick run in content, got: {fence}"
        );
        assert!(
            fence.chars().all(|c| c == '`'),
            "fence must consist only of backticks"
        );
    }

    #[test]
    fn test_render_includes_sections() {
        let sections = OutputSections {
            metadata: "Language: Rust (100%)".into(),
            directory_tree: "src/\n  main.rs".into(),
            module_map: String::new(),
            dependency_graph: String::new(),
            key_files: String::new(),
            signatures: String::new(),
            git_context: String::new(),
        };
        let output = render(&sections);
        assert!(output.contains("## Project Metadata"));
        assert!(output.contains("Language: Rust"));
        assert!(output.contains("## Directory Tree"));
        assert!(!output.contains("## Module")); // empty = omitted
    }

    // ── #43: repo-derived text reaches the briefing sanitised ─────────────

    fn sections_with(git_context: &str) -> OutputSections {
        OutputSections {
            metadata: "name: test".to_string(),
            directory_tree: "src/".to_string(),
            module_map: "### src/a.rs\n- `a`\n".to_string(),
            dependency_graph: "a -> b".to_string(),
            key_files: "main.rs".to_string(),
            signatures: "fn main()".to_string(),
            git_context: git_context.to_string(),
        }
    }

    #[test]
    fn an_ansi_escape_in_repo_text_does_not_reach_markdown_output() {
        // A commit message is attacker-controllable by anyone who can land a
        // commit, and it lands in a briefing an agent reads and a terminal
        // renders.
        let out = render(&sections_with(
            "### Recent Commits\n\n- `abc` \u{1b}[31mred\u{1b}[0m — a (d)\n",
        ));
        assert!(
            !out.contains('\u{1b}'),
            "an ESC reached the rendered briefing: {out:?}"
        );
        assert!(
            out.contains("[31mred"),
            "only the ESC is removed; the surrounding text stays readable: {out:?}"
        );
    }

    #[test]
    fn a_carriage_return_and_del_do_not_reach_markdown_output() {
        let out = render(&sections_with("a\rb\u{7f}c"));
        assert!(
            !out.contains('\r'),
            "a lone CR overwrites what a reader saw"
        );
        assert!(!out.contains('\u{7f}'), "DEL is not content");
        assert!(
            out.contains("abc"),
            "the characters around them survive: {out:?}"
        );
    }

    #[test]
    fn ordinary_markdown_structure_survives_sanitisation() {
        // The control for the whole change: "strip control characters" is
        // trivially satisfied by stripping too much.
        let out = render(&sections_with("### Recent Commits\n\n- `abc` msg\n"));
        assert!(out.contains("## Module / Component Map"), "{out:?}");
        assert!(out.contains("### src/a.rs"), "sub-headers survive: {out:?}");
        assert!(out.contains("- `a`"), "list items survive: {out:?}");
        assert!(out.contains("### Recent Commits"), "{out:?}");
        assert!(out.contains("- `abc` msg"), "{out:?}");
        assert!(out.contains("## Directory Tree"), "{out:?}");
    }

    #[test]
    fn the_directory_tree_is_still_the_only_fenced_section() {
        // git_context is cxpak-generated markdown — `### Recent Commits`, list
        // items — so fencing it would destroy its structure rather than protect
        // it. The injection it is exposed to is fixed where repo strings are
        // interpolated, not here. This pins that decision.
        let out = render(&sections_with("### Recent Commits\n\n- `abc` msg\n"));
        let fences = out.matches("```").count();
        assert_eq!(
            fences, 2,
            "exactly one fenced section (open + close) — the directory tree: {out:?}"
        );
    }

    #[test]
    fn newlines_and_tabs_are_kept() {
        assert_eq!(super::super::strip_control_chars("a\nb\tc"), "a\nb\tc");
        assert_eq!(super::super::strip_control_chars("a\u{1b}b"), "ab");
        assert_eq!(super::super::strip_control_chars("a\u{0}b"), "ab");
    }

    /// `--section` is a second entrypoint into this renderer, and it does not
    /// go through `push_section`. A guard installed on only one of two doors
    /// leaves the output surface exactly as reachable as before.
    #[test]
    fn a_single_section_render_is_sanitised_too() {
        let out = render_single_section("Key Files", "src/main.rs \u{1b}[1;31mobey\u{1b}[0m\u{7f}");
        assert!(!out.contains('\u{1b}'), "{out:?}");
        assert!(!out.contains('\u{7f}'), "{out:?}");
        assert!(out.contains("[1;31mobey[0m"), "{out:?}");
    }
}
