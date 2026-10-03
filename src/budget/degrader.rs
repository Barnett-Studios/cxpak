pub fn omission_marker(section: &str, omitted_tokens: usize, min_budget: usize) -> String {
    let display_tokens = if omitted_tokens >= 1000 {
        format!("~{:.1}k", omitted_tokens as f64 / 1000.0)
    } else {
        format!("~{}", omitted_tokens)
    };
    let display_budget = if min_budget >= 1000 {
        format!("{}k+", min_budget / 1000)
    } else {
        format!("{}+", min_budget)
    };
    format!("<!-- {section} omitted: {display_tokens} tokens. Use --tokens {display_budget} to include -->")
}

pub fn truncate_to_budget(
    content: &str,
    budget: usize,
    counter: &crate::budget::counter::TokenCounter,
    section_name: &str,
) -> (String, usize, usize) {
    truncate_to_budget_inner(content, budget, counter, section_name, None)
}

pub fn truncate_to_budget_with_pointer(
    content: &str,
    budget: usize,
    counter: &crate::budget::counter::TokenCounter,
    section_name: &str,
    detail_filename: &str,
) -> (String, usize, usize) {
    truncate_to_budget_inner(
        content,
        budget,
        counter,
        section_name,
        Some(detail_filename),
    )
}

/// Computes the net change in brace/bracket/paren nesting depth for a single
/// line of source, ignoring any such characters that appear inside
/// double-quoted string literals or after an unquoted `//` line comment.
///
/// This is a conservative, language-agnostic heuristic rather than a real
/// parse: it does not understand raw strings, `#`-comment languages, or
/// char-literal braces (e.g. Rust's `'{'`), so on pathological input it can
/// misjudge a handful of lines. Deliberately single-quote-blind (Rust
/// lifetimes like `&'a str` would otherwise wedge the scanner into a
/// permanent "inside string" state).
///
/// Shared with `auto_context::briefing`'s truncation, which has the same
/// mid-symbol defect for the same reason (issue #41).
///
/// Callers must treat depth `<= 0` as safe to cut at, not only depth `== 0`:
/// a stray unmatched closer in prose or a comment — a parenthetical `(see
/// below)`, a list marker `1)`, an emoticon `:)` — drives depth negative
/// without ever opening an unterminated block, and the cut point only needs
/// depth to be *at or below* its starting value (not back to exactly zero)
/// to be free of a dangling open symbol. Treating depth `== 0` as the only
/// safe point would, on content that's mostly prose with incidental parens,
/// find no safe boundary at all after the first stray `)` and over-truncate
/// everything that follows — the opposite failure from the one this
/// function exists to prevent. Only depth `> 0` (genuinely inside an open,
/// unterminated brace/bracket/paren) is unsafe to cut at.
pub(crate) fn bracket_depth_delta(line: &str) -> i64 {
    let mut delta: i64 = 0;
    let mut in_string = false;
    let mut prev = '\0';
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if !in_string && ch == '/' && chars.peek() == Some(&'/') {
            break; // rest of the line is a `//` comment
        }
        if in_string {
            if ch == '"' && prev != '\\' {
                in_string = false;
            }
            prev = ch;
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' | '[' | '(' => delta += 1,
            '}' | ']' | ')' => delta -= 1,
            _ => {}
        }
        prev = ch;
    }
    delta
}

fn truncate_to_budget_inner(
    content: &str,
    budget: usize,
    counter: &crate::budget::counter::TokenCounter,
    section_name: &str,
    detail_filename: Option<&str>,
) -> (String, usize, usize) {
    let total_tokens = counter.count(content);
    if total_tokens <= budget {
        return (content.to_string(), total_tokens, 0);
    }

    let mut lines = Vec::new();
    let mut used = 0;
    let mut depth: i64 = 0;
    let mut last_safe_boundary: Option<(usize, usize)> = None;
    let mut cut_short = false;
    for line in content.lines() {
        let line_tokens = counter.count(line) + 1;
        // Reserve 150 tokens for the omission marker.  The marker text can
        // reach ~100 tokens for long section names and large token counts, so
        // 50 was too small and could result in the final content exceeding the
        // caller's budget once the marker is appended.
        if used + line_tokens > budget.saturating_sub(150) {
            cut_short = true;
            break;
        }
        lines.push(line);
        used += line_tokens;
        depth += bracket_depth_delta(line);
        if depth <= 0 {
            last_safe_boundary = Some((lines.len(), used));
        }
    }

    // A raw line-count cut can land partway through a function/block,
    // emitting an opening brace with no matching close — a structurally
    // broken fragment (issue #41). When the cut happened while genuinely
    // inside an open block (depth > 0), back off to the last point where
    // nesting was at or below its starting depth so the retained content
    // never ends on a half-open symbol. depth <= 0 is intentionally treated
    // as already safe — see `bracket_depth_delta`'s doc comment.
    if cut_short && depth > 0 {
        match last_safe_boundary {
            Some((safe_len, safe_used)) => {
                lines.truncate(safe_len);
                used = safe_used;
            }
            None => {
                // No balanced point was ever reached — the cut lands inside
                // the very first symbol. Emit nothing rather than a
                // half-open fragment.
                lines.clear();
                used = 0;
            }
        }
    }

    let omitted = total_tokens - used;
    let marker = match detail_filename {
        Some(filename) => omission_pointer(section_name, filename, omitted),
        None => omission_marker(section_name, omitted, used + omitted + 500),
    };
    let mut truncated = lines.join("\n");
    truncated.push('\n');
    truncated.push_str(&marker);
    (truncated, used, omitted)
}

pub fn omission_pointer(section: &str, filename: &str, omitted_tokens: usize) -> String {
    let display_tokens = if omitted_tokens >= 1000 {
        format!("~{:.1}k", omitted_tokens as f64 / 1000.0)
    } else {
        format!("~{}", omitted_tokens)
    };
    format!("<!-- {section} full content: .cxpak/{filename} ({display_tokens} tokens) -->")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_omission_marker_small() {
        let marker = omission_marker("git context", 500, 3000);
        assert!(marker.contains("git context"));
        assert!(marker.contains("~500"));
        assert!(marker.contains("3k+"));
    }

    #[test]
    fn test_omission_marker_large() {
        let marker = omission_marker("signatures", 15000, 50000);
        assert!(marker.contains("~15.0k"));
        assert!(marker.contains("50k+"));
    }

    #[test]
    fn test_truncate_fits() {
        let counter = crate::budget::counter::TokenCounter::new();
        let content = "line one\nline two\nline three";
        let (result, used, omitted) = truncate_to_budget(content, 100, &counter, "test");
        assert_eq!(result, content.to_string());
        assert_eq!(omitted, 0);
        assert!(used > 0);
    }

    #[test]
    fn test_omission_pointer() {
        let pointer = omission_pointer("signatures", "signatures.md", 39400);
        assert!(pointer.contains(".cxpak/signatures.md"));
        assert!(pointer.contains("~39.4k tokens"));
        assert!(pointer.contains("full content"));
    }

    #[test]
    fn test_truncate_with_pointer() {
        let counter = crate::budget::counter::TokenCounter::new();
        let content = (0..100)
            .map(|i| format!("this is line number {} with some padding text", i))
            .collect::<Vec<_>>()
            .join("\n");
        let (result, _used, omitted) =
            truncate_to_budget_with_pointer(&content, 10, &counter, "module map", "modules.md");
        assert!(omitted > 0);
        assert!(result.contains(".cxpak/modules.md"));
        assert!(!result.contains("Use --tokens"));
    }

    #[test]
    fn test_truncate_exceeds() {
        let counter = crate::budget::counter::TokenCounter::new();
        let content = (0..100)
            .map(|i| format!("this is line number {} with some padding text", i))
            .collect::<Vec<_>>()
            .join("\n");
        let (result, _used, omitted) = truncate_to_budget(&content, 10, &counter, "test section");
        assert!(omitted > 0);
        assert!(result.contains("<!-- test section omitted"));
    }

    #[test]
    fn test_omission_marker_tiny_budget() {
        // Covers the min_budget < 1000 branch (line 10)
        let marker = omission_marker("section", 50, 500);
        assert!(marker.contains("~50"));
        assert!(marker.contains("500+"));
        assert!(!marker.contains("k+"));
    }

    #[test]
    fn test_omission_pointer_small_tokens() {
        // Covers the omitted_tokens < 1000 branch (line 78)
        let pointer = omission_pointer("details", "details.md", 42);
        assert!(pointer.contains("~42"));
        assert!(pointer.contains(".cxpak/details.md"));
        // Small tokens should show "~42" not "~0.0k"
        assert!(!pointer.contains("~0.0k"));
    }

    #[test]
    fn test_truncate_never_leaves_unbalanced_braces() {
        // Issue #41 (defect D): the original implementation truncates on a
        // raw line-count cut with no awareness of block structure, so a
        // budget that lands partway through a function's body emits its
        // opening brace with no matching close — an unbalanced, structurally
        // broken fragment. Sweep a wide range of budgets (forcing the cut
        // point to land inside every function in turn) and assert the
        // retained code is always brace-balanced whenever truncation
        // actually occurred.
        let counter = crate::budget::counter::TokenCounter::new();
        let content = (0..20)
            .map(|i| {
                let body_lines = 3 + (i % 5);
                let body = (0..body_lines)
                    .map(|j| format!("    let v{j} = {j} + {i};"))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("fn function_number_{i}() {{\n{body}\n    v0\n}}\n")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let total = counter.count(&content);
        assert!(total > 400, "fixture too small to exercise the sweep");

        let mut saw_truncation = false;
        for budget in (160..total).step_by(5) {
            let (result, _used, omitted) =
                truncate_to_budget(&content, budget, &counter, "source code");
            if omitted == 0 {
                continue;
            }
            saw_truncation = true;
            let code_only = result.split("<!--").next().unwrap_or(&result);
            let opens = code_only.matches('{').count();
            let closes = code_only.matches('}').count();
            assert_eq!(
                opens, closes,
                "budget {budget} produced unbalanced braces: {code_only:?}"
            );
        }
        assert!(saw_truncation, "sweep never exercised truncation");
    }

    #[test]
    fn test_truncate_prose_with_stray_closers_is_not_over_truncated() {
        // Issue #41 follow-up: a naive depth-counter that only accepts
        // depth == 0 as safe would find no safe boundary at all once prose
        // contains an unmatched closing paren/bracket — e.g. a list marker
        // "1)" or an emoticon ":)" inside a `//` comment — and would then
        // clear the whole retained section down to nothing. None of these
        // lines open or close a real block, so truncation should behave
        // exactly as the plain per-line budget cut would: the depth-based
        // backoff must not fire at all.
        let counter = crate::budget::counter::TokenCounter::new();
        let lines: Vec<String> = (0..60)
            .map(|i| format!("// note {i}: see item 1) above, also known as :) in the docs."))
            .collect();
        let content = lines.join("\n");
        let total = counter.count(&content);
        assert!(total > 300, "fixture too small to force truncation");

        let budget = total / 2;
        let (result, _used, omitted) = truncate_to_budget(&content, budget, &counter, "notes");
        assert!(omitted > 0, "fixture should have actually truncated");
        let code_only = result.split("<!--").next().unwrap_or(&result);
        assert!(
            !code_only.trim().is_empty(),
            "stray ')'/':)' in comment-only prose must not empty out the whole section"
        );
    }

    #[test]
    fn test_truncate_bare_prose_with_stray_closers_is_not_over_truncated() {
        // Same hazard as the comment-wrapped case above, but with the
        // stray closers in bare prose (no `//`, no string) — this is the
        // case that specifically distinguishes depth <= 0 ("safe") from
        // depth == 0 ("safe"): a lone ")" or ":)" drives depth negative and
        // it never climbs back to exactly zero, so a depth == 0 check would
        // never find a safe boundary again and would clear the whole
        // section once truncation kicked in.
        let lines: Vec<String> = (0..60)
            .map(|i| format!("Note {i}: see item 1) above, also known as :) in the docs."))
            .collect();
        let content = lines.join("\n");
        let counter = crate::budget::counter::TokenCounter::new();
        let total = counter.count(&content);
        assert!(total > 300, "fixture too small to force truncation");

        let budget = total / 2;
        let (result, _used, omitted) = truncate_to_budget(&content, budget, &counter, "notes");
        assert!(omitted > 0, "fixture should have actually truncated");
        let code_only = result.split("<!--").next().unwrap_or(&result);
        assert!(
            !code_only.trim().is_empty(),
            "a stray ')'/':)' in bare prose must not empty out the whole section"
        );
    }

    #[test]
    fn test_truncate_snippet_starting_mid_block_does_not_panic_or_misfire() {
        // A snippet that starts already unbalanced (its very first line is
        // a lone closing brace, because whatever opened it was excluded
        // from this content before truncate_to_budget ever saw it) must not
        // panic, and the pre-existing leading imbalance must not be treated
        // as "still inside an open block" — depth goes negative on line 1,
        // which is <= 0 and therefore a safe cut point.
        let counter = crate::budget::counter::TokenCounter::new();
        let mut content = String::from("}\n\n");
        for i in 0..40 {
            content.push_str(&format!(
                "fn tail_{i}() {{\n    let x = {i};\n    x\n}}\n\n"
            ));
        }
        let total = counter.count(&content);
        assert!(total > 300, "fixture too small to force truncation");

        // A budget that only fits the leading orphan "}" plus a little more
        // must still retain that first line rather than clearing everything.
        let budget = counter.count("}\n") + 200;
        let (result, _used, omitted) = truncate_to_budget(&content, budget, &counter, "tail");
        assert!(omitted > 0, "fixture should have actually truncated");
        let code_only = result.split("<!--").next().unwrap_or(&result);
        assert!(
            !code_only.trim().is_empty(),
            "a pre-existing leading imbalance must not force the whole section to empty"
        );
    }

    #[test]
    fn test_truncate_never_leaves_unbalanced_brackets_or_parens() {
        // Same defect as test_truncate_never_leaves_unbalanced_braces, but
        // for `[]`/`()` nesting from a multi-line call/collection literal —
        // the kind of construct that spans several lines in real code
        // without ever using `{}`.
        let counter = crate::budget::counter::TokenCounter::new();
        let content = (0..15)
            .map(|i| {
                let arg_lines = 3 + (i % 4);
                let args = (0..arg_lines)
                    .map(|j| format!("    value_{j},"))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("let v{i} = vec![\n{args}\n];\n")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let total = counter.count(&content);
        assert!(total > 300, "fixture too small to exercise the sweep");

        let mut saw_truncation = false;
        for budget in (160..total).step_by(5) {
            let (result, _used, omitted) =
                truncate_to_budget(&content, budget, &counter, "source code");
            if omitted == 0 {
                continue;
            }
            saw_truncation = true;
            let code_only = result.split("<!--").next().unwrap_or(&result);
            let opens = code_only.matches('[').count() + code_only.matches('(').count();
            let closes = code_only.matches(']').count() + code_only.matches(')').count();
            assert_eq!(
                opens, closes,
                "budget {budget} produced unbalanced brackets/parens: {code_only:?}"
            );
        }
        assert!(saw_truncation, "sweep never exercised truncation");
    }
}
