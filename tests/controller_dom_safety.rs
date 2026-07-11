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
    // For each `localStorage.` byte offset, find the nearest preceding `try {` AND a `catch`
    // that opens AFTER our offset. If none exists, the call is unguarded.
    let mut search_from = 0usize;
    while let Some(rel) = CONTROLLER[search_from..].find("localStorage.") {
        let abs = search_from + rel;
        // Skip if the line is a comment.
        let line_start = CONTROLLER[..abs].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line = &CONTROLLER[line_start..abs];
        if line.trim_start().starts_with("//") {
            search_from = abs + "localStorage.".len();
            continue;
        }
        // Find the nearest preceding `try {` (or `try{`).
        let try_open = CONTROLLER[..abs]
            .rfind("try {")
            .or_else(|| CONTROLLER[..abs].rfind("try{"));
        let mut guarded = false;
        if let Some(t) = try_open {
            // Walk forward from `t` counting braces; the try block ends when depth returns to 0.
            let mut depth = 0i64;
            let mut end = t;
            for (i, b) in CONTROLLER[t..].bytes().enumerate() {
                if b == b'{' {
                    depth += 1;
                } else if b == b'}' {
                    depth -= 1;
                    if depth == 0 {
                        end = t + i;
                        break;
                    }
                }
            }
            // "guarded" means the localStorage call's offset is between t and end (inside try block).
            if abs > t && abs < end {
                guarded = true;
            }
            // OR between end and end-of-catch-block.
            if !guarded {
                if let Some(catch_off) = CONTROLLER[end..].find("catch") {
                    let catch_abs = end + catch_off;
                    let mut depth2 = 0i64;
                    let mut catch_end = catch_abs;
                    for (i, b) in CONTROLLER[catch_abs..].bytes().enumerate() {
                        if b == b'{' {
                            depth2 += 1;
                        } else if b == b'}' {
                            depth2 -= 1;
                            if depth2 == 0 {
                                catch_end = catch_abs + i;
                                break;
                            }
                        }
                    }
                    if abs > catch_abs && abs < catch_end {
                        guarded = true;
                    }
                }
            }
        }
        assert!(
            guarded,
            "unguarded localStorage access at byte {abs}: line={line:?}"
        );
        search_from = abs + "localStorage.".len();
    }
}

// ── Palette is the single colour control (ADR-0172) ──────────────────────────
// The ☾/☀ theme toggle was removed — light/dark are palette variants applied as
// CSS custom properties on :root, so chrome recolours live with no view re-render.
// Data-encoding colours (health/risk green→yellow→red) are a fixed semantic ramp
// by design, palette-independent. Guard that the obsolete toggle is really gone.

#[test]
fn theme_toggle_is_removed() {
    assert!(
        !CONTROLLER.contains("toggleTheme"),
        "the ☾/☀ theme toggle was replaced by the palette picker (ADR-0172); \
         no toggleTheme should remain in the controller"
    );
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
    let count = CONTROLLER.matches(".toFixed(").count();
    assert_eq!(
        count, 1,
        "expected exactly one .toFixed(...) call (in CX.format.score), found {count}"
    );
    let format_helper_idx = CONTROLLER
        .find("CX.format = {")
        .or_else(|| CONTROLLER.find("CX.format ="))
        .expect("CX.format helper definition missing");
    let to_fixed_idx = CONTROLLER.find(".toFixed(").unwrap();
    let helper_end = CONTROLLER[format_helper_idx..]
        .find("};")
        .expect("CX.format block closing not found")
        + format_helper_idx;
    assert!(
        to_fixed_idx > format_helper_idx && to_fixed_idx < helper_end,
        ".toFixed(...) must appear inside the CX.format helper definition"
    );
}

#[test]
fn escape_priority_palette_before_inspector() {
    // Within the Escape-key handler, the palette check must come before the inspector check.
    // We anchor on the substring "ev.key === 'Escape'" and inspect the source text from there
    // forward to confirm paletteOpen is referenced before CX.state.inspector.
    let escape_idx = CONTROLLER
        .find("ev.key === 'Escape'")
        .expect("escape handler missing");
    let after = &CONTROLLER[escape_idx..];
    let pal = after
        .find("CX.state.paletteOpen")
        .expect("palette check missing in escape handler");
    let insp = after
        .find("CX.state.inspector")
        .expect("inspector check missing in escape handler");
    assert!(
        pal < insp,
        "palette check must precede inspector check in escape handler (pal={pal}, insp={insp})"
    );
}

#[test]
fn palette_items_have_role_option_and_stable_ids() {
    assert!(
        CONTROLLER.contains("'role', 'option'") || CONTROLLER.contains("\"role\", \"option\""),
        "palette items must be marked role=option"
    );
    assert!(
        CONTROLLER.contains("aria-activedescendant"),
        "palette input must track active option via aria-activedescendant"
    );
    assert!(
        CONTROLLER.contains("'aria-selected'") || CONTROLLER.contains("\"aria-selected\""),
        "palette items must toggle aria-selected"
    );
    assert!(
        CONTROLLER.contains("cxpak-palette-item-"),
        "palette items must have stable IDs (pattern: cxpak-palette-item-N)"
    );
}

#[test]
fn inspector_accepts_optional_fields_parameter() {
    assert!(
        CONTROLLER.contains("function openInspector(node, opts)")
            || CONTROLLER.contains("function openInspector(node, options)"),
        "openInspector must accept an optional fields parameter for context-aware display"
    );
}
