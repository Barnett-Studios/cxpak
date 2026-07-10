//! Reciprocal Rank Fusion (RRF) over the multi-signal sub-rankings
//! (cxpak 3.0.0, Phase D — ADR-0184).
//!
//! The inert combine (`MultiSignalScorer` weighted sum) mixes seven signals on
//! their raw magnitudes: a signal that happens to produce large absolute values
//! for a query dominates one that separates files just as well but on a smaller
//! numeric range. RRF sidesteps that by fusing the *ranks* each signal induces
//! rather than its magnitudes, so every signal contributes on the same footing.
//!
//! ## Formula
//!
//! For file `i` and signal `j`, let `rank_ij` be `i`'s **competition rank** when
//! the files are ordered by signal `j` descending: `1 + (files with a strictly
//! greater score)`, so all files tied on a signal share one rank. The fused
//! score is
//!
//! ```text
//! rrf_i = Σ_j  weights[j] · (K + 1) / (K + rank_ij)
//! ```
//!
//! with `K = `[`RRF_K`]` = 60` (the standard constant from Cormack, Clarke &
//! Büttcher, SIGIR 2009, "Reciprocal Rank Fusion outperforms Condorcet and
//! individual Rank Learning Methods"). `K` damps the pull of any single signal's
//! very top ranks so no one signal can dominate the fused order.
//!
//! ## Composition with the weighted sum
//!
//! Two design choices make RRF a drop-in *replacement* for the inert weighted
//! sum rather than a bolt-on:
//!
//!  1. **Weighted RRF.** Each signal's reciprocal-rank term is multiplied by the
//!     SAME tuned weight the inert sum uses. Plain unweighted RRF would discard
//!     the hand-tuned signal weights (symbol_match 0.27, pagerank 0.15, …); the
//!     weighted form keeps them meaningful — a heavily-weighted signal's ranking
//!     still counts for more.
//!  2. **Scale normalization via `(K + 1)`.** A rank-1 file contributes exactly
//!     `weights[j]` (since `(K+1)/(K+1) = 1`), so each term lands in `(0, 1]`.
//!     With `Σ weights = 1` the fused score lands on the SAME `[0, 1]` scale as
//!     the weighted sum. This keeps fused scores in-range so seed selection
//!     cannot COLLAPSE — an un-normalized `Σ 1/(K+rank) ≈ 0.016`-max score would
//!     fall entirely below [`crate::relevance::seed::SEED_THRESHOLD`] = 0.10 and
//!     empty the seed set. Note the normalization does NOT preserve threshold
//!     *discrimination*: the RRF worst-file floor `(K+1)/(K+n) ≈ 0.17–0.24` for
//!     `n ≳ 200` exceeds `SEED_THRESHOLD`, so on large repos Active-mode
//!     threshold filtering degenerates to the top-50 seed cap rather than the
//!     0.10 cutoff. This degeneration is MEASURED and net-positive: the
//!     full-corpus recall A/B (ADR-0187) puts Active at +164% recall over Inert
//!     at both budgets, with only 2 isolated flask regressions.
//!
//! ## Determinism and tie handling
//!
//! Rank assignment sorts file indices by `(signal_score desc via
//! `f64::total_cmp`, path asc)`, then assigns **competition ranks** so files
//! tied on a signal share a rank. The path-asc key fixes only a deterministic
//! iteration order; it does NOT feed the rank, so the fused score is fully
//! path-independent. A weight-0 signal is skipped entirely (it cannot change the
//! fusion), which keeps the embeddings-off path (embedding weight 0.0) free of
//! any noise from the constant neutral embedding scores.
//!
//! Competition ranking (not ordinal) is what makes a *uniform* signal inert: if
//! every file scores the same on signal `j`, all files get rank 1 and each
//! receives the identical constant `w_j·(K+1)/(K+1) = w_j`, so signal `j` cannot
//! reorder anything. The earlier ordinal scheme (distinct rank per sorted
//! position) instead spread a uniform signal's files across ranks `1..n` by path
//! and leaked its full weight `w_j` as an alphabetical gradient — enough to
//! overturn a genuinely informative signal (ADR-0190). The deterministic
//! path-asc tiebreak that resolves equal *fused* scores now lives solely in seed
//! selection (`(score desc, path asc)`, ADR-0188), not in the fusion itself.

/// The RRF constant `K`. Standard value from the RRF paper; larger `K` flattens
/// the reciprocal-rank curve (top ranks matter less), smaller `K` sharpens it.
pub const RRF_K: f64 = 60.0;

