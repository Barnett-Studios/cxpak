//! N2 acceptance (RED until RiskEntry gains `risk_percentile`): within-repo
//! percentile spans [0,1] even when raw risk scores collapse into a tiny band
//! (ADR-0198 — fixes the uniform-teal treemap).

use cxpak::intelligence::risk::compute_risk_ranking;
use cxpak::test_support::index_with;

#[test]
fn percentile_spreads_full_range_even_when_raw_scores_collapse() {
    // Raw risk scores realistically live in ~[0, 0.04]; percentile must still
    // span [0, 1].
    let ranking = compute_risk_ranking(&index_with().n_risky_files(10).build());
    let ps: Vec<f64> = ranking.iter().map(|e| e.risk_percentile).collect();
    assert!(
        (ps.iter().cloned().fold(f64::MIN, f64::max) - 1.0).abs() < 1e-9,
        "top percentile == 1.0"
    );
    assert!(
        ps.iter().cloned().fold(f64::MAX, f64::min) < 0.2,
        "bottom percentile near 0"
    );
    // Monotonic with raw score (entries are sorted descending by risk_score).
    for w in ranking.windows(2) {
        assert!(
            w[0].risk_score >= w[1].risk_score && w[0].risk_percentile >= w[1].risk_percentile,
            "percentile monotonic with risk_score"
        );
    }
}

#[test]
fn single_file_percentile_is_one() {
    let ranking = compute_risk_ranking(&index_with().file("only.rs").build());
    assert_eq!(ranking.len(), 1);
    assert!((ranking[0].risk_percentile - 1.0).abs() < 1e-9);
}
