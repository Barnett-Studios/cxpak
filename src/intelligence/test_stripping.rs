// Strip language-specific test blocks from source content before running
// security / route / secret-pattern detectors.
//
// The goal is to prevent detectors from flagging literal strings used as
// test fixtures inside test blocks (e.g., `AKIA…` AWS key patterns embedded
// in security.rs tests, SQL injection examples in test helpers, etc.).
//
// Line numbers are preserved by replacing stripped content with an equal
// number of newlines, so that any line-number calculation still points to
// the correct location in the original file.

// ---------------------------------------------------------------------------
// Rust: #[cfg(test)] mod tests { ... }
// ---------------------------------------------------------------------------

/// Remove `#[cfg(test)] mod <name> { ... }` blocks from Rust source content.
///
/// The implementation is a best-effort brace-counter.  It assumes
/// well-formed Rust input (matching braces, no unbalanced string literals
/// containing raw braces, etc.).  The stripped region is replaced with
/// blank lines so that byte offsets remain approximately correct for
/// error-reporting purposes.
pub fn strip_rust_test_blocks(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let bytes = content.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Look for `#[cfg(test)]`
        if bytes[i..].starts_with(b"#[cfg(test)]") {
            let marker_end = i + b"#[cfg(test)]".len();
            let mut j = marker_end;

            // Skip whitespace (including newlines) between the attribute and `mod`
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }

            // Expect `mod ` (with optional `pub ` prefix is handled below)
            if bytes[j..].starts_with(b"mod ") || bytes[j..].starts_with(b"pub mod ") {
                // Scan forward to the opening `{`
                let mut k = j;
                while k < bytes.len() && bytes[k] != b'{' {
                    k += 1;
                }

                if k < bytes.len() {
                    // Brace-count from `{` to the matching `}`
                    let mut depth = 0i32;
                    let mut end = k;
                    while end < bytes.len() {
                        match bytes[end] {
                            b'{' => depth += 1,
                            b'}' => {
                                depth -= 1;
                                if depth == 0 {
                                    end += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        end += 1;
                    }

                    // Replace the entire block (`#[cfg(test)]` … `}`) with blank
                    // lines so that line numbers for code AFTER the block remain
                    // correct.
                    let skipped = &content[i..end];
                    let newline_count = skipped.chars().filter(|&c| c == '\n').count();
                    for _ in 0..newline_count {
                        out.push('\n');
                    }
                    i = end;
                    continue;
                }
            }
        }

        out.push(bytes[i] as char);
        i += 1;
    }

    out
}

// ---------------------------------------------------------------------------
// Python: class Test* / def test_* blocks (indentation-based)
// ---------------------------------------------------------------------------

/// Remove `class Test*` and top-level `def test_*` blocks from Python source.
///
/// Python's indentation-based scoping means we keep skipping lines until we
/// see a line that is not blank and is indented at most as deeply as the
/// class/def declaration itself.
pub fn strip_python_test_blocks(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        if trimmed.starts_with("class Test") || trimmed.starts_with("def test_") {
            let indent = line.len() - trimmed.len();

            // Replace the header line with a blank placeholder
            out.push('\n');
            i += 1;

            // Skip all lines that are blank or indented strictly deeper than `indent`
            while i < lines.len() {
                let l = lines[i];
                let is_blank = l.trim().is_empty();
                let child_indent = l.len() - l.trim_start().len();
                let is_child = is_blank || child_indent > indent;
                if !is_child {
                    break;
                }
                out.push('\n');
                i += 1;
            }
            continue;
        }

        out.push_str(line);
        out.push('\n');
        i += 1;
    }

    out
}

// ---------------------------------------------------------------------------
// Rust comment stripping (// line comments and /* ... */ block comments)
// ---------------------------------------------------------------------------

