//! Accessibility & color-blindness adversarial tests for the SPA dashboard.
//!
//! Each test locks an invariant identified by the v2.1.1 critical UI/UX
//! evaluators: palette ARIA combobox pattern, inspector dialog role, severity
//! letter-codes for color-blind users, screen-reader live announcements,
//! help-overlay focus restoration.
#![cfg(feature = "visual")]

fn fixture_metadata() -> cxpak::visual::render::RenderMetadata {
    cxpak::visual::render::RenderMetadata {
        repo_name: "spa_a11y_test".to_string(),
        generated_at: "[REDACTED]".to_string(),
        health_score: None,
        node_count: 0,
        edge_count: 0,
        cxpak_version: "[REDACTED]".to_string(),
    }
}

fn render() -> String {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let idx = cxpak::index::CodebaseIndex::build_with_content(
        vec![],
        std::collections::HashMap::new(),
        &counter,
        std::collections::HashMap::new(),
    );
    cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).expect("render_spa")
}

/// Palette input MUST follow the ARIA combobox pattern. Without
/// role=combobox + aria-controls + aria-expanded, screen readers treat
/// the input as a plain text field and never announce the result list.
#[test]
fn palette_input_implements_aria_combobox_pattern() {
    let html = render();
    assert!(
        html.contains(r#"id="cxpak-palette-input""#),
        "palette input must exist"
    );
    let input_attrs = html
        .lines()
        .find(|l| l.contains(r#"id="cxpak-palette-input""#))
        .expect("palette input line");
    assert!(
        input_attrs.contains(r#"role="combobox""#),
        "palette input must declare role=combobox; got: {input_attrs}"
    );
    assert!(
        input_attrs.contains(r#"aria-controls="cxpak-palette-results""#),
        "palette input must aria-controls the results listbox; got: {input_attrs}"
    );
    assert!(
        input_attrs.contains(r#"aria-autocomplete="list""#),
        "palette input must declare aria-autocomplete=list; got: {input_attrs}"
    );
    assert!(
        input_attrs.contains(r#"aria-expanded="true""#),
        "palette input must declare aria-expanded=true (results visible while open); got: {input_attrs}"
    );
}

/// Palette listbox MUST have an aria-label so screen readers announce it
/// when navigated to via the input's aria-controls relationship.
#[test]
fn palette_results_listbox_has_aria_label() {
    let html = render();
    let listbox_line = html
        .lines()
        .find(|l| l.contains(r#"id="cxpak-palette-results""#))
        .expect("palette listbox line");
    assert!(
        listbox_line.contains(r#"role="listbox""#),
        "results must have role=listbox"
    );
    assert!(
        listbox_line.contains(r#"aria-label="Palette results""#),
        "results listbox must have aria-label; got: {listbox_line}"
    );
}

/// Inspector MUST be role=dialog, not role=complementary. role=complementary
/// is a landmark — focus is never trapped inside; role=dialog signals a
/// modal-like region the focus trap can recognize.
#[test]
fn inspector_uses_dialog_role_not_complementary() {
    let html = render();
    let aside = html
        .lines()
        .find(|l| l.contains(r#"id="cxpak-inspector""#))
        .expect("inspector aside line");
    assert!(
        aside.contains(r#"role="dialog""#),
        "inspector must use role=dialog; got: {aside}"
    );
    assert!(
        !aside.contains(r#"role="complementary""#),
        "inspector must NOT use role=complementary anymore"
    );
}

/// Result-count announcement to the live region: the controller must call
/// out the result count when the user types in the palette. Without this,
/// screen readers get no audible cue when results change.
#[test]
fn controller_announces_palette_result_count_to_live_region() {
    let js = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/cxpak-spa-controller.js"
    ))
    .expect("controller js");
    // The renderPaletteResults function must touch the live region with a
    // count-bearing string.
    assert!(
        js.contains("'cxpak-live'") || js.contains("\"cxpak-live\""),
        "controller must reference the cxpak-live region"
    );
    assert!(
        js.contains("' results'") || js.contains("'1 result'"),
        "controller must announce a count-bearing string ('N results' / '1 result')"
    );
    assert!(
        js.contains("No results"),
        "controller must announce empty state to the live region"
    );
}

/// Severity badge in the dashboard top-risks table MUST contain a
/// non-color discriminator (one of H/M/L letters) so deuteranopic /
/// protanopic users can distinguish severity without color.
#[test]
fn severity_badge_includes_letter_discriminator() {
    let html = render();
    // The dashboard renderer is inlined in the SPA; the controller
    // generates the H/M/L letter at runtime, but the JS source MUST
    // contain the letter mapping for it to render.
    assert!(
        html.contains("sevLetter") || html.contains("severity-dot"),
        "dashboard renderer must reference severity-dot styling"
    );
    // The SPA bundles render.rs's dashboard JS; check the inlined source
    // for the letter-code mapping.
    assert!(
        html.contains("'high' ? 'H'"),
        "dashboard JS must map severity high → letter 'H' for color-blind users"
    );
    assert!(
        html.contains("'medium' ? 'M'"),
        "dashboard JS must map severity medium → letter 'M'"
    );
    // ARIA label for screen readers (visible label is the letter; SR users
    // need the full name).
    assert!(
        html.contains("'High risk'"),
        "dashboard JS must include 'High risk' aria-label for SR users"
    );
}

/// Help overlay MUST save pre-help focus and restore it on close. Without
/// this, keyboard users who press `?` then `Esc` lose their place.
#[test]
fn help_overlay_saves_and_restores_focus() {
    let js = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/cxpak-spa-controller.js"
    ))
    .expect("controller js");
    // The `?` keybinding handler must capture document.activeElement before
    // opening the overlay.
    assert!(
        js.contains("preHelpFocus"),
        "controller must save preHelpFocus before opening help overlay"
    );
    // closeHelp must restore focus to the saved element.
    assert!(
        js.contains("closeHelp") && js.contains("preHelpFocus"),
        "closeHelp must reference preHelpFocus to restore"
    );
}

/// Severity dot CSS must size the badge large enough for the letter to be
/// legible (>= 12px). The old 8x8px circle was too small for a letter.
#[test]
fn severity_dot_css_is_large_enough_for_letter() {
    let css = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/cxpak-visual.css"
    ))
    .expect("css");
    // Find the .cxpak-severity-dot block and assert width/height >= 12px.
    let block = css
        .split(".cxpak-severity-dot ")
        .nth(1)
        .or_else(|| css.split(".cxpak-severity-dot{").nth(1))
        .expect(".cxpak-severity-dot block");
    let block = block.split('}').next().unwrap_or("");
    let parse_px = |attr: &str| -> Option<u32> {
        block.lines().find_map(|line| {
            let line = line.trim();
            if !line.starts_with(attr) {
                return None;
            }
            // attr: NN px;
            let rhs = line.split(':').nth(1)?.trim();
            let num = rhs.split("px").next()?.trim();
            num.parse::<u32>().ok()
        })
    };
    let w = parse_px("width").expect("width: NNpx in severity-dot");
    let h = parse_px("height").expect("height: NNpx in severity-dot");
    assert!(
        w >= 12,
        "severity-dot width must be >= 12px to fit a legible letter, got {w}px"
    );
    assert!(
        h >= 12,
        "severity-dot height must be >= 12px to fit a legible letter, got {h}px"
    );
}
