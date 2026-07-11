//! N8 acceptance: the prove-it drawer (ADR-0174). Overview risk rows and alerts
//! expose a `prove` affordance that opens the inspector showing the literal
//! derivation of the risk score from N7's churn/blast/test-penalty terms.
#![cfg(feature = "visual")]

use cxpak::test_support::render_fixture_spa;

#[test]
fn overview_risk_rows_emit_the_prove_it_derivation_drawer() {
    let html = render_fixture_spa();

    // The prove affordance is rendered on risk rows (and alerts).
    assert!(
        html.contains("cxpak-prove-btn"),
        "prove affordance present on Overview"
    );
    assert!(
        html.contains("proveRisk"),
        "proveRisk drawer handler emitted"
    );

    // The derivation string is built from the three N7 terms, in order.
    assert!(
        html.contains(" = churn(")
            && html.contains(") × blast(")
            && html.contains(") × test_penalty("),
        "literal derivation `score = churn(..) × blast(..) × test_penalty(..)` wired"
    );

    // The three provenance terms flow through to the drawer.
    assert!(
        html.contains("r.churn_term"),
        "churn term consumed by drawer"
    );
    assert!(
        html.contains("r.blast_term"),
        "blast term consumed by drawer"
    );
    assert!(
        html.contains("r.test_penalty_term"),
        "test-penalty term consumed by drawer"
    );

    // 'p' opens the drawer without navigating away.
    assert!(
        html.contains("ev.key === 'p'"),
        "p-key opens the prove drawer"
    );

    assert!(!html.contains("cdn.jsdelivr.net"), "no CDN");
}
