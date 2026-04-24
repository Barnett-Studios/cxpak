use crate::index::CodebaseIndex;
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
pub struct RecentChange {
    pub path: String,
    /// Days since the file's most recent commit in the 30-day churn window.
    /// Computed from the per-file last-commit epoch stored on each
    /// `ChurnEntry` (added in v2.1.1). For pre-v2.1.1 data without a
    /// last_commit_epoch, this is 0 (can't compute) and should be treated as
    /// unknown rather than "today".
    pub days_ago: u32,
    pub modifications_30d: u32,
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn days_between(later: i64, earlier: i64) -> u32 {
    let seconds = (later - earlier).max(0);
    (seconds / 86_400).min(u32::MAX as i64) as u32
}

/// Collect recently changed files from git_health churn data.
/// Returns files changed in the last 30 days, sorted by (days_ago asc,
/// modifications desc) — most-recent-first with modifications as tiebreaker,
/// which is the honest recency ordering now that per-file epoch is tracked.
pub fn compute_recent_changes(index: &CodebaseIndex) -> Vec<RecentChange> {
    let now = now_epoch();
    let mut entries: Vec<RecentChange> = index
        .conventions
        .git_health
        .churn_30d
        .iter()
        .filter(|e| e.modifications > 0)
        .map(|e| {
            let days_ago = e
                .last_commit_epoch
                .map(|ep| days_between(now, ep))
                .unwrap_or(0);
            RecentChange {
                path: e.path.clone(),
                days_ago,
                modifications_30d: e.modifications.min(u32::MAX as usize) as u32,
            }
        })
        .collect();

    entries.sort_by(|a, b| {
        a.days_ago
            .cmp(&b.days_ago)
            .then_with(|| b.modifications_30d.cmp(&a.modifications_30d))
            .then_with(|| a.path.cmp(&b.path))
    });
    entries
}

/// Compute recency score for a file: 1.0 for files changed today, linearly
/// decaying to 0.0 at 90 days. Returns 0.5 (neutral) when no git data available.
///
/// When a per-file `last_commit_epoch` is present (v2.1.1+), the score uses
/// the real age in days. Otherwise it falls back to bucket-based estimates
/// (30d -> 0.667, 180d-only -> 0.0) matching pre-v2.1.1 behavior, so callers
/// that depend on legacy scoring semantics on old churn data still work.
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

    // Prefer the real age when the epoch is populated.
    let best_epoch = churn_30d
        .and_then(|e| e.last_commit_epoch)
        .or_else(|| churn_180d.and_then(|e| e.last_commit_epoch));
    if let Some(ep) = best_epoch {
        let days = days_between(now_epoch(), ep) as f64;
        return (1.0 - days / 90.0).clamp(0.0, 1.0);
    }

    // Bucket-based fallback for pre-v2.1.1 data with no per-file epoch.
    if churn_30d.is_some() {
        0.667
    } else if churn_180d.is_some() {
        0.0
    } else {
        0.5
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
            last_commit_epoch: None,
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
            last_commit_epoch: None,
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
            last_commit_epoch: None,
        });
        index.conventions.git_health.churn_30d.push(ChurnEntry {
            path: "b.rs".into(),
            modifications: 10,
            last_commit_epoch: None,
        });
        let changes = compute_recent_changes(&index);
        assert_eq!(changes.len(), 2);
        // With no per-file epoch both entries share days_ago=0, so modification
        // count is the tiebreaker and b.rs (higher mods) sorts first.
        assert_eq!(changes[0].path, "b.rs", "higher modifications first");
        assert_eq!(changes[0].modifications_30d, 10);
    }

    #[test]
    fn test_compute_recent_changes_uses_real_days_ago() {
        // With last_commit_epoch populated, days_ago must reflect real age
        // and be the PRIMARY sort key (most recent first). The old placeholder
        // that emitted days_ago=0 for every entry would pass a modifications-
        // based order; this test catches that regression.
        use crate::budget::counter::TokenCounter;
        use crate::conventions::git_health::ChurnEntry;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("x.rs");
        std::fs::write(&fp, "fn x() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "x.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 9,
        }];
        let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let now = now_epoch();
        // Heavily-modified file (10 commits) but commits are ~20 days old.
        index.conventions.git_health.churn_30d.push(ChurnEntry {
            path: "old_hot.rs".into(),
            modifications: 10,
            last_commit_epoch: Some(now - 20 * 86_400),
        });
        // Lightly-modified file (1 commit) but committed TODAY.
        index.conventions.git_health.churn_30d.push(ChurnEntry {
            path: "fresh.rs".into(),
            modifications: 1,
            last_commit_epoch: Some(now),
        });
        let changes = compute_recent_changes(&index);
        assert_eq!(changes[0].path, "fresh.rs", "most recent must sort first");
        assert_eq!(
            changes[0].days_ago, 0,
            "fresh commit's days_ago must be 0 (real value), not a placeholder"
        );
        assert_eq!(
            changes[1].days_ago, 20,
            "older commit's days_ago must be ~20, not the 0 placeholder"
        );
    }

    #[test]
    fn test_recency_score_uses_real_epoch_when_present() {
        // Score with a real epoch must compute real age, not the bucket
        // estimate. At 45 days old, score = 1 - 45/90 = 0.5 exactly.
        use crate::budget::counter::TokenCounter;
        use crate::conventions::git_health::ChurnEntry;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("c.rs");
        std::fs::write(&fp, "fn c() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "c.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 9,
        }];
        let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let now = now_epoch();
        index.conventions.git_health.churn_180d.push(ChurnEntry {
            path: "c.rs".into(),
            modifications: 2,
            last_commit_epoch: Some(now - 45 * 86_400),
        });
        let score = recency_score_for_file("c.rs", &index);
        assert!(
            (score - 0.5).abs() < 0.02,
            "real-epoch-based score for 45d must be ~0.5, got {score}"
        );
    }
}
