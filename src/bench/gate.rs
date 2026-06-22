// D2.3 — CI recall-regression gate.
//
// Locks in cxpak's measured retrieval recall so no later change (the C2 ranking
// work, the D1 semantic work, or any refactor) can silently reduce it. The gate
// runs the recall harness on a FIXED, resource-safe subset of the corpus, reads
// back the shipped product's row — `cxpak (auto_context)` — and compares its
// recall@{8k,32k} against a committed baseline snapshot.
//
// Layering mirrors `recall.rs`:
//   * Pure comparison core — `Baseline`, `load_baseline`, `compare`,
//     `GateOutcome`. No I/O, no network; unit-tested in the default
//     `cargo test --features bench` run.
//   * Gated runner — `run_gate`. Drives the network harness on the fixed
//     subset, then calls `compare`. Only invoked from the `#[ignore]`d,
//     `CXPAK_BENCH_NET`-gated integration test (`tests/bench_gate.rs`).
//
// Gate POLICY (see ADR-0172 for the rationale):
//   * PRIMARY hard-fail: `cxpak (auto_context)` recall@{8k,32k} must be
//     >= baseline - TOLERANCE. This is a NON-REGRESSION gate.
//   * MRR is tracked and printed but NOT hard-gated — cxpak currently trails the
//     baselines on MRR (a known weakness D1/C2 will address); gating it now would
//     block the branch on a metric we are about to improve.
//   * The baseline subset is PINNED by (repo, pr): merged PRs are immutable, so
//     the measured numbers are reproducible run-to-run.

use crate::bench::recall::{run_harness_counted, SystemResult};
use crate::bench::CorpusEntry;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The row whose recall the gate locks in: the shipped product, end-to-end.
pub const GATE_SYSTEM: &str = "cxpak (auto_context)";

/// Allowed recall slippage below the committed baseline before the gate fails.
///
/// The subset is pinned to immutable merged PRs and the harness is
/// deterministic, so a faithful re-run reproduces the baseline EXACTLY; a strict
/// `0.0` tolerance would therefore be defensible. We keep a tiny epsilon to
/// absorb last-bit floating-point drift from averaging across entries on a
/// different host/CPU (the mean is a sum of ratios divided by N), so the gate
/// fails on a *real* recall regression rather than a `1e-15` rounding wobble.
/// Any drop larger than this is a genuine retrieval regression and fails.
pub const RECALL_TOLERANCE: f64 = 1e-6;

/// Format version of the committed `bench/baseline.json`. Bump on any breaking
/// change to the schema or the pinned subset so a stale baseline is rejected
/// loudly rather than compared against silently.
pub const BASELINE_FORMAT_VERSION: u32 = 1;

/// One pinned corpus entry in the gate subset, identified by `(repo, pr)`.
///
/// Only the identity is stored; the full `base_sha`/`head_sha`/`lang`/`title`
/// live in `bench/corpus.toml`, which `select_subset` joins against. Keeping the
/// baseline's subset list to bare identities means the corpus stays the single
/// source of truth for SHAs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsetEntry {
    pub repo: String,
    pub pr: u32,
}

/// A system's recorded metrics in the baseline snapshot (mirrors `SystemResult`,
/// but owned/serializable and decoupled from the harness type).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineSystem {
    pub system: String,
    pub recall_8k: f64,
    pub recall_32k: f64,
    pub mrr: f64,
}

impl From<&SystemResult> for BaselineSystem {
    fn from(r: &SystemResult) -> Self {
        BaselineSystem {
            system: r.system.clone(),
            recall_8k: r.recall_8k,
            recall_32k: r.recall_32k,
            mrr: r.mrr,
        }
    }
}

/// The committed baseline snapshot: the pinned subset + every system's numbers.
///
/// `systems` carries ALL rows (gate-relevant + informational baselines) so a
/// reviewer can read the cxpak-vs-baseline delta straight from the file; only
/// the [`GATE_SYSTEM`] row is hard-gated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    /// Schema/subset version — see [`BASELINE_FORMAT_VERSION`].
    pub format_version: u32,
    /// Human note on how/when the numbers were produced (provenance).
    pub generated_note: String,
    /// The exact pinned `(repo, pr)` subset the harness was run on.
    pub subset: Vec<SubsetEntry>,
    /// Per-system mean metrics over the subset, in harness row order.
    pub systems: Vec<BaselineSystem>,
}

impl Baseline {
    /// The gate-relevant row (`cxpak (auto_context)`), if present.
    pub fn gate_row(&self) -> Option<&BaselineSystem> {
        self.systems.iter().find(|s| s.system == GATE_SYSTEM)
    }
}

