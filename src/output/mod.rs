pub mod json;
pub mod markdown;
pub mod xml;

use crate::cli::OutputFormat;

/// Remove control characters that carry no meaning in a briefing and are a
/// rendering hazard in one.
///
/// Keeps `\n` and `\t`; drops the rest of C0 — ESC above all — plus DEL and a
/// lone `\r`. A crafted commit message, branch name or filename otherwise
/// renders ANSI escape sequences straight into a terminal or into the context an
/// agent reads (#43).
///
/// Stripping rather than escaping is deliberate, and it is not a new decision:
/// `output::xml::escape_xml` has filtered `0x0..=0x8 | 0xB..=0xC | 0xE..=0x1F`
/// since it was written. The XML renderer was hardened and the markdown one was
/// not, which is the whole asymmetry #43 describes. This is the one place that
/// policy now lives, so the two renderers cannot drift apart again.
///
/// `\r` and DEL are dropped here where `escape_xml` used to keep them: a lone
/// `\r` overwrites the line a reader has already seen, which is the same class
/// of harm as ESC. A `\r\n` line ending loses its `\r` and keeps its `\n`.
/// Returns a code fence string (backtick run) that is guaranteed to be longer
/// than the longest backtick run that appears at the start of any line in
/// `content`.  This prevents backtick injection from user-controlled content.
pub(crate) fn code_fence_for(content: &str) -> String {
    let max_run = content
        .lines()
        // CommonMark lets a closing fence be indented up to three spaces, so a
        // run at column 1-3 closes the block just as a run at column 0 does.
        // Counting only column 0 (#43) left `   ``` ` able to close ours.
        .map(|l| {
            l.strip_prefix("   ")
                .or_else(|| l.strip_prefix("  "))
                .or_else(|| l.strip_prefix(' '))
                .unwrap_or(l)
                .chars()
                .take_while(|&c| c == '`')
                .count()
        })
        .max()
        .unwrap_or(0);
    "`".repeat(max_run.max(2) + 1)
}

/// One fenced block of repo-derived text, with a fence `content` cannot close.
///
/// #43: every caller that packed a file body, a symbol body or a diff hunk into
/// markdown wrote its own `\`\`\`` by hand, so a source file containing a
/// three-backtick run ended the block and had the rest of itself read as
/// markdown. `info` is the language hint (`""` for none) and is trusted — it is
/// always a literal at the call site, never repo text.
pub(crate) fn fenced(content: &str, info: &str) -> String {
    let fence = code_fence_for(content);
    format!("{fence}{info}\n{content}\n{fence}\n\n")
}

pub(crate) fn strip_control_chars(s: &str) -> String {
    s.chars()
        .filter(|&c| c == '\n' || c == '\t' || !(c.is_control()))
        .collect()
}

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

pub fn render_single_section(title: &str, content: &str, format: &OutputFormat) -> String {
    match format {
        OutputFormat::Markdown => markdown::render_single_section(title, content),
        OutputFormat::Xml => xml::render_single_section(title, content),
        OutputFormat::Json => json::render_single_section(title, content),
    }
}

pub fn render(sections: &OutputSections, format: &OutputFormat) -> String {
    match format {
        OutputFormat::Markdown => markdown::render(sections),
        OutputFormat::Xml => xml::render(sections),
        OutputFormat::Json => json::render(sections),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sections() -> OutputSections {
        OutputSections {
            metadata: "name: test".to_string(),
            directory_tree: "src/".to_string(),
            module_map: String::new(),
            dependency_graph: String::new(),
            key_files: String::new(),
            signatures: String::new(),
            git_context: String::new(),
        }
    }

    #[test]
    fn test_render_dispatches_markdown() {
        let output = render(&make_sections(), &OutputFormat::Markdown);
        assert!(output.contains("# ") || output.contains("##"));
    }

    #[test]
    fn test_render_dispatches_xml() {
        let output = render(&make_sections(), &OutputFormat::Xml);
        assert!(output.contains("<cxpak>"));
    }

    #[test]
    fn test_render_dispatches_json() {
        let output = render(&make_sections(), &OutputFormat::Json);
        assert!(output.contains("\"metadata\""));
    }

    #[test]
    fn test_render_single_section_all_formats() {
        let md = render_single_section("Test", "content", &OutputFormat::Markdown);
        assert!(md.contains("content"));

        let xml = render_single_section("Test", "content", &OutputFormat::Xml);
        assert!(xml.contains("<cxpak>"));

        let json = render_single_section("Test", "content", &OutputFormat::Json);
        assert!(json.contains("content"));
    }
}

#[cfg(test)]
mod fence_tests {
    use super::{code_fence_for, fenced};

    /// The control: ordinary content keeps the fence every reader expects.
    /// "Emit a fence longer than the content" is trivially satisfied by always
    /// emitting twenty backticks, so this is the assertion that stops that.
    #[test]
    fn ordinary_content_gets_an_ordinary_three_backtick_fence() {
        let out = fenced("fn main() {}", "");
        assert_eq!(out, "```\nfn main() {}\n```\n\n", "{out:?}");
        assert_eq!(code_fence_for("fn main() {}"), "```");
    }

    #[test]
    fn the_info_string_rides_the_opening_fence_only() {
        let out = fenced("-a\n+b", "diff");
        assert!(out.starts_with("```diff\n"), "{out:?}");
        assert!(out.ends_with("\n```\n\n"), "{out:?}");
    }

    /// #43: content that already contains a fence must not be able to close the
    /// block. A README with a ```rust example is the ordinary case, not an attack.
    #[test]
    fn content_carrying_a_fence_cannot_close_the_block() {
        let readme = "# Demo\n\n```rust\nfn main() {}\n```\n\nDone.";
        let out = fenced(readme, "");
        let fence = code_fence_for(readme);
        assert_eq!(
            fence, "````",
            "a 3-run inside needs a 4-run around: {fence:?}"
        );
        assert!(out.starts_with("````\n"), "{out:?}");
        assert!(out.ends_with("\n````\n\n"), "{out:?}");
        // The block ends exactly once, at the end — not at the README's own fence.
        let closes: Vec<_> = out.lines().filter(|l| *l == fence).collect();
        assert_eq!(closes.len(), 2, "opened and closed once: {out:?}");
    }

    /// Only a run at the START of a line closes a fence, and the guard must grow
    /// past the longest one — not merely past three.
    #[test]
    fn the_fence_outgrows_the_longest_run_at_a_line_start() {
        assert_eq!(code_fence_for("a\n`````x\nb"), "``````");
        // Mid-line backticks close nothing, so they must not inflate the fence.
        assert_eq!(code_fence_for("let s = \"`````\";"), "```");
    }
    /// CommonMark allows a closing fence to be indented up to three spaces.
    /// Counting only column 0 left `   ``` ` able to close our block; counting
    /// any indentation would inflate the fence for ordinary indented code.
    #[test]
    fn an_indented_fence_still_counts_up_to_three_spaces() {
        assert_eq!(
            code_fence_for("a\n   ```\nb"),
            "````",
            "3 spaces still closes"
        );
        assert_eq!(code_fence_for("a\n  ```\nb"), "````");
        assert_eq!(code_fence_for("a\n ```\nb"), "````");
        // Four spaces is an indented code block, not a fence — it closes nothing.
        assert_eq!(code_fence_for("a\n    ```\nb"), "```");
        // And a doc comment is not a fence either.
        assert_eq!(code_fence_for("/// ```\n/// x\n/// ```"), "```");
    }
}
