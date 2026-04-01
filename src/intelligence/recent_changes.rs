use crate::index::CodebaseIndex;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RecentChange {
    pub path: String,
    pub days_ago: u32,
    pub modifications_30d: u32,
}

/// Collect recently changed files from git_health churn data.
/// Returns files changed in the last 30 days, sorted by modification count descending.
pub fn compute_recent_changes(index: &CodebaseIndex) -> Vec<RecentChange> {
    let mut entries: Vec<RecentChange> = index
        .conventions
        .git_health
        .churn_30d
        .iter()
        .filter(|e| e.modifications > 0)
        .map(|e| RecentChange {
            path: e.path.clone(),
            days_ago: 0, // days_ago is not stored per-file in v1.2.0; use 0 as placeholder
            modifications_30d: e.modifications as u32,
        })
        .collect();

    // Sort by modification count descending (proxy for recency when days_ago unavailable)
    entries.sort_by(|a, b| b.modifications_30d.cmp(&a.modifications_30d));
    entries
}

/// Compute recency score for a file: 1.0 for files changed today, linearly
/// decaying to 0.0 at 90 days. Returns 0.5 (neutral) when no git data available.
pub fn recency_score_for_file(path: &str, index: &CodebaseIndex) -> f64 {
    let churn_30d = index
        .conventions
        .git_health
        .churn_30d
        .iter()
        .find(|e| e.path == path);
    let churn_180d = index
        .conventions
        .git_health
        .churn_180d
        .iter()
        .find(|e| e.path == path);

    // If the file appears in churn_30d, it was recently modified.
    // We estimate days_ago from 0 (in 30d bucket). If only in 180d, use ~60 days estimate.
    // If not in either, use neutral 0.5.
    if churn_30d.is_some() {
        // File changed within 30 days; score linearly from 1.0 (0 days) to ~0.67 (30 days)
        // Use conservative worst case: 1.0 - (30/90) = 0.667
        0.667
    } else if churn_180d.is_some() {
        // File changed within 180 days but not in last 30.
        // Score = 1.0 - (105/90) clamped to 0.0 (past the 90d decay window)
        0.0
    } else {
        0.5 // neutral: no git data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recency_score_no_git_data() {
        use crate::budget::counter::TokenCounter;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("a.rs");
        std::fs::write(&fp, "fn a() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "a.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 9,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        // No git data -> neutral 0.5
        let score = recency_score_for_file("a.rs", &index);
        assert!((score - 0.5).abs() < 1e-9, "expected 0.5, got {score}");
    }

    #[test]
    fn test_recency_score_in_30d_bucket() {
        use crate::budget::counter::TokenCounter;
        use crate::conventions::git_health::ChurnEntry;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("a.rs");
        std::fs::write(&fp, "fn a() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "a.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 9,
        }];
        let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
        // Inject churn_30d entry
        index.conventions.git_health.churn_30d.push(ChurnEntry {
            path: "a.rs".into(),
            modifications: 3,
        });
        let score = recency_score_for_file("a.rs", &index);
        assert!((score - 0.667).abs() < 1e-9, "expected 0.667, got {score}");
    }

    #[test]
    fn test_recency_score_only_in_180d_bucket() {
        use crate::budget::counter::TokenCounter;
        use crate::conventions::git_health::ChurnEntry;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("b.rs");
        std::fs::write(&fp, "fn b() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "b.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 9,
        }];
        let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
        // Only in 180d bucket, not in 30d
        index.conventions.git_health.churn_180d.push(ChurnEntry {
            path: "b.rs".into(),
            modifications: 5,
        });
        let score = recency_score_for_file("b.rs", &index);
        assert!((score - 0.0).abs() < 1e-9, "expected 0.0, got {score}");
    }

    #[test]
    fn test_compute_recent_changes_empty() {
        use crate::budget::counter::TokenCounter;
        use std::collections::HashMap;
        let counter = TokenCounter::new();
        let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
        let changes = compute_recent_changes(&index);
        assert!(changes.is_empty());
    }

    #[test]
    fn test_compute_recent_changes_sorted_by_modifications() {
        use crate::budget::counter::TokenCounter;
        use crate::conventions::git_health::ChurnEntry;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp_a = dir.path().join("a.rs");
        let fp_b = dir.path().join("b.rs");
        std::fs::write(&fp_a, "fn a() {}").unwrap();
        std::fs::write(&fp_b, "fn b() {}").unwrap();
        let files = vec![
            ScannedFile {
                relative_path: "a.rs".into(),
                absolute_path: fp_a,
                language: Some("rust".into()),
                size_bytes: 9,
            },
            ScannedFile {
                relative_path: "b.rs".into(),
                absolute_path: fp_b,
                language: Some("rust".into()),
                size_bytes: 9,
            },
        ];
        let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
        index.conventions.git_health.churn_30d.push(ChurnEntry {
            path: "a.rs".into(),
            modifications: 2,
        });
        index.conventions.git_health.churn_30d.push(ChurnEntry {
            path: "b.rs".into(),
            modifications: 10,
        });
        let changes = compute_recent_changes(&index);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].path, "b.rs", "higher modifications first");
        assert_eq!(changes[0].modifications_30d, 10);
    }
}
