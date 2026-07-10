// D2.3 — CI recall-regression gate (integration surface).
//
// Three layers, matching the gating discipline of `bench_recall.rs`:
//   1. No-network baseline integrity — the committed `bench/baseline.json`
//      parses, has the expected format version, a non-empty pinned subset whose
//      every entry resolves against `corpus.toml`, and a gate row. Runs in the
//      default `cargo test --features bench` invocation; the gate's committed
//      artifact can never silently rot.
//   2. Network gate (`#[ignore]` + `CXPAK_BENCH_NET`) — runs the harness on the
//      pinned subset and asserts cxpak (auto_context) recall has NOT regressed
//      below baseline. This is the job whose exit status CI keys on.
//   3. Baseline generation (`#[ignore]` + `CXPAK_BENCH_GEN`) — regenerates
//      `bench/baseline.json` from a fresh harness run. Run manually when the
//      subset or measurement changes; never in CI.
//
// Requires the `bench` feature; the pure comparison logic (regression →
// fail, equal/better → pass, MRR-drop → pass) is unit-tested inside
// `src/bench/gate.rs`.
#![cfg(feature = "bench")]

use cxpak::bench::gate::{
    compare, default_subset, generate_baseline, load_baseline, run_gate, select_subset,
    BaselineSystem, BASELINE_FORMAT_VERSION, GATE_SYSTEM, RECALL_TOLERANCE,
};

fn repo_root() -> &'static std::path::Path {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
}

// ── Layer 1: committed baseline integrity (NO network — always runs) ────────

#[test]
fn committed_baseline_loads_and_validates() {
    // load_baseline enforces: file present, parses, format_version matches,
    // subset non-empty, gate row present. A green run here means the committed
    // gate artifact is well-formed.
    let baseline = load_baseline(repo_root()).expect("committed bench/baseline.json must load");
    assert_eq!(baseline.format_version, BASELINE_FORMAT_VERSION);
    assert!(!baseline.subset.is_empty(), "subset must be non-empty");

    let gate = baseline
        .gate_row()
        .expect("baseline must carry the gate row");
    assert_eq!(gate.system, GATE_SYSTEM);
    // The recorded gate metrics must be finite and in range — a corrupt NaN/out
    // -of-range baseline would make every later comparison meaningless.
    for (name, v) in [
        ("recall@8k", gate.recall_8k),
        ("recall@32k", gate.recall_32k),
        ("MRR", gate.mrr),
    ] {
        assert!(
            v.is_finite() && (0.0..=1.0).contains(&v),
            "baseline {name} out of range: {v}"
        );
    }
}

#[test]
fn committed_baseline_subset_resolves_against_corpus() {
    // Every pinned (repo, pr) in the committed baseline must exist in the corpus
    // — otherwise the gate would run on a silently shrunken subset.
    let baseline = load_baseline(repo_root()).expect("baseline loads");
    let corpus = cxpak::bench::load_corpus(repo_root()).expect("corpus loads");
    let resolved = select_subset(&baseline.subset, &corpus)
        .expect("every pinned subset entry must resolve against corpus.toml");
    assert_eq!(
        resolved.len(),
        baseline.subset.len(),
        "all pinned entries must resolve"
    );
    // Resolved entries carry full 40-hex SHAs (immutability → reproducibility).
    for e in &resolved {
        assert_eq!(
            e.base_sha.len(),
            40,
            "{}#{} base_sha not 40-hex",
            e.repo,
            e.pr
        );
    }
}

#[test]
fn committed_baseline_subset_matches_default() {
    // The committed subset should match the code's fixed `default_subset()` so
    // the gate documents exactly which PRs it locks in. (Regenerating with a
    // different subset is a deliberate, reviewed change to both.)
    let baseline = load_baseline(repo_root()).expect("baseline loads");
    assert_eq!(
        baseline.subset,
        default_subset(),
        "committed subset drifted from default_subset() — regenerate the baseline"
    );
}

#[test]
fn committed_baseline_carries_informational_systems() {
    // Beyond the gate row, the baseline records the other systems so reviewers
    // can read the cxpak-vs-baseline delta straight from the file.
    let baseline = load_baseline(repo_root()).expect("baseline loads");
    let names: Vec<&str> = baseline.systems.iter().map(|s| s.system.as_str()).collect();
    for expected in [
        GATE_SYSTEM,
        "cxpak (score_all ranking)",
        "ripgrep",
        "embeddings-only",
        "repomap (PageRank proxy)",
    ] {
        assert!(
            names.contains(&expected),
            "baseline missing system '{expected}': {names:?}"
        );
    }
}

