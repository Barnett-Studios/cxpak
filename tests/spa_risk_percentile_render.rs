//! N5 acceptance: the risk treemap colours by within-repo percentile, not the
//! scale-collapsed absolute score (ADR-0179). Assertions target the treemap
//! specifically — the architecture graph's separate `riskScale` (render.rs) is
//! out of scope for this node and legitimately still keys off risk_score.
#![cfg(feature = "visual")]

use cxpak::test_support::render_fixture_spa;

#[test]
fn treemap_colours_by_percentile_not_the_collapsed_absolute_ramp() {
    let html = render_fixture_spa();

    // risk_percentile is emitted into the treemap cell JSON (via TreemapNode).
    assert!(
        html.contains("risk_percentile"),
        "risk_percentile emitted into treemap cells"
    );
    // The treemap fills by percentile now …
    assert!(
        html.contains("color(d.data.risk_percentile)"),
        "treemap fills by percentile"
    );
    // … and no longer by the collapsed absolute score.
    assert!(
        !html.contains("color(d.data.risk_score)"),
        "absolute-score fill removed from the treemap"
    );
    // The opacity kludge that compensated for the collapsed range is gone.
    assert!(
        !html.contains("r < 0.1 ? 0.5 + r * 5"),
        "opacity kludge removed"
    );
    // The treemap's hardcoded absolute ramp (unique by its darkest stop) is gone.
    assert!(
        !html.contains("'#cc1144'"),
        "collapsed absolute treemap ramp removed"
    );
    assert!(!html.contains("cdn.jsdelivr.net"), "no CDN");
}
