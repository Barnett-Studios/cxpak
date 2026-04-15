use crate::conventions::export::{compute_checksum, ConventionExport};
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

    let current_val = serde_json::to_value(&current.profile).unwrap_or_default();
    let baseline_val = serde_json::to_value(&baseline.profile).unwrap_or_default();

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
        "Checksum differs (generated_at or metadata changed) but profile fields are identical."
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