/// Load and validate the committed baseline from `bench/baseline.json` under
/// `repo_root`.
///
/// Fails loudly on a missing file, a parse error, an unexpected
/// `format_version`, an empty subset, or a missing gate row — a malformed
/// baseline must never degrade into a vacuous pass.
pub fn load_baseline(repo_root: &Path) -> Result<Baseline, String> {
    let path = repo_root.join("bench/baseline.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    let baseline: Baseline = serde_json::from_str(&text)
        .map_err(|e| format!("parse error in {}: {}", path.display(), e))?;

    if baseline.format_version != BASELINE_FORMAT_VERSION {
        return Err(format!(
            "{}: format_version {} != expected {} (regenerate the baseline)",
            path.display(),
            baseline.format_version,
            BASELINE_FORMAT_VERSION
        ));
    }
    if baseline.subset.is_empty() {
        return Err(format!("{}: baseline subset is empty", path.display()));
    }
    if baseline.gate_row().is_none() {
        return Err(format!(
            "{}: baseline has no '{}' row to gate against",
            path.display(),
            GATE_SYSTEM
        ));
    }
    Ok(baseline)
}

/// Resolve the pinned `subset` against the full corpus, returning the matching
/// [`CorpusEntry`] list in subset order.
///
/// Fails if any pinned `(repo, pr)` is absent from the corpus — a baseline that
/// references a dropped corpus entry is stale and must be regenerated, not run
/// against a silently shrunken subset.
pub fn select_subset(
    subset: &[SubsetEntry],
    corpus: &[CorpusEntry],
) -> Result<Vec<CorpusEntry>, String> {
    let mut out = Vec::with_capacity(subset.len());
    for want in subset {
        let found = corpus
            .iter()
            .find(|e| e.repo == want.repo && e.pr == want.pr)
            .ok_or_else(|| {
                format!(
                    "pinned subset entry {}#{} not found in corpus.toml",
                    want.repo, want.pr
                )
            })?;
        out.push(found.clone());
    }
    Ok(out)
}

/// One per-budget recall check against the baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallCheck {
    pub budget_label: String,
    pub baseline: f64,
    pub current: f64,
    /// `current - baseline`. Negative beyond the tolerance is a regression.
    pub delta: f64,
    pub passed: bool,
}

/// The outcome of comparing a harness run against the baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct GateOutcome {
    /// Hard-gated recall checks (one per budget). The gate passes iff every
    /// entry's `passed` is true.
    pub checks: Vec<RecallCheck>,
    /// MRR: baseline vs current — tracked + reported, NOT gated.
    pub mrr_baseline: f64,
    pub mrr_current: f64,
    /// Overall pass/fail: AND of every `checks[i].passed`.
    pub passed: bool,
}

impl GateOutcome {
    /// Human-readable, deterministic summary for CI logs.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "recall-regression gate — system '{GATE_SYSTEM}', tolerance {RECALL_TOLERANCE:.0e}\n"
        ));
        for c in &self.checks {
            out.push_str(&format!(
                "  {}: baseline {:.4}  current {:.4}  delta {:+.4}  [{}]\n",
                c.budget_label,
                c.baseline,
                c.current,
                c.delta,
                if c.passed { "PASS" } else { "FAIL" }
            ));
        }
        out.push_str(&format!(
            "  MRR (tracked, not gated): baseline {:.4}  current {:.4}  delta {:+.4}\n",
            self.mrr_baseline,
            self.mrr_current,
            self.mrr_current - self.mrr_baseline
        ));
        out.push_str(if self.passed {
            "  => GATE PASS\n"
        } else {
            "  => GATE FAIL (recall regressed below baseline - tolerance)\n"
        });
        out
    }
}

/// Guard that a harness run covered EVERY pinned subset entry.
///
/// Pure, no-I/O — the count comparison the gate (and baseline generation) rely
/// on, factored out so it can be unit-tested without the network. `counted` is
/// how many entries actually produced data; `expected` is the pinned subset
/// size. The metrics are means, so `counted < expected` yields a mean over a
/// different denominator than the committed baseline — not comparable. Returns
/// `Err` on any mismatch (`context` names the caller for a clear message),
/// `Ok(())` only when the full subset ran.
pub fn verify_full_subset(counted: usize, expected: usize, context: &str) -> Result<(), String> {
    if counted != expected {
        return Err(format!(
            "{context}: ran {counted} of {expected} subset entries (need all {expected}); \
             a partial-subset mean is not comparable to the full-subset baseline"
        ));
    }
    Ok(())
}

