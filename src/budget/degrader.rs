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
    truncate_to_budget_inner(content, budget, counter, section_name, Some(detail_filename))
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
    for line in content.lines() {
        let line_tokens = counter.count(line) + 1;
        if used + line_tokens > budget.saturating_sub(50) {
            break;
        }
        lines.push(line);
        used += line_tokens;
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
}