/// Remove Rust-style `//` line comments and `/* ... */` block comments from
/// source content. Preserves line count by emitting newlines for stripped
/// regions.
///
/// Purpose: detector regexes (security, routes, secrets) should not match on
/// literal example patterns embedded in comments describing what they match.
///
/// Does NOT distinguish comment-in-string from actual comment — e.g., the
/// literal string `"// not a comment"` will be treated as starting a comment.
/// This is acceptable for detector input where we'd rather miss a match than
/// flag a doc example.
pub fn strip_rust_comments(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let bytes = content.as_bytes();
    let mut i = 0;
    // String state: 0 = not in string, N = in raw string opened with N hashes
    // (e.g., r#"..."# = 1, r##"..."## = 2). For regular strings, use raw_hashes = 0
    // and track via in_string flag.
    let mut in_string = false;
    let mut raw_hashes: usize = 0; // 0 = not raw; >0 = raw with that many hashes
    let mut in_char = false;
    let mut escape = false;

    while i < bytes.len() {
        let b = bytes[i];

        if escape {
            escape = false;
            out.push(b as char);
            i += 1;
            continue;
        }
        // Backslash escapes only apply to non-raw strings and char literals
        if b == b'\\' && ((in_string && raw_hashes == 0) || in_char) {
            escape = true;
            out.push(b as char);
            i += 1;
            continue;
        }

        if in_string {
            // For raw strings, the closing is `"` followed by the same number of `#`s.
            if raw_hashes > 0 {
                if b == b'"'
                    && i + raw_hashes < bytes.len()
                    && bytes[(i + 1)..=(i + raw_hashes)].iter().all(|&c| c == b'#')
                {
                    // Emit the `"` and the closing `#`s
                    for k in 0..=raw_hashes {
                        out.push(bytes[i + k] as char);
                    }
                    i += raw_hashes + 1;
                    in_string = false;
                    raw_hashes = 0;
                    continue;
                }
            } else if b == b'"' {
                in_string = false;
            }
            out.push(b as char);
            i += 1;
            continue;
        }
        if in_char {
            if b == b'\'' {
                in_char = false;
            }
            out.push(b as char);
            i += 1;
            continue;
        }

        // Line comment: `// ...` up to end of line (preserve the newline).
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            if i < bytes.len() {
                out.push('\n');
                i += 1;
            }
            continue;
        }

        // Block comment: `/* ... */` (can span multiple lines — preserve newlines)
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            if i + 1 < bytes.len() {
                i += 2;
            }
            continue;
        }

        // Detect raw string: r#"..."# or r##"..."## etc.
        // Also handle `br#"..."#` (byte raw strings).
        let raw_start = if b == b'r' || (b == b'b' && i + 1 < bytes.len() && bytes[i + 1] == b'r') {
            let prefix_len = if b == b'b' { 2 } else { 1 };
            let mut j = i + prefix_len;
            let hash_start = j;
            while j < bytes.len() && bytes[j] == b'#' {
                j += 1;
            }
            let hashes = j - hash_start;
            if hashes > 0 && j < bytes.len() && bytes[j] == b'"' {
                // Found r#"... or r##"... etc.
                Some((j + 1, hashes, prefix_len + hashes + 1))
            } else {
                None
            }
        } else {
            None
        };

        if let Some((string_start, hashes, prefix_total)) = raw_start {
            // Emit the prefix `r#"` (or `br#"`) verbatim
            for k in 0..prefix_total {
                out.push(bytes[i + k] as char);
            }
            i = string_start;
            in_string = true;
            raw_hashes = hashes;
            continue;
        }

        // Enter regular string literal
        if b == b'"' {
            in_string = true;
            raw_hashes = 0;
        } else if b == b'\'' {
            // Heuristic for char vs lifetime: look ahead for closing `'` within 5 bytes
            let end = (i + 5).min(bytes.len());
            let found_close = bytes[(i + 1)..end].contains(&b'\'');
            if found_close {
                in_char = true;
            }
        }

        out.push(b as char);
        i += 1;
    }

    out
}

// ---------------------------------------------------------------------------
// Regex-pattern heuristic for secret detection
// ---------------------------------------------------------------------------

/// Returns true if `snippet` contains regex meta-sequences that strongly
/// suggest it's a regex pattern DEFINITION rather than real secret-looking
/// user content.
///
/// Used by the secret-pattern detector to skip matches on the pattern strings
/// themselves (e.g., `"://[^:]+:[^@]+@"` matching its own definition).
pub fn looks_like_regex_pattern(snippet: &str) -> bool {
    snippet.contains("[^")
        || snippet.contains("(?:")
        || snippet.contains("(?P")
        || snippet.contains("\\w")
        || snippet.contains("\\d")
        || snippet.contains("\\s")
        || snippet.contains("\\b")
        || snippet.contains("[A-Z")
        || snippet.contains("[a-z")
        || snippet.contains("[0-9")
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Strip test blocks from `content` based on the file extension of
/// `file_path`.  Returns the cleaned content ready for security /
/// route / secret-pattern scanning.
///
/// - `.rs` → removes `#[cfg(test)] mod … { … }` blocks
/// - `.py` / `.pyi` → removes `class Test*` and `def test_*` blocks
/// - All other extensions → returned unchanged (JS/TS test blocks are
///   `describe()`/`it()`/`test()` call expressions that are difficult to
///   strip without a full AST; false positives there are much rarer)
pub fn strip_test_blocks(content: &str, file_path: &str) -> String {
    let ext = file_path
        .rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "rs" => strip_rust_test_blocks(content),
        "py" | "pyi" => strip_python_test_blocks(content),
        _ => content.to_string(),
    }
}

