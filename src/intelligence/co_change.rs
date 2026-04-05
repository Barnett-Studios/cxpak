use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoChangeEdge {
    pub file_a: String,
    pub file_b: String,
    pub count: u32,
    pub recency_weight: f64,
}

/// Decay weight for a commit `days_ago` days old (180d window).
/// Returns 1.0 for days_ago <= 30, linearly decays to 0.3 at days_ago == 180.
/// Commits older than 180 days are excluded before calling this.
pub fn co_change_weight(days_ago: i64) -> f64 {
    if days_ago <= 30 {
        1.0
    } else {
        // days_ago in (30, 180]: linearly interpolate from 1.0 down to 0.3
        1.0 - 0.7 * (days_ago - 30) as f64 / 150.0
    }
}

/// Build co-change edges from a list of (commit_files, days_ago) pairs.
///
/// A pair (file_a, file_b) becomes an edge when it co-appears in >= 3 commits
/// within the 180-day window. `recency_weight` is the weight of the most recent
/// co-commit (not the average), per the design spec.
///
/// `commits` is `Vec<(Vec<String>, i64)>` where the i64 is days_ago at index time.
pub fn build_co_changes(commits: &[(Vec<String>, i64)]) -> Vec<CoChangeEdge> {
    use std::collections::HashMap;

    // Map (sorted file_a, file_b) -> (count, most_recent_days_ago)
    let mut pair_data: HashMap<(String, String), (u32, i64)> = HashMap::new();

    for (files, days_ago) in commits {
        if files.len() < 2 {
            continue;
        }
        // Build all pairs from the commit's changed files (sorted for dedup)
        for i in 0..files.len() {
            for j in (i + 1)..files.len() {
                let a = files[i].clone();
                let b = files[j].clone();
                let key = if a <= b { (a, b) } else { (b, a) };
                let entry = pair_data.entry(key).or_insert((0, *days_ago));
                entry.0 += 1;
                // Track the most recent (smallest days_ago)
                if *days_ago < entry.1 {
                    entry.1 = *days_ago;
                }
            }
        }
    }

    pair_data
        .into_iter()
        .filter(|(_, (count, _))| *count >= 3)
        .map(
            |((file_a, file_b), (count, most_recent_days))| CoChangeEdge {
                file_a,
                file_b,
                count,
                recency_weight: co_change_weight(most_recent_days),
            },
        )
        .collect()
}

/// Alias for `build_co_changes()` taking per-commit dates.
pub fn build_co_change_edges_with_dates(
    commits: &[(Vec<String>, i64)],
    threshold: u32,
    window_days: i64,
) -> Vec<CoChangeEdge> {
    // Filter commits outside window, then delegate to build_co_changes
    let within_window: Vec<(Vec<String>, i64)> = commits
        .iter()
        .filter(|(_, days_ago)| *days_ago <= window_days)
        .cloned()
        .collect();
    let mut edges = build_co_changes(&within_window);
    edges.retain(|e| e.count >= threshold);
    edges
}

/// Convenience wrapper: convert `Vec<Vec<String>>` (no dates) by assuming days_ago=0.
pub fn build_co_change_edges(
    commits: &[Vec<String>],
    threshold: u32,
    window_days: i64,
) -> Vec<CoChangeEdge> {
    let with_dates: Vec<(Vec<String>, i64)> = commits.iter().map(|c| (c.clone(), 0i64)).collect();
    build_co_change_edges_with_dates(&with_dates, threshold, window_days)
}

