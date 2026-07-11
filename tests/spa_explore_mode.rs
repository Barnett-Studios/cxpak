//! N9 acceptance: Architecture + Risk collapse into one Explore mode with a
//! Dependencies|Risk lens toggle (ADR-0192). One nav item, both lenses in the
//! DOM, no standalone Architecture/Risk tabs.
#![cfg(feature = "visual")]

use cxpak::test_support::render_fixture_spa;

#[test]
fn explore_mode_replaces_architecture_and_risk_tabs() {
    let html = render_fixture_spa();

    // Single Explore nav item …
    assert!(
        html.contains("data-view=\"explore\" href=\"#explore\""),
        "Explore nav item present"
    );
    // … and the two former nav tabs are gone.
    assert!(
        !html.contains(">Architecture</a>"),
        "Architecture nav tab removed"
    );
    assert!(!html.contains(">Risk</a>"), "Risk nav tab removed");

    // Both lenses render into their panels under one view.
    assert!(
        html.contains("id=\"view-explore\""),
        "explore view shell present"
    );
    assert!(
        html.contains("id=\"explore-deps\""),
        "Dependencies lens panel present"
    );
    assert!(
        html.contains("id=\"explore-risk\""),
        "Risk lens panel present"
    );
    assert!(
        html.contains("data-lens=\"deps\"") && html.contains("data-lens=\"risk\""),
        "both lens toggle buttons present"
    );

    // The Explore renderer is wired.
    assert!(
        html.contains("CX.init['explore']"),
        "explore init registered"
    );

    // Risk is the default lens (deps panel starts hidden).
    assert!(
        html.contains("id=\"explore-deps\" class=\"cxpak-lens-panel\" hidden"),
        "Risk is the default lens"
    );

    assert!(!html.contains("cdn.jsdelivr.net"), "no CDN");
}
