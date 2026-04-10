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
/// for deterministic key ordering.
pub fn compute_checksum(profile: &ConventionProfile) -> String {
    let value = serde_json::to_value(profile).unwrap_or_default();
    let stable = to_stable_value(value);
    let json = serde_json::to_string(&stable).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn to_stable_value(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => {
            let btree: BTreeMap<_, _> = map
                .into_iter()
                .map(|(k, val)| (k, to_stable_value(val)))
                .collect();
            serde_json::Value::Object(btree.into_iter().collect())
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(to_stable_value).collect())
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
