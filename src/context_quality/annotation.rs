// Language-aware context annotations

use crate::context_quality::degradation::{DetailLevel, FileRole};
use crate::relevance::SignalResult;

/// Returns the line comment prefix and suffix for a given language.
///
/// The tuple is `(line_prefix, line_suffix)`. For block comments (HTML, CSS)
/// the suffix is non-empty; for line comments it is an empty string.
pub fn comment_syntax(language: &str) -> (&'static str, &'static str) {
    match language {
        "rust" | "javascript" | "typescript" | "java" | "go" | "c" | "cpp" | "csharp" | "swift"
        | "kotlin" | "scala" | "dart" | "zig" | "groovy" | "objc" | "proto" | "graphql" | "php"
        | "prisma" => ("// ", ""),
        "python" | "ruby" | "bash" | "perl" | "r" | "julia" | "elixir" | "yaml" | "toml"
        | "makefile" | "dockerfile" | "hcl" => ("# ", ""),
        "haskell" | "lua" | "sql" | "ocaml" | "ocaml_interface" => ("-- ", ""),
        "html" | "xml" | "svelte" | "markdown" => ("<!-- ", " -->"),
        "css" | "scss" => ("/* ", " */"),
        "matlab" => ("% ", ""),
        // JSON does not support comments; we still emit a `// ` header-only
        // annotation — callers that serialize strict JSON must strip it.
        // This matches the fallback for unknown languages.
        _ => ("// ", ""),
    }
}

/// All context required to produce a cxpak annotation header for one file.
pub struct AnnotationContext {
    pub path: String,
    pub language: String,
    pub score: f64,
    pub role: FileRole,
    /// For dependency files, the path of the file that pulled this one in.
    pub parent: Option<String>,
    pub signals: Vec<SignalResult>,
    pub detail_level: DetailLevel,
    pub tokens: usize,
}

