//! N7 acceptance (RED until RiskEntry carries the three factor terms): the
//! provenance drawer needs the literal derivation, so churn_term × blast_term ×
//! test_penalty_term must reproduce risk_score exactly (ADR-0174).

use cxpak::intelligence::risk::compute_risk_ranking;
use cxpak::test_support::index_with;

#[test]
fn risk_terms_multiply_to_score() {
    let ranking = compute_risk_ranking(&index_with().n_risky_files(5).build());
    assert!(!ranking.is_empty(), "risk set must be non-empty");
    for e in &ranking {
        let recomposed = e.churn_term * e.blast_term * e.test_penalty_term;
        assert!(
            (recomposed - e.risk_score).abs() < 1e-9,
            "churn_term × blast_term × test_penalty_term must reproduce risk_score, got {recomposed} vs {}",
            e.risk_score
        );
    }
}
