use crate::conventions::export::{compute_checksum, to_stable_value, ConventionExport};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ConventionDiff {
    pub has_changes: bool,
    pub summary: String,
    pub changed_fields: Vec<String>,
}

pub fn diff_exports(current: &ConventionExport, baseline: &ConventionExport) -> ConventionDiff {
    // Before trusting the checksum fast-path, verify that the stored checksum
    // in `current` matches a freshly-computed value.  This prevents a stale or
    // hand-edited export from being silently treated as "no changes".
    let recomputed = compute_checksum(&current.profile);
    if recomputed == current.checksum && current.checksum == baseline.checksum {
        return ConventionDiff {
            has_changes: false,
            summary: "No convention changes detected.".to_string(),
            changed_fields: Vec::new(),
        };
    }

    // The SAME canonical image the checksum hashes. Comparing raw values here would let the
    // two disagree — checksum equal, field diff not — which is the failure the fast-path
    // above exists to avoid rather than to hide.
    let current_val = to_stable_value(serde_json::to_value(&current.profile).unwrap_or_default());
    let baseline_val = to_stable_value(serde_json::to_value(&baseline.profile).unwrap_or_default());

    let mut changed = Vec::new();
    if let (serde_json::Value::Object(cur), serde_json::Value::Object(base)) =
        (current_val, baseline_val)
    {
        for (key, cur_val) in &cur {
            let base_val = base.get(key);
            if base_val != Some(cur_val) {
                changed.push(key.clone());
            }
        }
        for key in base.keys() {
            if !cur.contains_key(key) {
                changed.push(key.clone());
            }
        }
    }

    changed.sort();
    changed.dedup();

    let summary = if changed.is_empty() {
        // NOT "generated_at changed": the timestamp is excluded from the hash by construction
        // — `compute_checksum` hashes the profile only — so naming it sent every reader of
        // this line to the one cause that cannot produce it (cxpak#70). What remains, now
        // that both sides are canonicalized, is a checksum that does not describe the
        // profile beside it: a hand-edited or stale export, or one written by a different
        // cxpak whose canonical form differs.
        "Checksum differs but every profile field is identical — the stored checksum does \
         not describe this profile. The export is stale, hand-edited, or written by a \
         different cxpak version; re-run `cxpak conventions export`."
            .to_string()
    } else {
        format!(
            "{} convention category(s) changed: {}",
            changed.len(),
            changed.join(", ")
        )
    };

    ConventionDiff {
        has_changes: true,
        summary,
        changed_fields: changed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conventions::export::build_export;
    use crate::conventions::ConventionProfile;

    #[test]
    fn diff_identical_exports_is_empty() {
        let profile = ConventionProfile::default();
        let a = build_export("repo", profile);
        let diff = diff_exports(&a, &a);
        assert!(!diff.has_changes);
        assert!(diff.changed_fields.is_empty());
    }

    /// cxpak#70, end to end at this layer: the reported defect is that an unchanged tree
    /// reads as drift, and the two exports of an unchanged tree differ exactly here — the
    /// same entries, a different permutation.
    #[test]
    fn two_exports_differing_only_in_list_order_report_no_changes() {
        use crate::core_graph::conventions::ChurnEntry;
        let entry = |path: &str| ChurnEntry {
            path: path.to_string(),
            modifications: 1,
            last_commit_epoch: Some(1_700_000_000),
        };
        let mut pa = ConventionProfile::default();
        let mut pb = ConventionProfile::default();
        pa.git_health.churn_180d = vec![
            entry("pkg/gamma.py"),
            entry("pkg/alpha.py"),
            entry(".gitignore"),
        ];
        pb.git_health.churn_180d = vec![
            entry("pkg/alpha.py"),
            entry(".gitignore"),
            entry("pkg/gamma.py"),
        ];
        let a = build_export("repo", pa);
        let b = build_export("repo", pb);
        let diff = diff_exports(&a, &b);
        assert!(
            !diff.has_changes,
            "an unchanged tree exported twice must not read as drift: {} / {:?}",
            diff.summary, diff.changed_fields
        );
    }

    /// The summary for "checksum differs, fields identical" used to name `generated_at`,
    /// which `compute_checksum` excludes by construction — it hashes the profile only. The
    /// one diagnostic offered for this state sent every reader to the one cause that cannot
    /// produce it.
    #[test]
    fn the_identical_fields_summary_does_not_blame_the_timestamp() {
        let mut export = build_export("repo", ConventionProfile::default());
        let baseline = build_export("repo", ConventionProfile::default());
        // A stale/hand-edited checksum: the profile is untouched, the stored digest is not.
        export.checksum = "deadbeefdeadbeef".to_string();
        let diff = diff_exports(&export, &baseline);
        assert!(
            diff.has_changes && diff.changed_fields.is_empty(),
            "{diff:?}"
        );
        assert!(
            !diff.summary.contains("generated_at"),
            "the timestamp cannot affect the checksum, so it must not be offered as the \
             explanation: {}",
            diff.summary
        );
        assert!(
            diff.summary.contains("stale") || diff.summary.contains("hand-edited"),
            "the summary must name a cause that can actually produce this state: {}",
            diff.summary
        );
    }

    /// The field-by-field path, which only runs when the checksum fast-path does NOT fire.
    /// Without canonicalizing both sides here, the two disagree: the checksum says the
    /// profiles are the same and the field walk says `git_health` changed, purely from list
    /// order. A stale checksum is the ordinary way to reach this path.
    #[test]
    fn a_stale_checksum_does_not_turn_list_order_into_a_changed_field() {
        use crate::core_graph::conventions::ChurnEntry;
        let entry = |path: &str| ChurnEntry {
            path: path.to_string(),
            modifications: 1,
            last_commit_epoch: Some(1_700_000_000),
        };
        let mut pa = ConventionProfile::default();
        let mut pb = ConventionProfile::default();
        pa.git_health.churn_180d = vec![entry("pkg/gamma.py"), entry("pkg/alpha.py")];
        pb.git_health.churn_180d = vec![entry("pkg/alpha.py"), entry("pkg/gamma.py")];
        let mut a = build_export("repo", pa);
        let b = build_export("repo", pb);
        a.checksum = "deadbeefdeadbeef".to_string();
        let diff = diff_exports(&a, &b);
        assert!(
            diff.changed_fields.is_empty(),
            "a permuted list is not a changed field: {:?}",
            diff.changed_fields
        );
    }

    #[test]
    fn diff_detects_changed_checksum() {
        let mut pa = ConventionProfile::default();
        let mut pb = ConventionProfile::default();
        pa.git_health.reverts = vec![];
        pb.git_health.reverts = vec![crate::conventions::git_health::RevertEntry {
            commit_message: "revert fix".into(),
            reverted_message: Some("fix: something".into()),
        }];
        let a = build_export("repo", pa);
        let b = build_export("repo", pb);
        assert_ne!(a.checksum, b.checksum);
        let diff = diff_exports(&a, &b);
        assert!(diff.has_changes);
        assert!(!diff.summary.is_empty());
    }

    #[test]
    fn diff_output_contains_field_name() {
        let mut pa = ConventionProfile::default();
        let mut pb = ConventionProfile::default();
        pa.git_health.reverts = vec![];
        pb.git_health.reverts = vec![crate::conventions::git_health::RevertEntry {
            commit_message: "revert fix".into(),
            reverted_message: Some("fix: something".into()),
        }];
        let a = build_export("repo", pa);
        let b = build_export("repo", pb);
        let diff = diff_exports(&a, &b);
        assert!(diff.changed_fields.iter().any(|f| f.contains("git_health")));
    }

    #[test]
    fn diff_recomputed_checksum_fast_path() {
        // Identical profiles: recomputed checksum equals stored checksum equals baseline → no changes.
        let profile = ConventionProfile::default();
        let current = build_export("repo", profile.clone());
        let baseline = build_export("repo", profile);
        let diff = diff_exports(&current, &baseline);
        assert!(
            !diff.has_changes,
            "identical profiles should have no changes"
        );
        assert!(diff.changed_fields.is_empty());
    }

    #[test]
    fn diff_tampered_checksum_falls_through() {
        // Tamper current's checksum to something fake; since recomputed != stored,
        // fast path is NOT taken and we fall through to field diff.
        let profile = ConventionProfile::default();
        let mut current = build_export("repo", profile.clone());
        let baseline = build_export("repo", profile);
        // Set a fake checksum that matches baseline but doesn't match the profile
        current.checksum = "00000000000000000000000000000000".to_string();
        // recomputed ≠ current.checksum so fast path skips; fall through to field diff.
        // Both profiles are identical → changed_fields empty, summary notes metadata change.
        let diff = diff_exports(&current, &baseline);
        // has_changes may be true (checksum was tampered) but no actual field changes.
        assert!(diff.changed_fields.is_empty(), "no profile fields changed");
    }
}