/// Render a multi-line annotation header for a file.
///
/// Line 1 — `[cxpak]` marker with the file path.
/// Line 2 — relevance score, role, and (for dependencies) the parent path.
/// Line 3 — signal breakdown; omitted at `Documented` and coarser levels, and
///           omitted when the signals list is empty at finer levels.
/// Line 4 — detail level name and token count.
pub fn annotate_file(ctx: &AnnotationContext) -> String {
    let (pre, suf) = comment_syntax(&ctx.language);

    let role_str = match ctx.role {
        FileRole::Selected => "selected",
        FileRole::Dependency => "dependency",
    };

    let level_name = match ctx.detail_level {
        DetailLevel::Full => "full",
        DetailLevel::Trimmed => "trimmed",
        DetailLevel::Documented => "documented",
        DetailLevel::Signature => "signature",
        DetailLevel::Stub => "stub",
    };

    // Line 1: path marker
    let line1 = format!(
        "{pre}[cxpak] {path}{suf}",
        pre = pre,
        path = ctx.path,
        suf = suf
    );

    // Line 2: score + role + optional parent
    let line2 = if let Some(parent) = &ctx.parent {
        format!(
            "{pre}score: {score:.4} | role: {role} | parent: {parent}{suf}",
            pre = pre,
            score = ctx.score,
            role = role_str,
            parent = parent,
            suf = suf,
        )
    } else {
        format!(
            "{pre}score: {score:.4} | role: {role}{suf}",
            pre = pre,
            score = ctx.score,
            role = role_str,
            suf = suf,
        )
    };

    // Line 3: signal breakdown — only at Full (0) and Trimmed (1)
    let show_signals =
        ctx.detail_level == DetailLevel::Full || ctx.detail_level == DetailLevel::Trimmed;

    let line3 = if show_signals && !ctx.signals.is_empty() {
        let sig_parts: Vec<String> = ctx
            .signals
            .iter()
            .map(|s| format!("{}={:.2}", s.name, s.score))
            .collect();
        Some(format!(
            "{pre}signals: {signals}{suf}",
            pre = pre,
            signals = sig_parts.join(", "),
            suf = suf,
        ))
    } else {
        None
    };

    // Line 4: detail level + token count
    let line4 = format!(
        "{pre}detail_level: {level} ({tokens} tokens){suf}",
        pre = pre,
        level = level_name,
        tokens = ctx.tokens,
        suf = suf,
    );

    let mut parts = vec![line1, line2];
    if let Some(l3) = line3 {
        parts.push(l3);
    }
    parts.push(line4);
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------------
    // comment_syntax
    // ---------------------------------------------------------------------------

    #[test]
    fn comment_syntax_c_style_languages() {
        for lang in &[
            "rust",
            "javascript",
            "typescript",
            "java",
            "go",
            "c",
            "cpp",
            "csharp",
            "swift",
            "kotlin",
            "scala",
            "dart",
            "zig",
            "groovy",
            "objc",
            "proto",
            "graphql",
        ] {
            let (pre, suf) = comment_syntax(lang);
            assert_eq!(pre, "// ", "wrong prefix for {lang}");
            assert_eq!(suf, "", "wrong suffix for {lang}");
        }
    }

    #[test]
    fn comment_syntax_hash_style_languages() {
        for lang in &[
            "python",
            "ruby",
            "bash",
            "perl",
            "r",
            "julia",
            "elixir",
            "yaml",
            "toml",
            "makefile",
            "dockerfile",
        ] {
            let (pre, suf) = comment_syntax(lang);
            assert_eq!(pre, "# ", "wrong prefix for {lang}");
            assert_eq!(suf, "", "wrong suffix for {lang}");
        }
    }

    #[test]
    fn comment_syntax_double_dash_languages() {
        for lang in &["haskell", "lua", "sql", "ocaml", "ocaml_interface"] {
            let (pre, suf) = comment_syntax(lang);
            assert_eq!(pre, "-- ", "wrong prefix for {lang}");
            assert_eq!(suf, "", "wrong suffix for {lang}");
        }
    }

    #[test]
    fn comment_syntax_html_block_languages() {
        for lang in &["html", "xml", "svelte", "markdown"] {
            let (pre, suf) = comment_syntax(lang);
            assert_eq!(pre, "<!-- ", "wrong prefix for {lang}");
            assert_eq!(suf, " -->", "wrong suffix for {lang}");
        }
    }

    #[test]
    fn comment_syntax_css_block_languages() {
        for lang in &["css", "scss"] {
            let (pre, suf) = comment_syntax(lang);
            assert_eq!(pre, "/* ", "wrong prefix for {lang}");
            assert_eq!(suf, " */", "wrong suffix for {lang}");
        }
    }

    #[test]
    fn comment_syntax_matlab() {
        let (pre, suf) = comment_syntax("matlab");
        assert_eq!(pre, "% ");
        assert_eq!(suf, "");
    }

    #[test]
    fn comment_syntax_unknown_defaults_to_c_style() {
        let (pre, suf) = comment_syntax("cobol");
        assert_eq!(pre, "// ");
        assert_eq!(suf, "");
    }

    // ---------------------------------------------------------------------------
    // annotate_file helpers
    // ---------------------------------------------------------------------------

    fn make_signal(name: &'static str, score: f64) -> SignalResult {
        SignalResult {
            name,
            score,
            detail: format!("detail for {name}"),
        }
    }

    fn make_ctx_selected_full() -> AnnotationContext {
        AnnotationContext {
            path: "src/main.rs".to_string(),
            language: "rust".to_string(),
            score: 0.8765,
            role: FileRole::Selected,
            parent: None,
            signals: vec![
                make_signal("path_similarity", 0.75),
                make_signal("symbol_match", 0.50),
            ],
            detail_level: DetailLevel::Full,
            tokens: 320,
        }
    }

    // ---------------------------------------------------------------------------
    // annotate_file — line structure
    // ---------------------------------------------------------------------------

    #[test]
    fn annotate_selected_full_has_four_lines() {
        let ctx = make_ctx_selected_full();
        let output = annotate_file(&ctx);
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 4, "expected 4 lines, got:\n{output}");
    }

    #[test]
    fn annotate_line1_contains_cxpak_marker_and_path() {
        let ctx = make_ctx_selected_full();
        let output = annotate_file(&ctx);
        let line1 = output.lines().next().unwrap();
        assert!(line1.contains("[cxpak]"), "line 1 missing [cxpak]: {line1}");
        assert!(
            line1.contains("src/main.rs"),
            "line 1 missing path: {line1}"
        );
        assert!(line1.starts_with("// "), "line 1 wrong prefix: {line1}");
    }

    #[test]
    fn annotate_line2_score_four_decimal_places() {
        let ctx = make_ctx_selected_full();
        let output = annotate_file(&ctx);
        let line2 = output.lines().nth(1).unwrap();
        // 0.8765 should appear with exactly 4 decimal places
        assert!(line2.contains("0.8765"), "score missing in line 2: {line2}");
        assert!(
            line2.contains("selected"),
            "role missing in line 2: {line2}"
        );
    }

    #[test]
    fn annotate_line2_no_parent_for_selected() {
        let ctx = make_ctx_selected_full();
        let output = annotate_file(&ctx);
        let line2 = output.lines().nth(1).unwrap();
        assert!(
            !line2.contains("parent:"),
            "line 2 should not contain parent for selected file: {line2}"
        );
    }

    #[test]
    fn annotate_line3_signals_at_full() {
        let ctx = make_ctx_selected_full();
        let output = annotate_file(&ctx);
        let line3 = output.lines().nth(2).unwrap();
        assert!(
            line3.contains("signals:"),
            "line 3 missing 'signals:': {line3}"
        );
        assert!(
            line3.contains("path_similarity=0.75"),
            "missing path_similarity signal: {line3}"
        );
        assert!(
            line3.contains("symbol_match=0.50"),
            "missing symbol_match signal: {line3}"
        );
    }

    #[test]
    fn annotate_line4_detail_level_and_tokens() {
        let ctx = make_ctx_selected_full();
        let output = annotate_file(&ctx);
        let line4 = output.lines().nth(3).unwrap();
        assert!(
            line4.contains("detail_level: full"),
            "line 4 missing level: {line4}"
        );
        assert!(
            line4.contains("320 tokens"),
            "line 4 missing token count: {line4}"
        );
    }

    // ---------------------------------------------------------------------------
    // annotate_file — dependency with parent
    // ---------------------------------------------------------------------------

    #[test]
    fn annotate_dependency_shows_parent_field() {
        let ctx = AnnotationContext {
            path: "src/util.rs".to_string(),
            language: "rust".to_string(),
            score: 0.4321,
            role: FileRole::Dependency,
            parent: Some("src/main.rs".to_string()),
            signals: vec![make_signal("term_frequency", 0.30)],
            detail_level: DetailLevel::Full,
            tokens: 100,
        };
        let output = annotate_file(&ctx);
        let line2 = output.lines().nth(1).unwrap();
        assert!(
            line2.contains("dependency"),
            "line 2 should say 'dependency': {line2}"
        );
        assert!(
            line2.contains("parent: src/main.rs"),
            "line 2 should contain parent path: {line2}"
        );
    }

    #[test]
    fn annotate_dependency_no_parent_omits_parent_field() {
        let ctx = AnnotationContext {
            path: "src/util.rs".to_string(),
            language: "rust".to_string(),
            score: 0.4321,
            role: FileRole::Dependency,
            parent: None,
            signals: vec![],
            detail_level: DetailLevel::Trimmed,
            tokens: 50,
        };
        let output = annotate_file(&ctx);
        let line2 = output.lines().nth(1).unwrap();
        assert!(
            !line2.contains("parent:"),
            "line 2 should not contain parent field when None: {line2}"
        );
    }

    // ---------------------------------------------------------------------------
    // annotate_file — signal line omission rules
    // ---------------------------------------------------------------------------

    #[test]
    fn annotate_signal_line_omitted_at_documented() {
        let ctx = AnnotationContext {
            path: "src/lib.rs".to_string(),
            language: "rust".to_string(),
            score: 0.6,
            role: FileRole::Selected,
            parent: None,
            signals: vec![make_signal("path_similarity", 0.8)],
            detail_level: DetailLevel::Documented,
            tokens: 200,
        };
        let output = annotate_file(&ctx);
        let lines: Vec<&str> = output.lines().collect();
        // Should be only 3 lines (no signals line)
        assert_eq!(
            lines.len(),
            3,
            "signals line should be omitted at Documented: {output}"
        );
        assert!(
            !output.contains("signals:"),
            "should not contain signals at Documented: {output}"
        );
    }

    #[test]
    fn annotate_signal_line_omitted_at_signature() {
        let ctx = AnnotationContext {
            path: "src/lib.rs".to_string(),
            language: "rust".to_string(),
            score: 0.5,
            role: FileRole::Dependency,
            parent: None,
            signals: vec![make_signal("symbol_match", 0.9)],
            detail_level: DetailLevel::Signature,
            tokens: 80,
        };
        let output = annotate_file(&ctx);
        assert!(
            !output.contains("signals:"),
            "should not contain signals at Signature: {output}"
        );
    }

    #[test]
    fn annotate_signal_line_omitted_at_stub() {
        let ctx = AnnotationContext {
            path: "src/lib.rs".to_string(),
            language: "rust".to_string(),
            score: 0.3,
            role: FileRole::Dependency,
            parent: None,
            signals: vec![make_signal("import_proximity", 0.5)],
            detail_level: DetailLevel::Stub,
            tokens: 10,
        };
        let output = annotate_file(&ctx);
        assert!(
            !output.contains("signals:"),
            "should not contain signals at Stub: {output}"
        );
    }

    #[test]
    fn annotate_signal_line_present_at_trimmed() {
        let ctx = AnnotationContext {
            path: "src/lib.rs".to_string(),
            language: "rust".to_string(),
            score: 0.7,
            role: FileRole::Selected,
            parent: None,
            signals: vec![make_signal("term_frequency", 0.65)],
            detail_level: DetailLevel::Trimmed,
            tokens: 150,
        };
        let output = annotate_file(&ctx);
        assert!(
            output.contains("signals:"),
            "signals line should appear at Trimmed: {output}"
        );
        assert!(
            output.contains("term_frequency=0.65"),
            "signal value missing: {output}"
        );
    }

    #[test]
    fn annotate_empty_signals_omitted_even_at_full() {
        let ctx = AnnotationContext {
            path: "src/lib.rs".to_string(),
            language: "rust".to_string(),
            score: 0.5,
            role: FileRole::Selected,
            parent: None,
            signals: vec![],
            detail_level: DetailLevel::Full,
            tokens: 100,
        };
        let output = annotate_file(&ctx);
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(
            lines.len(),
            3,
            "empty signals should produce 3 lines at Full: {output}"
        );
        assert!(
            !output.contains("signals:"),
            "should not emit signals: line when signals empty: {output}"
        );
    }

    // ---------------------------------------------------------------------------
    // annotate_file — block comment wrapping (HTML)
    // ---------------------------------------------------------------------------

    #[test]
    fn annotate_html_uses_block_comment_syntax() {
        let ctx = AnnotationContext {
            path: "index.html".to_string(),
            language: "html".to_string(),
            score: 0.5,
            role: FileRole::Selected,
            parent: None,
            signals: vec![make_signal("path_similarity", 0.5)],
            detail_level: DetailLevel::Full,
            tokens: 60,
        };
        let output = annotate_file(&ctx);
        for line in output.lines() {
            assert!(
                line.starts_with("<!-- "),
                "HTML line should start with '<!-- ': {line}"
            );
            assert!(
                line.ends_with(" -->"),
                "HTML line should end with ' -->': {line}"
            );
        }
    }

    // ---------------------------------------------------------------------------
    // annotate_file — CSS block comment wrapping
    // ---------------------------------------------------------------------------

    #[test]
    fn annotate_css_uses_block_comment_syntax() {
        let ctx = AnnotationContext {
            path: "styles.css".to_string(),
            language: "css".to_string(),
            score: 0.4,
            role: FileRole::Dependency,
            parent: Some("index.html".to_string()),
            signals: vec![],
            detail_level: DetailLevel::Signature,
            tokens: 40,
        };
        let output = annotate_file(&ctx);
        for line in output.lines() {
            assert!(
                line.starts_with("/* "),
                "CSS line should start with '/* ': {line}"
            );
            assert!(
                line.ends_with(" */"),
                "CSS line should end with ' */': {line}"
            );
        }
    }

    // ---------------------------------------------------------------------------
    // annotate_file — level names in line 4
    // ---------------------------------------------------------------------------

    #[test]
    fn annotate_level_names_correct() {
        let levels = [
            (DetailLevel::Full, "full"),
            (DetailLevel::Trimmed, "trimmed"),
            (DetailLevel::Documented, "documented"),
            (DetailLevel::Signature, "signature"),
            (DetailLevel::Stub, "stub"),
        ];
        for (level, expected_name) in levels {
            let ctx = AnnotationContext {
                path: "a.rs".to_string(),
                language: "rust".to_string(),
                score: 0.5,
                role: FileRole::Selected,
                parent: None,
                signals: vec![],
                detail_level: level,
                tokens: 10,
            };
            let output = annotate_file(&ctx);
            let last_line = output.lines().last().unwrap();
            assert!(
                last_line.contains(expected_name),
                "expected level name '{expected_name}' in last line: {last_line}"
            );
        }
    }

    // ---------------------------------------------------------------------------
    // annotate_file — language family smoke tests
    // ---------------------------------------------------------------------------

    #[test]
    fn annotate_python_uses_hash_comment() {
        let ctx = AnnotationContext {
            path: "main.py".to_string(),
            language: "python".to_string(),
            score: 0.9,
            role: FileRole::Selected,
            parent: None,
            signals: vec![make_signal("path_similarity", 0.9)],
            detail_level: DetailLevel::Full,
            tokens: 200,
        };
        let output = annotate_file(&ctx);
        for line in output.lines() {
            assert!(
                line.starts_with("# "),
                "Python line should start with '# ': {line}"
            );
        }
    }

    #[test]
    fn annotate_lua_uses_double_dash_comment() {
        let ctx = AnnotationContext {
            path: "init.lua".to_string(),
            language: "lua".to_string(),
            score: 0.6,
            role: FileRole::Dependency,
            parent: None,
            signals: vec![],
            detail_level: DetailLevel::Documented,
            tokens: 75,
        };
        let output = annotate_file(&ctx);
        for line in output.lines() {
            assert!(
                line.starts_with("-- "),
                "Lua line should start with '-- ': {line}"
            );
        }
    }

    #[test]
    fn annotate_matlab_uses_percent_comment() {
        let ctx = AnnotationContext {
            path: "script.m".to_string(),
            language: "matlab".to_string(),
            score: 0.4,
            role: FileRole::Selected,
            parent: None,
            signals: vec![],
            detail_level: DetailLevel::Stub,
            tokens: 30,
        };
        let output = annotate_file(&ctx);
        for line in output.lines() {
            assert!(
                line.starts_with("% "),
                "MATLAB line should start with '% ': {line}"
            );
        }
    }
}