/// Number of fused signals, in the fixed order used throughout the scorer:
/// `[path_similarity, symbol_match, import_proximity, term_frequency,
///   recency_boost, pagerank, embedding_similarity]`.
pub const N_SIGNALS: usize = 7;

/// Compute the weighted, scale-normalized RRF score for every file.
///
/// `files[i] = (path_i, signal_scores_i)` where `signal_scores_i[j]` is file
/// `i`'s raw score for signal `j` (fixed order, see [`N_SIGNALS`]). `weights[j]`
/// is signal `j`'s tuned weight (the same weights the inert weighted sum uses).
///
/// Returns one fused score per file, aligned to the input order. See the module
/// docs for the formula, composition rationale, and determinism guarantees.
pub fn fuse(files: &[(String, [f64; N_SIGNALS])], weights: [f64; N_SIGNALS], k: f64) -> Vec<f64> {
    let n = files.len();
    let mut rrf = vec![0.0_f64; n];
    if n == 0 {
        return rrf;
    }

    for (j, &w) in weights.iter().enumerate() {
        // A weight-0 signal contributes `0 · (K+1)/(K+rank) = 0` for every file,
        // so it cannot affect the fusion — skip it (and, importantly, keep its
        // arbitrary tie order from leaking path bias into the fused scores).
        if w == 0.0 {
            continue;
        }

        // Order file indices by this signal's score descending. The path-asc
        // secondary key fixes only a deterministic *iteration* order; it does
        // NOT influence the rank a file receives (ties share a rank below), so a
        // uniform signal cannot leak alphabetical path bias into the fused score.
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| {
            files[b].1[j]
                .total_cmp(&files[a].1[j])
                .then_with(|| files[a].0.cmp(&files[b].0))
        });

        // Competition ("1224") ranking: every file tied on this signal's score
        // shares the rank `1 + (files with a strictly greater score)`, which for
        // the first member of a tie group at sorted position `i` is `i + 1`. A
        // signal uniform across all files therefore gives every file rank 1 and
        // contributes `w · (K+1)/(K+1) = w` to each — a constant that cannot
        // reorder anything. Ordinal ranking (distinct rank per position) instead
        // leaked a uniform signal's full weight as a path-order gradient.
        let mut i = 0;
        while i < n {
            let score = files[order[i]].1[j];
            let mut end = i;
            while end < n && files[order[end]].1[j].total_cmp(&score) == std::cmp::Ordering::Equal {
                end += 1;
            }
            let rank = i as f64 + 1.0; // competition rank shared by the tie group
            let contribution = w * (k + 1.0) / (k + rank);
            for &idx in &order[i..end] {
                rrf[idx] += contribution;
            }
            i = end;
        }
    }

    rrf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eq(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    /// Equal-weight fusion: a file ranked high by TWO signals must beat a file
    /// ranked high by only ONE signal.
    #[test]
    fn high_in_two_signals_beats_high_in_one() {
        // Signal scores per file, 7 signals. Only the first two signals vary.
        // file "a": top of signal 0 AND signal 1.
        // file "b": top of signal 0 only; bottom of signal 1.
        // file "c": filler.
        let mk = |s0: f64, s1: f64| {
            let mut v = [0.0; N_SIGNALS];
            v[0] = s0;
            v[1] = s1;
            v
        };
        let files = vec![
            ("a".to_string(), mk(0.9, 0.9)),
            ("b".to_string(), mk(0.8, 0.1)),
            ("c".to_string(), mk(0.1, 0.8)),
        ];
        let mut weights = [0.0; N_SIGNALS];
        weights[0] = 0.5;
        weights[1] = 0.5;

        let scores = fuse(&files, weights, RRF_K);
        // a is rank-1 in both signals → highest fused score.
        assert!(
            scores[0] > scores[1],
            "a ({}) should beat b ({})",
            scores[0],
            scores[1]
        );
        assert!(
            scores[0] > scores[2],
            "a ({}) should beat c ({})",
            scores[0],
            scores[2]
        );
    }

    /// A rank-1-in-every-signal file with weights summing to 1 scores ~1.0 —
    /// proof the `(K+1)` normalization lands on the weighted sum's `[0,1]` scale.
    #[test]
    fn rank_one_everywhere_scores_near_one() {
        let mut weights = [0.0; N_SIGNALS];
        weights[0] = 0.6;
        weights[1] = 0.4; // Σ = 1.0

        // "top" is strictly highest in both weighted signals; "other" is lower.
        let files = vec![
            ("a_top".to_string(), {
                let mut v = [0.0; N_SIGNALS];
                v[0] = 1.0;
                v[1] = 1.0;
                v
            }),
            ("b_other".to_string(), {
                let mut v = [0.0; N_SIGNALS];
                v[0] = 0.5;
                v[1] = 0.5;
                v
            }),
        ];
        let scores = fuse(&files, weights, RRF_K);
        // rank-1 in both weighted signals ⇒ Σ weights · 1 = 1.0 exactly.
        assert!(eq(scores[0], 1.0), "expected ~1.0, got {}", scores[0]);
        assert!(scores[0] <= 1.0 + 1e-12, "must not exceed 1.0 scale");
    }

    /// Weight-0 signals are inert: they never change the fused score, so the
    /// embeddings-off path (embedding weight 0.0) carries no path-order noise
    /// from the constant neutral embedding scores.
    #[test]
    fn zero_weight_signal_is_inert() {
        // Two files that TIE on signal 6 (the embedding slot), differ on signal 0.
        let files = vec![
            ("z_last".to_string(), {
                let mut v = [0.0; N_SIGNALS];
                v[0] = 0.9;
                v[6] = 0.5; // tie
                v
            }),
            ("a_first".to_string(), {
                let mut v = [0.0; N_SIGNALS];
                v[0] = 0.1;
                v[6] = 0.5; // tie
                v
            }),
        ];
        let mut w_with = [0.0; N_SIGNALS];
        w_with[0] = 1.0; // only signal 0 matters
        let scores_a = fuse(&files, w_with, RRF_K);

        // The weight-0 case must be identical to omitting signal 6 entirely:
        // a skipped signal contributes nothing, so only signal 0 shapes the order.
        let mut w_zero6 = [0.0; N_SIGNALS];
        w_zero6[0] = 1.0;
        w_zero6[6] = 0.0;
        let scores_b = fuse(&files, w_zero6, RRF_K);
        assert!(eq(scores_a[0], scores_b[0]));
        assert!(eq(scores_a[1], scores_b[1]));
    }

    /// Path-independent fusion: two files with byte-identical signal vectors
    /// receive the SAME fused score (competition ranking → both share rank 1),
    /// regardless of path order, and the fusion is reproducible across runs. The
    /// deterministic path-asc tiebreak that resolves equal fused scores lives in
    /// seed selection (ADR-0188), not in the fusion.
    #[test]
    fn identical_vectors_get_equal_scores() {
        let mk = || {
            let mut v = [0.0; N_SIGNALS];
            v[0] = 0.5;
            v
        };
        let files = vec![("b.rs".to_string(), mk()), ("a.rs".to_string(), mk())];
        let mut weights = [0.0; N_SIGNALS];
        weights[0] = 1.0;

        let run1 = fuse(&files, weights, RRF_K);
        let run2 = fuse(&files, weights, RRF_K);
        assert_eq!(run1, run2, "fusion must be reproducible");
        assert!(
            eq(run1[0], run1[1]),
            "identical signal vectors must fuse to equal scores (no path bias): {run1:?}"
        );
    }

    /// The C1 defect (ADR-0190): a uniform (non-discriminating) weighted signal
    /// must not overturn a genuinely informative one. Under the old ordinal
    /// ranking the all-zero signal below spread files by path and exactly
    /// canceled signal 0, tying the strictly-better file with the worse one (and,
    /// with more uniform weight, letting the worse file win by filename).
    /// Competition ranking gives every file rank 1 on a uniform signal, so it
    /// adds an identical constant and cannot reorder.
    #[test]
    fn uniform_signal_cannot_overturn_informative_signal() {
        // signal 0 is informative (zzz strictly beats aaa); signal 1 is uniform.
        let mk = |s0: f64| {
            let mut v = [0.0; N_SIGNALS];
            v[0] = s0;
            v[1] = 0.0; // uniform across both files
            v
        };
        // "zzz_winner" sorts AFTER "aaa_loser" — the worst case for path bias.
        let files = vec![
            ("zzz_winner".to_string(), mk(0.9)),
            ("aaa_loser".to_string(), mk(0.1)),
        ];
        let mut weights = [0.0; N_SIGNALS];
        weights[0] = 0.5;
        weights[1] = 0.5; // equal weight to the uniform signal — the cancellation case

        let scores = fuse(&files, weights, RRF_K);
        assert!(
            scores[0] > scores[1],
            "informative winner ({}) must strictly beat the path-favored loser ({})",
            scores[0],
            scores[1]
        );
    }

    #[test]
    fn empty_input_yields_empty() {
        let files: Vec<(String, [f64; N_SIGNALS])> = vec![];
        let mut weights = [0.0; N_SIGNALS];
        weights[0] = 1.0;
        assert!(fuse(&files, weights, RRF_K).is_empty());
    }
}
