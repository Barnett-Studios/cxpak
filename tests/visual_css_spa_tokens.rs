#[test]
fn css_defines_light_mode_tokens() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").expect("css file exists");
    assert!(
        css.contains(r#":root[data-theme="light"]"#),
        "missing light-mode selector"
    );
    assert!(
        css.contains("--bg-primary: #f8f9fc"),
        "missing light bg color"
    );
}

#[test]
fn css_defines_palette_styles() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").unwrap();
    assert!(css.contains(".cxpak-palette"), "missing palette base");
    assert!(
        css.contains(".cxpak-palette-input"),
        "missing palette input"
    );
    assert!(css.contains(".cxpak-palette-item"), "missing palette item");
}

#[test]
fn css_defines_inspector_styles() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").unwrap();
    assert!(
        css.contains(".cxpak-inspector"),
        "missing inspector container"
    );
    assert!(
        css.contains(".cxpak-inspector.open"),
        "missing inspector open state"
    );
}

#[test]
fn css_defines_freshness_states() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").unwrap();
    assert!(css.contains(".cxpak-freshness"));
    assert!(css.contains(".cxpak-freshness.stale"));
    assert!(css.contains(".cxpak-freshness.old"));
}

#[test]
fn css_defines_reduced_motion() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").unwrap();
    assert!(css.contains("prefers-reduced-motion"));
}

#[test]
fn css_defines_focus_ring() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").unwrap();
    assert!(css.contains(":focus-visible"));
}

// ── Fix 3: palette input focus ring ─────────────────────────────────────────

#[test]
fn palette_input_has_focus_ring() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").unwrap();
    assert!(
        css.contains(".cxpak-palette-input:focus-visible"),
        "palette input must have a :focus-visible style; outline: none alone is an a11y regression"
    );
}

// ── Fix 4: natively-focusable elements have focus ring ───────────────────────

#[test]
fn natively_focusable_elements_have_focus_ring() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").unwrap();
    assert!(
        css.contains("button:focus-visible"),
        "button must get focus ring"
    );
    assert!(
        css.contains("a[href]:focus-visible"),
        "anchor must get focus ring"
    );
    assert!(
        css.contains("input:focus-visible"),
        "input must get focus ring"
    );
}

// ── Fix 5: long paths have ellipsis overflow ─────────────────────────────────

#[test]
fn long_paths_have_ellipsis_overflow() {
    let css = std::fs::read_to_string("assets/cxpak-visual.css").unwrap();
    fn has_ellipsis_block(css: &str, selector: &str) -> bool {
        let pos = css
            .find(selector)
            .unwrap_or_else(|| panic!("selector not found: {selector}"));
        let block = &css[pos..];
        let block_end = block.find('}').unwrap();
        let block = &block[..block_end];
        block.contains("text-overflow: ellipsis") && block.contains("overflow: hidden")
    }
    assert!(
        has_ellipsis_block(&css, ".cxpak-inspector-value"),
        "inspector-value missing ellipsis overflow"
    );
    assert!(
        has_ellipsis_block(&css, ".cxpak-palette-item .label"),
        "palette label missing ellipsis overflow"
    );
}
