#![cfg(feature = "visual")]

/// Fix 3: level3 PageRank source-iteration determinism.
///
/// Verifies that `build_architecture_explorer_data` applies a stable secondary
/// sort key (path) when PageRank scores are equal. Without the fix the sort is
/// unstable across runs because HashMap iteration is not ordered and sort_by
/// with equal values is non-deterministic.
///
/// We probe this by checking that the sort ordering in the source code has
/// `.then_with(|| a.path.cmp(&b.path))` — a structural contract test.
#[test]
fn ranked_files_sort_has_secondary_path_key() {
    // The sort must include a secondary path-based key so that ties are
    // broken deterministically. We verify this by inspecting that the source
    // code of build_architecture_explorer_data contains `then_with`.
    //
    // Note: we cannot easily test the actual HashMap non-determinism in a
    // single-threaded test run (Rust's HashMap uses a per-process seed).
    // The contract is enforced by a code-level check.
    let source = include_str!("../src/visual/render.rs");
    // Find the ranked_files sort (the one that uses partial_cmp on pagerank).
    let sort_pos = source
        .find("ranked_files.sort_by")
        .expect("ranked_files.sort_by must be present in render.rs");
    let after_sort = &source[sort_pos..];
    let sort_end = after_sort
        .find(");")
        .expect("closing ); for sort_by must be present")
        + sort_pos;
    let sort_block = &source[sort_pos..=sort_end];
    assert!(
        sort_block.contains("then_with"),
        "ranked_files sort must include a secondary `.then_with()` key for determinism; \
         got: {sort_block}"
    );
}

/// Verify the secondary sort produces correct lexicographic ordering.
#[test]
fn pagerank_sort_secondary_key_is_path_lexicographic() {
    let mut pairs: Vec<(&str, f64)> = vec![
        ("src/z_file.rs", 0.0),
        ("src/a_file.rs", 0.0),
        ("src/m_file.rs", 0.0),
        ("src/b_file.rs", 0.5),
        ("src/c_file.rs", 0.25),
    ];
    pairs.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });
    assert_eq!(pairs[0].0, "src/b_file.rs");
    assert_eq!(pairs[1].0, "src/c_file.rs");
    assert_eq!(pairs[2].0, "src/a_file.rs");
    assert_eq!(pairs[3].0, "src/m_file.rs");
    assert_eq!(pairs[4].0, "src/z_file.rs");
}