/// Compare a freshly-measured gate row against the baseline gate row.
///
/// Pure: no I/O. PRIMARY gate is recall non-regression at each budget
/// (`current >= baseline - tolerance`); MRR is recorded for reporting but never
/// affects `passed`. This is the function the no-network unit test pins.
pub fn compare(baseline: &BaselineSystem, current: &BaselineSystem, tolerance: f64) -> GateOutcome {
    let check = |label: &str, base: f64, cur: f64| -> RecallCheck {
        let delta = cur - base;
        RecallCheck {
            budget_label: label.to_string(),
            baseline: base,
            current: cur,
            delta,
            // Non-regression: current must not fall more than `tolerance` below
            // baseline. Equal or better always passes; an improvement never
            // fails. MRR is deliberately excluded from this predicate.
            passed: cur >= base - tolerance,
        }
    };

    let checks = vec![
        check("recall@8k", baseline.recall_8k, current.recall_8k),
        check("recall@32k", baseline.recall_32k, current.recall_32k),
    ];
    let passed = checks.iter().all(|c| c.passed);

    GateOutcome {
        checks,
        mrr_baseline: baseline.mrr,
        mrr_current: current.mrr,
        passed,
    }
}

/// The FIXED, resource-safe gate subset: one small-repo PR per language family
/// that D2.2 proved indexes cheaply in CI.
///
/// Deliberately EXCLUDES the repos that blew CI's memory/time budget in D2.2:
/// `spring-projects/spring-boot` (Java — ~7 GB RSS, 100% CPU on index) and
/// `microsoft/TypeScript` (huge embedding-heavy tree). `cli/cli` (Go) is also
/// excluded as the largest of the remaining repos. What's left — ripgrep,
/// flask, express — are small enough that a fetch+index+5-system run over three
/// PRs finishes well inside a default GitHub runner.
///
/// Pinned by `(repo, pr)`: merged PRs are immutable, so these SHAs (resolved via
/// `corpus.toml`) and therefore the measured recall are reproducible.
pub fn default_subset() -> Vec<SubsetEntry> {
    vec![
        SubsetEntry {
            repo: "BurntSushi/ripgrep".to_string(),
            pr: 3420,
        },
        SubsetEntry {
            repo: "pallets/flask".to_string(),
            pr: 5928,
        },
        SubsetEntry {
            repo: "expressjs/express".to_string(),
            pr: 7234,
        },
    ]
}

/// Run the harness on `subset` and assemble a [`Baseline`] from the results.
///
/// Network + compute — used only by the (gated) baseline-generation test to
/// produce `bench/baseline.json` from REAL measured numbers; the committed file
/// is never hand-written.
pub fn generate_baseline(
    subset: Vec<SubsetEntry>,
    repo_root: &Path,
    generated_note: impl Into<String>,
) -> Result<Baseline, String> {
    let corpus = crate::bench::load_corpus(repo_root)?;
    let entries = select_subset(&subset, &corpus)?;
    let (results, counted) = run_harness_counted(&entries, repo_root)?;
    // The committed baseline must be a FULL-subset mean — a baseline averaged
    // over a partial run would bake a wrong denominator into the gate floor.
    verify_full_subset(counted, entries.len(), "baseline generation")?;
    if !results.iter().any(|r| r.system == GATE_SYSTEM) {
        return Err(format!("harness produced no '{GATE_SYSTEM}' row"));
    }
    Ok(Baseline {
        format_version: BASELINE_FORMAT_VERSION,
        generated_note: generated_note.into(),
        subset,
        systems: results.iter().map(BaselineSystem::from).collect(),
    })
}

