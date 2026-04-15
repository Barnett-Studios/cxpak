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

    // ── Dispatch ─────────────────────────────────────────────────────────────

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
