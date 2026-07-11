//! N1 acceptance (RED until the cascade fills `surprising_connections`):
//! surprising connections = co-change pairs with no import edge.

use cxpak::intelligence::insights::{surprising_connections, SurprisingLink};
use cxpak::test_support::index_with;

// Single consumer, so the pair-comparison helper lives here rather than in
// test_support (a cxpak trait impl for a cxpak type would violate the orphan
// rule from an integration-test crate; a shared typed helper would also force
// test_support to reference SurprisingLink before N1 exists).
fn unordered_eq(l: &SurprisingLink, x: &str, y: &str) -> bool {
    (l.a == x && l.b == y) || (l.a == y && l.b == x)
}

#[test]
fn surprising_connections_excludes_imported_pairs_and_keeps_unimported() {
    // A imports B AND they co-change → NOT surprising.
    // C and D co-change with NO import edge → surprising.
    let index = index_with()
        .file("A")
        .imports("B")
        .co_change("A", "B", 0.9)
        .co_change("C", "D", 0.8)
        .build();
    let links = surprising_connections(&index);
    assert!(
        links.iter().all(|l| !unordered_eq(l, "A", "B")),
        "imported+co-changed pair must be filtered out"
    );
    assert!(
        links.iter().any(|l| unordered_eq(l, "C", "D")),
        "co-changed-without-import pair must surface"
    );
}

#[test]
fn surprising_connections_is_deterministic() {
    let index = index_with()
        .co_change("C", "D", 0.8)
        .co_change("E", "F", 0.8)
        .build();
    assert_eq!(surprising_connections(&index), surprising_connections(&index));
}

#[test]
fn surprising_connection_score_is_the_cochange_recency_weight() {
    let index = index_with().co_change("C", "D", 0.8).build();
    let links = surprising_connections(&index);
    let cd = links
        .iter()
        .find(|l| unordered_eq(l, "C", "D"))
        .expect("C-D surfaces");
    assert!((cd.co_change_score - 0.8).abs() < 1e-9, "score is recency_weight");
}