// Mirrors the gate's compare() contract at the integration boundary against the
// REAL committed baseline row: comparing it to itself must pass. (Exhaustive
// regression/MRR cases live in src/bench/gate.rs unit tests.)
#[test]
fn committed_baseline_compares_clean_against_itself() {
    let baseline = load_baseline(repo_root()).expect("baseline loads");
    let gate = baseline.gate_row().expect("gate row");
    let same = BaselineSystem {
        system: gate.system.clone(),
        recall_8k: gate.recall_8k,
        recall_32k: gate.recall_32k,
        mrr: gate.mrr,
    };
    let outcome = compare(gate, &same, RECALL_TOLERANCE);
    assert!(
        outcome.passed,
        "baseline compared to itself must pass:\n{}",
        outcome.render()
    );
}

// ── Layer 2: network recall gate (gated — CI keys on this) ──────────────────
//
// Runs the harness on the pinned subset and asserts non-regression. Double-
// gated: `#[ignore]` (cargo test skips it) AND a CXPAK_BENCH_NET check. In CI
// the bench job sets CXPAK_BENCH_NET=1 and GH_TOKEN, so this DOES run with teeth
// on this repo's branches/PRs. Run locally with:
//   CXPAK_BENCH_NET=1 cargo test --features bench --test bench_gate \
//     recall_gate_holds -- --ignored --nocapture

#[test]
#[ignore = "network + compute — run with: CXPAK_BENCH_NET=1 cargo test --features bench --test bench_gate -- --ignored"]
fn recall_gate_holds() {
    if std::env::var("CXPAK_BENCH_NET").is_err() {
        // Opt-out path for `--ignored` runs without the env (e.g. a developer
        // running all ignored tests offline). CI ALWAYS sets it, so the gate has
        // teeth there. This is the documented graceful-skip, not a silent pass
        // of a failing gate: with the env unset we never claim the gate ran.
        eprintln!("CXPAK_BENCH_NET unset — skipping network recall gate");
        return;
    }

    let (outcome, results) = run_gate(repo_root()).expect("gate harness runs end-to-end");

    // Print the full table + the gate verdict for the CI log.
    println!(
        "\n{}\n{}",
        cxpak::bench::recall::render_comparison_table(&results),
        outcome.render()
    );

    assert!(
        outcome.passed,
        "RECALL REGRESSION: cxpak (auto_context) recall fell below baseline.\n{}",
        outcome.render()
    );
}

// ── Layer 3: baseline (re)generation (gated — manual only) ──────────────────
//
// Regenerates bench/baseline.json from a fresh harness run on default_subset().
// Triple-gated so it never runs in CI: `#[ignore]` + CXPAK_BENCH_NET (network) +
// CXPAK_BENCH_GEN (explicit write opt-in). Run with:
//   CXPAK_BENCH_NET=1 CXPAK_BENCH_GEN=1 cargo test --features bench \
//     --test bench_gate regenerate_baseline -- --ignored --nocapture

#[test]
#[ignore = "network + writes bench/baseline.json — run with CXPAK_BENCH_NET=1 CXPAK_BENCH_GEN=1 ... --ignored"]
fn regenerate_baseline() {
    if std::env::var("CXPAK_BENCH_NET").is_err() || std::env::var("CXPAK_BENCH_GEN").is_err() {
        eprintln!("CXPAK_BENCH_NET and CXPAK_BENCH_GEN both required — skipping baseline regen");
        return;
    }

    let note = format!(
        "Generated by tests/bench_gate.rs::regenerate_baseline on {} from default_subset() \
         (3 small-repo PRs). cxpak (auto_context) recall@{{8k,32k}} is the gated row; other \
         systems informational. See ADR-0172.",
        chrono_like_utc_date()
    );
    let baseline = generate_baseline(default_subset(), repo_root(), note)
        .expect("baseline generation runs end-to-end");

    let path = repo_root().join("bench/baseline.json");
    let mut json = serde_json::to_string_pretty(&baseline).expect("serialize baseline");
    json.push('\n');
    std::fs::write(&path, json).expect("write bench/baseline.json");

    println!("\nwrote {}\n", path.display());
    println!(
        "{}",
        cxpak::bench::recall::render_comparison_table(
            &baseline
                .systems
                .iter()
                .map(|s| cxpak::bench::recall::SystemResult {
                    system: s.system.clone(),
                    recall_8k: s.recall_8k,
                    recall_32k: s.recall_32k,
                    mrr: s.mrr,
                })
                .collect::<Vec<_>>()
        )
    );
}

/// Minimal UTC date string (YYYY-MM-DD) for the provenance note, via the `date`
/// CLI so no new dependency is pulled in. Falls back to a stable placeholder if
/// `date` is unavailable (the note is documentation, not gate logic).
fn chrono_like_utc_date() -> String {
    std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%d"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown-date".to_string())
}
