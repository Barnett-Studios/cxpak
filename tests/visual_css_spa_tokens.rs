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