/// Strip both test blocks AND comments from source content. Appropriate
/// input for security / route / secret detectors, which shouldn't match on
/// literal example patterns inside either test fixtures or documentation
/// comments.
///
/// - `.rs` → strips `#[cfg(test)] mod … { … }` blocks AND `//` / `/* */` comments
/// - `.py` / `.pyi` → strips test classes/functions (no comment stripping yet)
/// - Other → unchanged
pub fn strip_test_blocks_and_comments(content: &str, file_path: &str) -> String {
    let ext = file_path
        .rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "rs" => strip_rust_comments(&strip_rust_test_blocks(content)),
        "py" | "pyi" => strip_python_test_blocks(content),
        _ => content.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Rust stripping ───────────────────────────────────────────────────────

    #[test]
    fn test_strip_rust_test_blocks_removes_cfg_test_mod() {
        let input = "pub fn real() {}\n\n#[cfg(test)]\nmod tests {\n    let secret = \"AKIA123\";\n    app_get(\"/users\");\n}\n\npub fn after() {}";
        let out = strip_rust_test_blocks(input);
        assert!(
            !out.contains("AKIA123"),
            "AWS key pattern inside test block must be stripped"
        );
        assert!(
            !out.contains("app_get"),
            "route call inside test block must be stripped"
        );
        assert!(out.contains("pub fn real"), "real code must be preserved");
        assert!(
            out.contains("pub fn after"),
            "code after block must be preserved"
        );
    }

    #[test]
    fn test_strip_rust_preserves_line_count() {
        let input = "line1\n#[cfg(test)]\nmod tests {\n    stuff\n    more\n}\nline7\n";
        let out = strip_rust_test_blocks(input);
        // The output must have the same number of lines as the input so that
        // any line-number calculations for code after the block stay correct.
        let input_lines = input.lines().count();
        let out_lines = out.lines().count();
        assert_eq!(
            out_lines, input_lines,
            "line count must be preserved after stripping: input={input_lines} out={out_lines}"
        );
    }

    #[test]
    fn test_strip_rust_preserves_non_test_code() {
        let input = "fn foo() { let x = 1; }\n\npub fn bar() {}";
        let out = strip_rust_test_blocks(input);
        assert_eq!(out, input, "content without test blocks must be unchanged");
    }

    #[test]
    fn test_strip_rust_nested_braces_handled() {
        // The test block itself has nested braces — make sure brace-counting works.
        let input = "fn real() {}\n#[cfg(test)]\nmod tests {\n    fn inner() {\n        if true { let s = \"AKIA_FAKE\"; }\n    }\n}\nfn also_real() {}";
        let out = strip_rust_test_blocks(input);
        assert!(!out.contains("AKIA_FAKE"), "nested AKIA must be stripped");
        assert!(out.contains("fn real"), "real fn must survive");
        assert!(out.contains("fn also_real"), "fn after block must survive");
    }

    #[test]
    fn test_strip_rust_multiple_test_blocks() {
        let input = "fn a() {}\n#[cfg(test)]\nmod tests_a {\n    let k = \"AKIA1\";\n}\nfn b() {}\n#[cfg(test)]\nmod tests_b {\n    let k = \"AKIA2\";\n}\nfn c() {}";
        let out = strip_rust_test_blocks(input);
        assert!(!out.contains("AKIA1"), "first test block must be stripped");
        assert!(!out.contains("AKIA2"), "second test block must be stripped");
        assert!(out.contains("fn a"), "fn a must survive");
        assert!(out.contains("fn b"), "fn b must survive");
        assert!(out.contains("fn c"), "fn c must survive");
    }

    // ── Python stripping ─────────────────────────────────────────────────────

    #[test]
    fn test_strip_python_removes_test_class() {
        let input = "def real():\n    pass\n\nclass TestFoo:\n    def test_one(self):\n        secret = 'AKIA'\n\ndef after():\n    pass\n";
        let out = strip_python_test_blocks(input);
        assert!(
            !out.contains("AKIA"),
            "secret inside TestFoo must be stripped"
        );
        assert!(out.contains("def real"), "real fn must survive");
        assert!(out.contains("def after"), "fn after class must survive");
    }

    #[test]
    fn test_strip_python_removes_def_test_function() {
        let input = "def production():\n    pass\n\ndef test_something():\n    secret = 'AKIA'\n\ndef other():\n    pass\n";
        let out = strip_python_test_blocks(input);
        assert!(
            !out.contains("AKIA"),
            "secret inside def test_ must be stripped"
        );
        assert!(out.contains("def production"), "production fn must survive");
        assert!(out.contains("def other"), "fn after test fn must survive");
    }

    #[test]
    fn test_strip_python_preserves_non_test_code() {
        let input =
            "class RealClass:\n    def method(self):\n        pass\n\ndef helper():\n    pass\n";
        let out = strip_python_test_blocks(input);
        // Non-test class names (not starting with Test) must be preserved.
        assert!(
            out.contains("class RealClass"),
            "non-test class must survive"
        );
        assert!(out.contains("def helper"), "helper must survive");
    }

    // ── Comment stripping ────────────────────────────────────────────────────

    #[test]
    fn test_strip_rust_comments_removes_line_comments() {
        let input =
            "fn real() {\n    // this is a comment with secret AKIA123\n    println!(\"hi\");\n}\n";
        let out = strip_rust_comments(input);
        assert!(
            !out.contains("AKIA123"),
            "secret in line comment must be stripped"
        );
        assert!(out.contains("fn real"), "real code must survive");
        assert!(
            out.contains("println!"),
            "code outside comment must survive"
        );
    }

    #[test]
    fn test_strip_rust_comments_removes_block_comments() {
        let input = "fn real() {\n    /* example: secret AKIA123 in docs */\n    let x = 1;\n}";
        let out = strip_rust_comments(input);
        assert!(
            !out.contains("AKIA123"),
            "secret in block comment must be stripped"
        );
        assert!(out.contains("fn real"));
        assert!(out.contains("let x = 1"));
    }

    #[test]
    fn test_strip_rust_comments_preserves_string_with_double_slash() {
        let input = "let url = \"https://example.com/path\";\n";
        let out = strip_rust_comments(input);
        // The `//` inside the string should NOT be treated as starting a comment
        assert!(
            out.contains("https://example.com/path"),
            "double-slash inside string must survive"
        );
    }

    #[test]
    fn test_strip_rust_comments_preserves_line_count() {
        let input = "line1\n// comment\nline3\n/* block\ncomment */\nline6\n";
        let out = strip_rust_comments(input);
        assert_eq!(out.lines().count(), input.lines().count());
    }

    // ── Regex pattern heuristic ──────────────────────────────────────────────

    #[test]
    fn test_looks_like_regex_pattern_detects_negated_class() {
        assert!(looks_like_regex_pattern("://[^:]+:[^@]+@"));
    }

    #[test]
    fn test_looks_like_regex_pattern_detects_word_metacharacter() {
        assert!(looks_like_regex_pattern("AKIA[A-Z0-9]{16}"));
    }

    #[test]
    fn test_looks_like_regex_pattern_skips_real_secrets() {
        assert!(!looks_like_regex_pattern("AKIAIOSFODNN7EXAMPLE"));
        assert!(!looks_like_regex_pattern("postgres://user:pass@host/db"));
        assert!(!looks_like_regex_pattern("ghp_1234567890abcdefghij"));
    }

    // ── Dispatch ─────────────────────────────────────────────────────────────

    #[test]
    fn test_strip_test_blocks_and_comments_strips_both() {
        let input = "fn real() {}\n// secret AKIA1 in comment\n#[cfg(test)]\nmod tests {\n    let s = \"AKIA2\";\n}\nfn after() {}";
        let out = strip_test_blocks_and_comments(input, "src/lib.rs");
        assert!(!out.contains("AKIA1"), "comment-secret stripped");
        assert!(!out.contains("AKIA2"), "test-block-secret stripped");
        assert!(out.contains("fn real"));
        assert!(out.contains("fn after"));
    }

    #[test]
    fn test_strip_test_blocks_dispatches_by_extension() {
        let rust_input = "fn ok() {}\n#[cfg(test)]\nmod tests {\n    let x = \"secret\";\n}\n";
        let py_input = "def ok():\n    pass\n\nclass TestFoo:\n    secret = 'AKIA'\n";
        let js_input = "describe('test', () => { const s = 'AKIA'; });\n";

        let rust_out = strip_test_blocks(rust_input, "src/lib.rs");
        let py_out = strip_test_blocks(py_input, "app/foo.py");
        let js_out = strip_test_blocks(js_input, "app/foo.js");

        assert!(!rust_out.contains("secret"), "Rust test block stripped");
        assert!(!py_out.contains("AKIA"), "Python test class stripped");
        // JS/TS is not stripped — returned as-is
        assert_eq!(js_out, js_input, "JS content must be unchanged");
    }
}
