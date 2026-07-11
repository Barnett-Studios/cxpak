// D2.2 — recall@budget metric + baselines.
//
// Two test layers:
//   1. Pure metric correctness (NO network/compute) — `recall_at_budget`, `mrr`,
//      and `render_comparison_table` on synthetic inputs. Runs in the default
//      `cargo test --features bench` invocation.
//   2. Harness smoke (NETWORK + compute) — the full pipeline on a 2–3 entry
//      subset of the corpus. `#[ignore]`d and additionally env-gated behind
//      `CXPAK_BENCH_NET`, so default `cargo test` never touches the network.
//
// Requires the `bench` feature; without it the crate exposes no `bench` module.
#![cfg(feature = "bench")]

use cxpak::bench::recall::{
    ground_truth_at_base, mrr, recall_at_budget, render_comparison_table, ChangedFile, SystemResult,
};
use std::collections::HashSet;

fn gt(files: &[&str]) -> HashSet<String> {
    files.iter().map(|s| s.to_string()).collect()
}

/// Construct a `ChangedFile` record (mirrors the `gh api .../files` shape).
fn cf(filename: &str, status: &str, previous: Option<&str>) -> ChangedFile {
    ChangedFile {
        filename: filename.to_string(),
        status: status.to_string(),
        previous_filename: previous.map(str::to_string),
    }
}

// ── recall_at_budget ──────────────────────────────────────────────────────

#[test]
fn recall_empty_ground_truth_is_one() {
    // An empty denominator means "nothing to retrieve" — define recall as 1.0
    // (vacuously perfect) so empty-gt entries don't drag the mean toward 0.
    let selected = vec!["a.rs".to_string()];
    assert_eq!(recall_at_budget(&selected, &gt(&[])), 1.0);
}

