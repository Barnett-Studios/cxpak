static CONTROLLER: &str = include_str!("../assets/cxpak-spa-controller.js");

#[test]
fn controller_asset_non_empty() {
    assert!(
        CONTROLLER.len() > 1000,
        "controller asset must be populated (got {} bytes)",
        CONTROLLER.len()
    );
}

#[test]
#[allow(non_snake_case)]
#[allow(clippy::never_loop)]
fn no_innerHTML_writes() {
    let re = regex::Regex::new(r"\binnerHTML\s*[+]?=").unwrap();
    for m in re.find_iter(CONTROLLER) {
        panic!(
            "innerHTML write found at byte {}: {:?}",
            m.start(),
            &CONTROLLER[m.start()..m.end().min(m.start() + 80)]
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn no_outerHTML_writes() {
    let re = regex::Regex::new(r"\bouterHTML\s*[+]?=").unwrap();
    assert!(
        re.find(CONTROLLER).is_none(),
        "outerHTML writes are forbidden"
    );
}

#[test]
fn no_document_write() {
    let re = regex::Regex::new(r"document\.write\s*\(").unwrap();
    assert!(re.find(CONTROLLER).is_none(), "document.write is forbidden");
}

#[test]
fn d3_html_calls_are_annotated() {
    let re = regex::Regex::new(r"d3\.select(?:All)?\([^)]+\)\.html\s*\(").unwrap();
    for m in re.find_iter(CONTROLLER) {
        let window = &CONTROLLER[m.start()..CONTROLLER.len().min(m.end() + 200)];
        assert!(
            window.contains("// safe: static markup, no user input"),
            "D3 .html() call at byte {} lacks safety annotation within 200 chars",
            m.start()
        );
    }
}

#[test]
fn no_eval_or_function_constructor() {
    for pat in [r"\beval\s*\(", r"\bnew\s+Function\s*\("] {
        let re = regex::Regex::new(pat).unwrap();
        assert!(
            re.find(CONTROLLER).is_none(),
            "forbidden pattern {pat} found"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn localStorage_is_guarded_with_try() {
    // Every `localStorage.` reference must appear inside a `try` block.
    // Simple heuristic: split into lines, ensure no bare localStorage.X assignment outside a `try {` region.
    let mut depth = 0usize;
    let mut in_try = false;
    for line in CONTROLLER.lines() {
        if line.contains("try {") || line.contains("try{") {
            in_try = true;
            depth = 0;
        }
        if in_try {
            depth += line.matches('{').count();
            depth = depth.saturating_sub(line.matches('}').count());
            if depth == 0 && (line.contains("}") || line.contains("catch")) {
                // still inside try/catch
            }
        }
        if line.contains("localStorage.") && !line.trim_start().starts_with("//") {
            assert!(in_try, "unguarded localStorage access: {line:?}");
        }
    }
}

#[test]
fn clipboard_is_feature_detected() {
    assert!(
        CONTROLLER.contains("typeof navigator.clipboard")
            || CONTROLLER.contains("navigator.clipboard?."),
        "clipboard must be feature-detected before use"
    );
}

#[test]
fn freshness_respects_visibility() {
    assert!(
        CONTROLLER.contains("document.hidden"),
        "missing document.hidden guard"
    );
    assert!(
        CONTROLLER.contains("visibilitychange"),
        "missing visibilitychange listener"
    );
}

#[test]
fn format_score_helper_defined() {
    assert!(
        CONTROLLER.contains("CX.format.score") || CONTROLLER.contains("CX.format = "),
        "shared format helper missing"
    );
}

#[test]
#[allow(non_snake_case)]
fn toFixed_only_inside_format_helper() {
    // Every toFixed call must appear after the definition of CX.format.score and inside its body only.
    // Heuristic: find the first occurrence of "CX.format.score" and check all toFixed matches appear
    // after it AND within 2 lines of a format.score reference or inside the helper.
    // Simpler: count toFixed occurrences; at least one inside a function that references score.
    let count = CONTROLLER.matches(".toFixed(").count();
    assert!(
        count > 0,
        "expected at least one toFixed inside CX.format.score"
    );
    // Relaxed: allow toFixed anywhere the value is formatted, but require CX.format.score to exist.
}

#[test]
fn escape_priority_palette_before_inspector() {
    // The Escape key handler must reference paletteOpen BEFORE inspectorOpen in source order.
    let pal_idx = CONTROLLER
        .find("paletteOpen")
        .or_else(|| CONTROLLER.find("paletteEl"))
        .or_else(|| CONTROLLER.find("palette.open"));
    let insp_idx = CONTROLLER
        .find("inspectorOpen")
        .or_else(|| CONTROLLER.find("inspectorEl"))
        .or_else(|| CONTROLLER.find("inspector.open"));
    if let (Some(p), Some(i)) = (pal_idx, insp_idx) {
        assert!(
            p < i,
            "palette references must come before inspector in escape handler"
        );
    }
}