/// Run the recall harness on the baseline's pinned subset and compare to the
/// committed baseline. Network + compute — invoked only from the gated test.
///
/// Returns the comparison [`GateOutcome`] plus the full fresh harness rows (so
/// the caller can print the whole table, including the informational baselines).
/// FAILS LOUDLY (Err) if the corpus/baseline can't load, the subset can't be
/// resolved, the harness wipes out, or the gate row is missing from the run —
/// in CI (where the token exists) a network failure must surface as a gate
/// failure, never a silent pass.
pub fn run_gate(repo_root: &Path) -> Result<(GateOutcome, Vec<SystemResult>), String> {
    let baseline = load_baseline(repo_root)?;
    let corpus = crate::bench::load_corpus(repo_root)?;
    let subset = select_subset(&baseline.subset, &corpus)?;

    let (results, counted) = run_harness_counted(&subset, repo_root)?;

    // The baseline metrics are MEANS over the full pinned subset. If any entry
    // silently failed to fetch/index (transient network/index error → the
    // harness's "skipping…" note), `run_harness` would still return Ok with a
    // mean over only the survivors — a different denominator than the baseline.
    // Comparing that partial mean to the full-subset baseline is apples-to-
    // oranges: it can spuriously PASS (masking a real regression) or FAIL
    // (red-lighting a clean branch). Refuse it loudly — a partial run is a gate
    // failure to RUN, not a recall verdict.
    verify_full_subset(counted, subset.len(), "recall gate")?;

    let current = results
        .iter()
        .find(|r| r.system == GATE_SYSTEM)
        .map(BaselineSystem::from)
        .ok_or_else(|| format!("harness produced no '{GATE_SYSTEM}' row"))?;

    // `gate_row()` is guaranteed present — `load_baseline` validated it.
    let base_row = baseline
        .gate_row()
        .expect("load_baseline guarantees a gate row");

    let outcome = compare(base_row, &current, RECALL_TOLERANCE);
    Ok((outcome, results))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys(system: &str, r8: f64, r32: f64, mrr: f64) -> BaselineSystem {
        BaselineSystem {
            system: system.to_string(),
            recall_8k: r8,
            recall_32k: r32,
            mrr,
        }
    }

    #[test]
    fn equal_recall_passes() {
        let base = sys(GATE_SYSTEM, 0.25, 0.40, 0.13);
        let cur = sys(GATE_SYSTEM, 0.25, 0.40, 0.13);
        let out = compare(&base, &cur, RECALL_TOLERANCE);
        assert!(out.passed, "equal recall must pass:\n{}", out.render());
    }

    #[test]
    fn improved_recall_passes() {
        let base = sys(GATE_SYSTEM, 0.25, 0.40, 0.13);
        let cur = sys(GATE_SYSTEM, 0.50, 0.60, 0.20);
        let out = compare(&base, &cur, RECALL_TOLERANCE);
        assert!(out.passed, "improved recall must pass:\n{}", out.render());
        assert!(out.checks[0].delta > 0.0);
    }

    #[test]
    fn recall_regression_at_8k_fails() {
        let base = sys(GATE_SYSTEM, 0.25, 0.40, 0.13);
        // @8k drops well below tolerance; @32k holds.
        let cur = sys(GATE_SYSTEM, 0.10, 0.40, 0.13);
        let out = compare(&base, &cur, RECALL_TOLERANCE);
        assert!(!out.passed, "8k regression must fail:\n{}", out.render());
        assert!(!out.checks[0].passed);
        assert!(out.checks[1].passed, "32k held, should still pass");
    }

    #[test]
    fn recall_regression_at_32k_fails() {
        let base = sys(GATE_SYSTEM, 0.25, 0.40, 0.13);
        let cur = sys(GATE_SYSTEM, 0.25, 0.20, 0.13);
        let out = compare(&base, &cur, RECALL_TOLERANCE);
        assert!(!out.passed, "32k regression must fail:\n{}", out.render());
        assert!(out.checks[0].passed);
        assert!(!out.checks[1].passed);
    }

    #[test]
    fn mrr_drop_alone_does_not_fail() {
        // Recall held exactly; MRR cratered. The gate must STILL pass — MRR is
        // tracked, not gated (the whole point of the policy).
        let base = sys(GATE_SYSTEM, 0.25, 0.40, 0.50);
        let cur = sys(GATE_SYSTEM, 0.25, 0.40, 0.01);
        let out = compare(&base, &cur, RECALL_TOLERANCE);
        assert!(
            out.passed,
            "MRR drop alone must NOT fail the gate:\n{}",
            out.render()
        );
        assert!(out.mrr_current < out.mrr_baseline);
    }

    #[test]
    fn tiny_subtolerance_drop_still_passes() {
        // A drop smaller than the tolerance (float wobble) must not trip the gate.
        let base = sys(GATE_SYSTEM, 0.25, 0.40, 0.13);
        let cur = sys(GATE_SYSTEM, 0.25 - RECALL_TOLERANCE / 2.0, 0.40, 0.13);
        let out = compare(&base, &cur, RECALL_TOLERANCE);
        assert!(
            out.passed,
            "sub-tolerance drop must pass:\n{}",
            out.render()
        );
    }

    #[test]
    fn drop_just_beyond_tolerance_fails() {
        // A drop just past the tolerance IS a regression and must fail — proves
        // the tolerance is a hairline, not a loophole.
        let base = sys(GATE_SYSTEM, 0.25, 0.40, 0.13);
        let cur = sys(GATE_SYSTEM, 0.25 - RECALL_TOLERANCE * 10.0, 0.40, 0.13);
        let out = compare(&base, &cur, RECALL_TOLERANCE);
        assert!(
            !out.passed,
            "drop beyond tolerance must fail:\n{}",
            out.render()
        );
    }

    #[test]
    fn from_system_result_maps_fields() {
        let r = SystemResult {
            system: GATE_SYSTEM.to_string(),
            recall_8k: 0.25,
            recall_32k: 0.40,
            mrr: 0.13,
        };
        let b = BaselineSystem::from(&r);
        assert_eq!(b.system, GATE_SYSTEM);
        assert_eq!(b.recall_8k, 0.25);
        assert_eq!(b.recall_32k, 0.40);
        assert_eq!(b.mrr, 0.13);
    }

    #[test]
    fn gate_row_finds_shipped_product_row() {
        let baseline = Baseline {
            format_version: BASELINE_FORMAT_VERSION,
            generated_note: "test".to_string(),
            subset: vec![SubsetEntry {
                repo: "x/y".to_string(),
                pr: 1,
            }],
            systems: vec![
                sys("ripgrep", 0.1, 0.2, 0.5),
                sys(GATE_SYSTEM, 0.25, 0.40, 0.13),
            ],
        };
        let row = baseline.gate_row().expect("gate row present");
        assert_eq!(row.system, GATE_SYSTEM);
        assert_eq!(row.recall_8k, 0.25);
    }

    #[test]
    fn select_subset_resolves_against_corpus() {
        let corpus = vec![
            CorpusEntry {
                repo: "a/b".to_string(),
                pr: 1,
                base_sha: "sha1".to_string(),
                head_sha: "sha2".to_string(),
                lang: "Rust".to_string(),
                title: None,
            },
            CorpusEntry {
                repo: "a/b".to_string(),
                pr: 2,
                base_sha: "sha3".to_string(),
                head_sha: "sha4".to_string(),
                lang: "Rust".to_string(),
                title: None,
            },
        ];
        let subset = vec![SubsetEntry {
            repo: "a/b".to_string(),
            pr: 2,
        }];
        let resolved = select_subset(&subset, &corpus).expect("resolves");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].pr, 2);
        assert_eq!(resolved[0].base_sha, "sha3");
    }

    #[test]
    fn select_subset_missing_entry_errors() {
        let corpus = vec![CorpusEntry {
            repo: "a/b".to_string(),
            pr: 1,
            base_sha: "sha1".to_string(),
            head_sha: "sha2".to_string(),
            lang: "Rust".to_string(),
            title: None,
        }];
        let subset = vec![SubsetEntry {
            repo: "a/b".to_string(),
            pr: 99,
        }];
        let err = select_subset(&subset, &corpus).expect_err("must error on missing entry");
        assert!(
            err.contains("99"),
            "error should name the missing pr: {err}"
        );
    }

    #[test]
    fn verify_full_subset_passes_on_full_run() {
        // Every pinned entry produced data → Ok.
        assert!(verify_full_subset(3, 3, "recall gate").is_ok());
    }

    #[test]
    fn verify_full_subset_errs_on_partial_run() {
        // A partial run (some entries silently failed) → Err, NOT a pass and NOT
        // a recall-based fail. This is the load-bearing guard: a 2-of-3 mean is
        // not comparable to a 3-entry baseline.
        let err = verify_full_subset(2, 3, "recall gate")
            .expect_err("partial run must Err, not silently compare");
        assert!(
            err.contains("2 of 3"),
            "message should name the counts: {err}"
        );
        assert!(
            err.contains("recall gate"),
            "message should name the caller: {err}"
        );
        assert!(
            err.to_lowercase().contains("not comparable"),
            "message should explain why: {err}"
        );
    }

    #[test]
    fn verify_full_subset_errs_on_total_wipeout() {
        // Zero entries counted (every fetch failed) is also a refusal, not a
        // pass. (run_harness already Errs on a total wipeout before reaching
        // here, but the guard is defensive on its own.)
        assert!(verify_full_subset(0, 3, "recall gate").is_err());
    }

    #[test]
    fn render_outcome_is_deterministic_and_labeled() {
        let base = sys(GATE_SYSTEM, 0.25, 0.40, 0.13);
        let cur = sys(GATE_SYSTEM, 0.25, 0.40, 0.13);
        let out = compare(&base, &cur, RECALL_TOLERANCE);
        let a = out.render();
        let b = out.render();
        assert_eq!(a, b, "render must be deterministic");
        assert!(a.contains("recall@8k"));
        assert!(a.contains("recall@32k"));
        assert!(a.contains("MRR (tracked, not gated)"));
        assert!(a.contains("GATE PASS"));
    }
}