#[test]
fn recall_perfect_is_one() {
    let selected = vec!["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()];
    assert_eq!(recall_at_budget(&selected, &gt(&["a.rs", "b.rs"])), 1.0);
}

#[test]
fn recall_zero_overlap_is_zero() {
    let selected = vec!["x.rs".to_string(), "y.rs".to_string()];
    assert_eq!(recall_at_budget(&selected, &gt(&["a.rs", "b.rs"])), 0.0);
}

#[test]
fn recall_partial_is_fraction() {
    // 1 of 2 ground-truth files retrieved.
    let selected = vec!["a.rs".to_string(), "z.rs".to_string()];
    assert_eq!(recall_at_budget(&selected, &gt(&["a.rs", "b.rs"])), 0.5);
}

#[test]
fn recall_ignores_duplicate_selected() {
    // Set intersection — duplicates in `selected` must not inflate the numerator.
    let selected = vec!["a.rs".to_string(), "a.rs".to_string()];
    assert_eq!(recall_at_budget(&selected, &gt(&["a.rs", "b.rs"])), 0.5);
}

// ── mrr ────────────────────────────────────────────────────────────────────

#[test]
fn mrr_rank_one_is_one() {
    let ranked = vec!["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()];
    assert_eq!(mrr(&ranked, &gt(&["a.rs"])), 1.0);
}

#[test]
fn mrr_rank_three_is_one_third() {
    let ranked = vec!["x.rs".to_string(), "y.rs".to_string(), "a.rs".to_string()];
    let v = mrr(&ranked, &gt(&["a.rs"]));
    assert!((v - 1.0 / 3.0).abs() < 1e-9, "expected 0.333…, got {v}");
}

#[test]
fn mrr_no_hit_is_zero() {
    let ranked = vec!["x.rs".to_string(), "y.rs".to_string()];
    assert_eq!(mrr(&ranked, &gt(&["a.rs"])), 0.0);
}

#[test]
fn mrr_uses_first_hit_only() {
    // First gt match is at rank 2 even though rank 4 also matches → 1/2.
    let ranked = vec![
        "x.rs".to_string(),
        "b.rs".to_string(),
        "y.rs".to_string(),
        "a.rs".to_string(),
    ];
    assert_eq!(mrr(&ranked, &gt(&["a.rs", "b.rs"])), 0.5);
}

#[test]
fn mrr_empty_ground_truth_is_zero() {
    // No relevant item to find → MRR is undefined; define as 0.0.
    let ranked = vec!["a.rs".to_string()];
    assert_eq!(mrr(&ranked, &gt(&[])), 0.0);
}

// ── render_comparison_table ────────────────────────────────────────────────

#[test]
fn table_renders_stable_output() {
    let results = vec![
        SystemResult {
            system: "cxpak".to_string(),
            recall_8k: 0.6,
            recall_32k: 0.8,
            mrr: 0.5,
        },
        SystemResult {
            system: "ripgrep".to_string(),
            recall_8k: 0.4,
            recall_32k: 0.5,
            mrr: 0.3,
        },
    ];

    let table = render_comparison_table(&results);

    // Header + both rows present, deterministic column order.
    assert!(
        table.contains("recall@8k"),
        "missing recall@8k header:\n{table}"
    );
    assert!(
        table.contains("recall@32k"),
        "missing recall@32k header:\n{table}"
    );
    assert!(table.contains("MRR"), "missing MRR header:\n{table}");
    assert!(table.contains("cxpak"), "missing cxpak row:\n{table}");
    assert!(table.contains("ripgrep"), "missing ripgrep row:\n{table}");
    // Rendered in input order (caller controls ordering).
    let cx = table.find("cxpak").unwrap();
    let rg = table.find("ripgrep").unwrap();
    assert!(cx < rg, "rows not in input order:\n{table}");

    // Byte-for-byte stable across calls.
    assert_eq!(table, render_comparison_table(&results));
}

#[test]
fn table_handles_empty_results() {
    let table = render_comparison_table(&[]);
    // Still emits a header so the output is never blank.
    assert!(
        table.contains("recall@8k"),
        "empty table must keep header:\n{table}"
    );
}

// ── ground_truth_at_base (NO network — the base-tree nuance) ───────────────
//
// Ground truth = changed files that EXIST at base_sha. Retrieval runs over the
// base tree, so `added` files (which don't exist there) must be excluded;
// `renamed` files exist under their PREVIOUS path; everything else exists under
// its current path. These pure cases are the most error-prone part of the
// metric, so they get explicit coverage in the default-with-bench suite.

#[test]
fn gt_added_is_excluded() {
    // An added file does not exist at base — it cannot be retrieved, so it must
    // not appear in (or inflate) the ground-truth denominator.
    let result = ground_truth_at_base(&[cf("src/new.rs", "added", None)]);
    assert!(
        result.is_empty(),
        "added file must be excluded, got {result:?}"
    );
}

#[test]
fn gt_modified_is_kept() {
    let result = ground_truth_at_base(&[cf("src/lib.rs", "modified", None)]);
    assert_eq!(result, gt(&["src/lib.rs"]));
}

#[test]
fn gt_removed_is_kept() {
    // A removed/deleted file still exists at base (the PR deletes it).
    let result = ground_truth_at_base(&[cf("src/old.rs", "removed", None)]);
    assert_eq!(result, gt(&["src/old.rs"]));
}

#[test]
fn gt_renamed_uses_previous_filename() {
    // The base-tree path is the PRE-rename path; the post-rename name does not
    // exist at base, so it must NOT be in the ground truth.
    let result = ground_truth_at_base(&[cf("src/new_name.rs", "renamed", Some("src/old_name.rs"))]);
    assert_eq!(result, gt(&["src/old_name.rs"]));
    assert!(
        !result.contains("src/new_name.rs"),
        "post-rename name must not be in gt: {result:?}"
    );
}

#[test]
fn gt_renamed_without_previous_falls_back_to_filename() {
    // Defensive: GitHub always supplies previous_filename for a rename, but if
    // it is absent we fall back to the current filename rather than dropping it.
    let result = ground_truth_at_base(&[cf("src/renamed.rs", "renamed", None)]);
    assert_eq!(result, gt(&["src/renamed.rs"]));
}

#[test]
fn gt_mixed_statuses() {
    // A realistic mix: added excluded; modified/removed kept by name; renamed
    // kept by previous path.
    let result = ground_truth_at_base(&[
        cf("a_added.rs", "added", None),
        cf("b_modified.rs", "modified", None),
        cf("c_removed.rs", "removed", None),
        cf("d_new.rs", "renamed", Some("d_old.rs")),
    ]);
    assert_eq!(
        result,
        gt(&["b_modified.rs", "c_removed.rs", "d_old.rs"]),
        "mixed-status ground truth incorrect"
    );
}

// ── Harness smoke (NETWORK + compute, gated) ───────────────────────────────
//
// Runs the FULL pipeline (fetch repo@base_sha → index → all systems at {8k,32k})
// on a small subset, asserting a complete, well-formed comparison table.
//
// Double-gated: `#[ignore]` (so `cargo test` skips it) AND a `CXPAK_BENCH_NET`
// env check (so even `--ignored` is a no-op without explicit opt-in). Run with:
//   CXPAK_BENCH_NET=1 cargo test --features bench harness_smoke -- --ignored --nocapture

#[test]
#[ignore = "network + compute — run with: CXPAK_BENCH_NET=1 cargo test --features bench -- --ignored"]
fn harness_smoke_subset() {
    if std::env::var("CXPAK_BENCH_NET").is_err() {
        eprintln!("CXPAK_BENCH_NET unset — skipping network harness smoke");
        return;
    }

    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let corpus = cxpak::bench::load_corpus(repo_root).expect("corpus loads");

    // Subset: take up to `n` entries spread across DISTINCT repos (one per repo,
    // in corpus order) so the smoke exercises multiple languages rather than
    // hammering a single repo. `n` defaults to 2 (fast smoke) and is overridable
    // via CXPAK_BENCH_N for a broader baseline run.
    let n: usize = std::env::var("CXPAK_BENCH_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    let mut seen_repos = HashSet::new();
    let subset: Vec<_> = corpus
        .into_iter()
        .filter(|e| seen_repos.insert(e.repo.clone()))
        .take(n)
        .collect();

    let results =
        cxpak::bench::recall::run_harness(&subset, repo_root).expect("harness runs end-to-end");

    // Every system must appear.
    let systems: HashSet<&str> = results.iter().map(|r| r.system.as_str()).collect();
    for expected in [
        "cxpak (auto_context)",
        "cxpak (score_all ranking)",
        "ripgrep",
        "embeddings-only",
        "repomap (PageRank proxy)",
    ] {
        assert!(
            systems.contains(expected),
            "missing system '{expected}' in results: {systems:?}"
        );
    }

    // All metrics finite and in [0,1].
    for r in &results {
        for (name, v) in [
            ("recall@8k", r.recall_8k),
            ("recall@32k", r.recall_32k),
            ("MRR", r.mrr),
        ] {
            assert!(
                v.is_finite() && (0.0..=1.0).contains(&v),
                "{} {} out of range: {}",
                r.system,
                name,
                v
            );
        }
    }

    // Table renders with all rows.
    let table = render_comparison_table(&results);
    println!("\n{table}\n");
    assert!(table.contains("cxpak"));
}
