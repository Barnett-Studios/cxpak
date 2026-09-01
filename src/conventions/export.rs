use crate::conventions::ConventionProfile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConventionExport {
    pub version: String,
    pub generated_at: String,
    pub generator: String,
    pub repo: String,
    pub profile: ConventionProfile,
    pub checksum: String,
}

/// Compute a stable SHA256 checksum of the profile by serializing via BTreeMap
/// for deterministic key ordering **and ordering the arrays under those keys**.
///
/// cxpak#70: sorting keys does not order the lists inside them. Several profile fields are
/// built by draining a `HashMap`, whose iteration order is randomised per process, and their
/// sorts were not total — `churn_180d` sorts by `modifications` descending, so a tree where
/// every file was touched once has no tiebreaker and emits a different permutation each run.
/// The checksum therefore differed between two exports of an unchanged tree, and
/// `conventions diff` reported drift on 9 runs in 10, which is a CI gate that enforces
/// nothing in the direction that gets it switched off.
///
/// The producers are fixed to sort totally, so the emitted profile is stable for a human
/// reading two committed files. This is the backstop under that: a field that ever becomes
/// non-deterministic again cannot make the checksum move, because reordering a list without
/// changing its contents is not drift. Ranked lists keep their rank in the emitted JSON —
/// only the hashed image is reordered.
pub fn compute_checksum(profile: &ConventionProfile) -> String {
    let value = serde_json::to_value(profile).unwrap_or_default();
    let stable = to_stable_value(value);
    let json = serde_json::to_string(&stable).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Canonical image of a value: object keys sorted, array elements sorted, recursively.
pub(crate) fn to_stable_value(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => {
            let btree: BTreeMap<_, _> = map
                .into_iter()
                .map(|(k, val)| (k, to_stable_value(val)))
                .collect();
            serde_json::Value::Object(btree.into_iter().collect())
        }
        serde_json::Value::Array(arr) => {
            // Sorted by the canonical serialization of each element: a total order that needs
            // no per-field knowledge, so a new list field is covered the day it is added
            // rather than the day someone remembers to add it here.
            let mut items: Vec<serde_json::Value> = arr.into_iter().map(to_stable_value).collect();
            items.sort_by_cached_key(|v| serde_json::to_string(v).unwrap_or_default());
            serde_json::Value::Array(items)
        }
        other => other,
    }
}

/// Build a convention export for a given repo path and profile.
pub fn build_export(repo: &str, profile: ConventionProfile) -> ConventionExport {
    let checksum = compute_checksum(&profile);
    ConventionExport {
        version: "1.0".to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        generator: format!("cxpak {}", env!("CARGO_PKG_VERSION")),
        repo: repo.to_string(),
        profile,
        checksum,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convention_export_roundtrip() {
        let profile = ConventionProfile::default();
        let export = build_export("test-repo", profile);
        assert_eq!(export.version, "1.0");
        assert!(export.generator.starts_with("cxpak "));
        assert!(!export.checksum.is_empty());
        let json = serde_json::to_string(&export).unwrap();
        let back: ConventionExport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.checksum, export.checksum);
    }

    #[test]
    fn checksum_is_deterministic() {
        let profile = ConventionProfile::default();
        let a = compute_checksum(&profile);
        let b = compute_checksum(&profile);
        assert_eq!(a, b);
    }

    /// cxpak#70. Two profiles whose only difference is the ORDER of a list must hash the
    /// same: reordering entries without changing them is not drift, and the producers feed
    /// these lists from `HashMap`s whose drain order is randomised per process.
    #[test]
    fn checksum_ignores_the_order_of_list_entries() {
        use crate::core_graph::conventions::ChurnEntry;
        let entry = |path: &str| ChurnEntry {
            path: path.to_string(),
            modifications: 1,
            last_commit_epoch: Some(1_700_000_000),
        };
        let mut a = ConventionProfile::default();
        let mut b = ConventionProfile::default();
        a.git_health.churn_180d = vec![
            entry("pkg/alpha.py"),
            entry("pkg/beta.py"),
            entry(".gitignore"),
        ];
        b.git_health.churn_180d = vec![
            entry(".gitignore"),
            entry("pkg/alpha.py"),
            entry("pkg/beta.py"),
        ];
        assert_eq!(
            compute_checksum(&a),
            compute_checksum(&b),
            "a permuted list is the same profile; a checksum that moves with it reports \
             drift on an unchanged tree"
        );
    }

    /// The control for the test above: order-insensitivity must not become
    /// content-insensitivity. Same three paths, one modification count changed.
    #[test]
    fn checksum_still_moves_when_a_list_entry_changes() {
        use crate::core_graph::conventions::ChurnEntry;
        let entry = |path: &str, n: usize| ChurnEntry {
            path: path.to_string(),
            modifications: n,
            last_commit_epoch: Some(1_700_000_000),
        };
        let mut a = ConventionProfile::default();
        let mut b = ConventionProfile::default();
        a.git_health.churn_180d = vec![entry("pkg/alpha.py", 1), entry("pkg/beta.py", 1)];
        b.git_health.churn_180d = vec![entry("pkg/beta.py", 1), entry("pkg/alpha.py", 2)];
        assert_ne!(compute_checksum(&a), compute_checksum(&b));
    }

    #[test]
    fn checksum_changes_on_profile_change() {
        let mut profile_a = ConventionProfile::default();
        let mut profile_b = ConventionProfile::default();
        profile_a.git_health.reverts = vec![];
        profile_b.git_health.reverts = vec![crate::conventions::git_health::RevertEntry {
            commit_message: "revert bad deploy".into(),
            reverted_message: Some("feat: bad deploy".into()),
        }];
        let a = compute_checksum(&profile_a);
        let b = compute_checksum(&profile_b);
        assert_ne!(a, b);
    }
}
