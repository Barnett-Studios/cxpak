use crate::index::CodebaseIndex;
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
pub struct RecentChange {
    pub path: String,
    /// Days since the file's most recent commit in the 30-day churn window.
    /// Computed from the per-file last-commit epoch added on `ChurnEntry`
    /// in v2.1.1.  For pre-v2.1.1 cached data the field is `None`,
    /// serialized as `null`, and `unknown_days_ago` is `true` so a client
    /// can distinguish "unknown" from a real `0` (today). Sort order
    /// places unknown entries LAST — never claiming "this file changed
    /// today" when we don't actually know.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_ago: Option<u32>,
    /// Convenience flag — true when `days_ago` is `None`. Mirrors the
    /// presence/absence of the field for clients that don't introspect.
    pub unknown_days_ago: bool,
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
///
/// Sort order:
/// 1. Known days_ago ascending (most recent first)
/// 2. Unknown days_ago entries last (sort key = u32::MAX so they trail)
/// 3. Modifications desc (tiebreaker within same recency)
/// 4. Path asc (deterministic final key)
///
/// This corrects a v2.1.1 ordering bug where unknown-epoch entries fell
/// back to days_ago=0 and sorted as "most recent" — silently lying to the
/// user that pre-cached files were the freshest.
pub fn compute_recent_changes(index: &CodebaseIndex) -> Vec<RecentChange> {
    let now = now_epoch();
    let mut entries: Vec<RecentChange> = index
        .conventions
        .git_health
        .churn_30d
        .iter()
        .filter(|e| e.modifications > 0)
        .map(|e| {
            let days_ago = e.last_commit_epoch.map(|ep| days_between(now, ep));
            RecentChange {
                path: e.path.clone(),
                days_ago,
                unknown_days_ago: days_ago.is_none(),
                modifications_30d: e.modifications.min(u32::MAX as usize) as u32,
            }
        })
        .collect();

    entries.sort_by(|a, b| {
        // None sorts after Some(*) — unknown entries trail the list.
        let a_key = a.days_ago.unwrap_or(u32::MAX);
        let b_key = b.days_ago.unwrap_or(u32::MAX);
        a_key
            .cmp(&b_key)
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
            changes[0].days_ago,
            Some(0),
            "fresh commit's days_ago must be Some(0) (real value), not a placeholder"
        );
        assert!(
            !changes[0].unknown_days_ago,
            "fresh commit days_ago is known"
        );
        assert_eq!(
            changes[1].days_ago,
            Some(20),
            "older commit's days_ago must be Some(20), not the 0 placeholder"
        );
    }

    #[test]
    fn test_unknown_days_ago_sorts_last_not_first() {
        // Pre-v2.1.1 cached data has last_commit_epoch = None.  The earlier
        // sort fell back to days_ago = 0 for those entries, putting them
        // ahead of real recent commits — silently lying that the cached
        // file was the freshest.  Unknown entries must trail.
        use crate::budget::counter::TokenCounter;
        use crate::conventions::git_health::ChurnEntry;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("z.rs");
        std::fs::write(&fp, "fn z() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "z.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 9,
        }];
        let mut index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let now = now_epoch();
        // Entry with KNOWN epoch, 5 days old, modest churn.
        index.conventions.git_health.churn_30d.push(ChurnEntry {
            path: "known_fresh.rs".into(),
            modifications: 2,
            last_commit_epoch: Some(now - 5 * 86_400),
        });
        // Entry with UNKNOWN epoch (pre-v2.1.1 cached), heavy churn.
        index.conventions.git_health.churn_30d.push(ChurnEntry {
            path: "legacy_unknown.rs".into(),
            modifications: 50,
            last_commit_epoch: None,
        });
        let changes = compute_recent_changes(&index);
        assert_eq!(
            changes[0].path, "known_fresh.rs",
            "the entry with a real epoch must sort first; old code put unknown first by virtue of days_ago=0 placeholder. Got: {changes:?}"
        );
        assert_eq!(changes[1].path, "legacy_unknown.rs");
        assert!(changes[1].unknown_days_ago);
        assert_eq!(changes[1].days_ago, None);
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
