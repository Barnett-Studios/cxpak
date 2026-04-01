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