/// Mine co-change data from a git repository using git2.
/// Returns `(changed_files, days_ago)` for each commit within the window.
pub fn mine_co_changes_from_git(
    repo_path: &std::path::Path,
    window_days: i64,
) -> Vec<(Vec<String>, i64)> {
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let cutoff = now - window_days * 86400;

    let mut revwalk = match repo.revwalk() {
        Ok(rw) => rw,
        Err(_) => return vec![],
    };
    if revwalk.push_head().is_err() {
        return vec![];
    }

    let mut results = Vec::new();

    for oid_result in revwalk {
        let oid = match oid_result {
            Ok(o) => o,
            Err(_) => continue,
        };
        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let commit_time = commit.time().seconds();
        if commit_time < cutoff {
            break; // revwalk is time-ordered descending
        }

        let days_ago = (now - commit_time) / 86400;

        let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());
        let current_tree = match commit.tree() {
            Ok(t) => t,
            Err(_) => continue,
        };

        let diff = match repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&current_tree), None) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let mut changed_files: Vec<String> = Vec::new();
        diff.foreach(
            &mut |delta, _| {
                if let Some(path) = delta.new_file().path() {
                    changed_files.push(path.to_string_lossy().to_string());
                }
                true
            },
            None,
            None,
            None,
        )
        .ok();

        if changed_files.len() >= 2 {
            results.push((changed_files, days_ago));
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_co_change_weight_at_zero_days() {
        assert!((co_change_weight(0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_co_change_weight_at_30_days() {
        assert!((co_change_weight(30) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_co_change_weight_at_180_days() {
        // 1.0 - 0.7 * (180-30)/150 = 1.0 - 0.7 = 0.3
        assert!((co_change_weight(180) - 0.3).abs() < 1e-9);
    }

    #[test]
    fn test_co_change_weight_at_105_days() {
        // 1.0 - 0.7 * 75/150 = 1.0 - 0.35 = 0.65
        assert!((co_change_weight(105) - 0.65).abs() < 1e-9);
    }

    #[test]
    fn test_build_co_changes_threshold_3() {
        // Two files co-appear in exactly 2 commits -> filtered out (< 3)
        let commits = vec![
            (vec!["a.rs".to_string(), "b.rs".to_string()], 10i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 20i64),
        ];
        let edges = build_co_changes(&commits);
        assert!(
            edges.is_empty(),
            "pairs with < 3 co-commits must be excluded"
        );
    }

    #[test]
    fn test_build_co_changes_exactly_3_commits() {
        let commits = vec![
            (vec!["a.rs".to_string(), "b.rs".to_string()], 5i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 15i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 25i64),
        ];
        let edges = build_co_changes(&commits);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].count, 3);
        // Most recent is 5 days ago -> weight = 1.0
        assert!((edges[0].recency_weight - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_build_co_changes_recency_uses_most_recent() {
        // Co-appear 3 times; most recent is 100 days ago
        let commits = vec![
            (vec!["a.rs".to_string(), "b.rs".to_string()], 100i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 150i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 170i64),
        ];
        let edges = build_co_changes(&commits);
        assert_eq!(edges.len(), 1);
        // Weight for 100 days: 1.0 - 0.7 * 70/150 = 1.0 - 0.3267 = 0.6733
        let expected = 1.0 - 0.7 * 70.0 / 150.0;
        assert!((edges[0].recency_weight - expected).abs() < 1e-9);
    }

    #[test]
    fn test_build_co_changes_pair_ordering_canonical() {
        // Same pair in different order should be deduped
        let commits = vec![
            (vec!["b.rs".to_string(), "a.rs".to_string()], 5i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 10i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 15i64),
        ];
        let edges = build_co_changes(&commits);
        assert_eq!(edges.len(), 1, "reversed pair should be deduplicated");
        assert_eq!(edges[0].count, 3);
    }

    #[test]
    fn test_build_co_changes_single_file_commits_ignored() {
        // Commits with only 1 file produce no pairs
        let commits = vec![
            (vec!["a.rs".to_string()], 5i64),
            (vec!["a.rs".to_string()], 10i64),
            (vec!["a.rs".to_string()], 15i64),
        ];
        let edges = build_co_changes(&commits);
        assert!(edges.is_empty());
    }

    #[test]
    fn test_build_co_change_edges_threshold_filters_noise() {
        let commits: Vec<Vec<String>> = vec![
            vec!["a.rs".into(), "b.rs".into()],
            vec!["a.rs".into(), "b.rs".into()],
        ];
        let edges = build_co_change_edges(&commits, 3, 180);
        assert!(edges.is_empty(), "below threshold should produce no edges");
    }

    #[test]
    fn test_build_co_change_edges_meets_threshold() {
        let commits: Vec<Vec<String>> = vec![
            vec!["a.rs".into(), "b.rs".into()],
            vec!["a.rs".into(), "b.rs".into()],
            vec!["a.rs".into(), "b.rs".into()],
        ];
        let edges = build_co_change_edges(&commits, 3, 180);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].count, 3);
    }

    #[test]
    fn test_build_co_change_edges_with_dates_recency() {
        let commits_with_dates = vec![
            (vec!["a.rs".to_string(), "b.rs".to_string()], 0i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 5i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 10i64),
        ];
        let edges = build_co_change_edges_with_dates(&commits_with_dates, 3, 180);
        assert!((edges[0].recency_weight - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_build_co_change_edges_excludes_beyond_window() {
        let commits_with_dates = vec![
            (vec!["a.rs".to_string(), "b.rs".to_string()], 181i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 200i64),
            (vec!["a.rs".to_string(), "b.rs".to_string()], 250i64),
        ];
        let edges = build_co_change_edges_with_dates(&commits_with_dates, 3, 180);
        assert!(
            edges.is_empty(),
            "commits beyond 180-day window must be excluded"
        );
    }

    #[test]
    fn test_mine_co_changes_nonexistent_repo_returns_empty() {
        let result = mine_co_changes_from_git(std::path::Path::new("/nonexistent/path"), 180);
        assert!(result.is_empty(), "non-existent repo must return empty vec");
    }

    #[test]
    fn test_build_co_changes_multiple_pairs() {
        // Three files co-appearing: a+b (4x), a+c (3x), b+c (2x - excluded)
        let commits: Vec<(Vec<String>, i64)> = (0..4)
            .map(|i| (vec!["a.rs".to_string(), "b.rs".to_string()], i as i64 * 10))
            .chain((0..3).map(|i| (vec!["a.rs".to_string(), "c.rs".to_string()], i as i64 * 10)))
            .chain((0..2).map(|i| (vec!["b.rs".to_string(), "c.rs".to_string()], i as i64 * 10)))
            .collect();
        let edges = build_co_changes(&commits);
        assert_eq!(
            edges.len(),
            2,
            "a+b and a+c should qualify; b+c (count=2) should not"
        );
        let has_ab = edges.iter().any(|e| {
            (e.file_a == "a.rs" && e.file_b == "b.rs") || (e.file_a == "b.rs" && e.file_b == "a.rs")
        });
        let has_ac = edges.iter().any(|e| {
            (e.file_a == "a.rs" && e.file_b == "c.rs") || (e.file_a == "c.rs" && e.file_b == "a.rs")
        });
        assert!(has_ab);
        assert!(has_ac);
    }
}
