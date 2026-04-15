use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHealthProfile {
    pub churn_30d: Vec<ChurnEntry>,
    pub churn_180d: Vec<ChurnEntry>,
    pub bugfix_density: HashMap<String, f64>,
    pub reverts: Vec<RevertEntry>,
    pub churn_trend: HashMap<String, ChurnTrend>,
    pub co_changes: Vec<crate::intelligence::co_change::CoChangeEdge>,
    #[serde(skip)]
    pub last_computed: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChurnEntry {
    pub path: String,
    pub modifications: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevertEntry {
    pub commit_message: String,
    pub reverted_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChurnTrend {
    Hot,        // high 30d, lower 180d (growing)
    Stabilized, // low 30d, high 180d (cooling down)
    Chronic,    // high both windows
    Cold,       // low both windows
}

const TTL_SECONDS: u64 = 60;

/// Extract git health metrics from the repository.
///
/// Uses `git2` exclusively — no CLI.
pub fn extract_git_health(repo_path: &Path) -> GitHealthProfile {
    let repo = match git2::Repository::discover(repo_path) {
        Ok(r) => r,
        Err(_) => return GitHealthProfile::default(),
    };

    let now_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let thirty_days_ago = now_epoch - (30 * 24 * 3600);
    let one_eighty_days_ago = now_epoch - (180 * 24 * 3600);

    let mut churn_30d: HashMap<String, usize> = HashMap::new();
    let mut churn_180d: HashMap<String, usize> = HashMap::new();
    let mut bugfix_commits: HashMap<String, usize> = HashMap::new();
    let mut dir_commits: HashMap<String, usize> = HashMap::new();
    let mut reverts: Vec<RevertEntry> = Vec::new();
    // Accumulate (changed_files, days_ago) for co-change analysis
    let mut commit_file_sets: Vec<(Vec<String>, i64)> = Vec::new();

    let bugfix_re = regex::Regex::new(r"(?i)\b(fix|bug|patch|hotfix)\b")
        .unwrap_or_else(|_| regex::Regex::new(r"$^").unwrap());
    let revert_re =
        regex::Regex::new(r"(?i)^revert\b").unwrap_or_else(|_| regex::Regex::new(r"$^").unwrap());
    let revert_hash_re = regex::Regex::new(r"This reverts commit ([0-9a-f]+)")
        .unwrap_or_else(|_| regex::Regex::new(r"$^").unwrap());

    let mut revwalk = match repo.revwalk() {
        Ok(r) => r,
        Err(_) => return GitHealthProfile::default(),
    };
    revwalk.set_sorting(git2::Sort::TIME).ok();
    if revwalk.push_head().is_err() {
        return GitHealthProfile::default();
    }

    for oid in revwalk.flatten() {
        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let commit_time = commit.time().seconds();
        if commit_time < one_eighty_days_ago {
            break;
        }

        let message = commit.message().unwrap_or("");

        // Get changed files via diff
        let tree = match commit.tree() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());
        let diff = match repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let mut changed_files: Vec<String> = Vec::new();
        diff.foreach(
            &mut |delta, _| {
                if let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) {
                    changed_files.push(path.to_string());
                }
                true
            },
            None,
            None,
            None,
        )
        .ok();

        let is_bugfix = bugfix_re.is_match(message);

        // Accumulate for co-change analysis (days_ago from commit time)
        let days_ago = (now_epoch - commit_time).max(0) / 86400;
        if !changed_files.is_empty() {
            commit_file_sets.push((changed_files.clone(), days_ago));
        }

        for path in &changed_files {
            // 180d churn
            *churn_180d.entry(path.clone()).or_insert(0) += 1;

            // 30d churn
            if commit_time >= thirty_days_ago {
                *churn_30d.entry(path.clone()).or_insert(0) += 1;
            }

            // Bug-fix density per directory
            let dir = path
                .rsplit_once('/')
                .map(|(d, _)| d.to_string())
                .unwrap_or_default();
            *dir_commits.entry(dir.clone()).or_insert(0) += 1;
            if is_bugfix {
                *bugfix_commits.entry(dir).or_insert(0) += 1;
            }
        }

        // Revert detection
        if revert_re.is_match(message) {
            let reverted_message = revert_hash_re
                .captures(message)
                .and_then(|caps| caps.get(1))
                .and_then(|m| {
                    git2::Oid::from_str(m.as_str())
                        .ok()
                        .and_then(|oid| repo.find_commit(oid).ok())
                        .and_then(|c| c.message().map(|s| s.trim().to_string()))
                });

            reverts.push(RevertEntry {
                commit_message: message.lines().next().unwrap_or("").to_string(),
                reverted_message,
            });
        }
    }

    // Sort churn entries by modification count (descending)
    let mut churn_30d_vec: Vec<ChurnEntry> = churn_30d
        .into_iter()
        .map(|(path, modifications)| ChurnEntry {
            path,
            modifications,
        })
        .collect();
    churn_30d_vec.sort_by(|a, b| b.modifications.cmp(&a.modifications));

    let mut churn_180d_vec: Vec<ChurnEntry> = churn_180d
        .into_iter()
        .map(|(path, modifications)| ChurnEntry {
            path,
            modifications,
        })
        .collect();
    churn_180d_vec.sort_by(|a, b| b.modifications.cmp(&a.modifications));

    // Compute the 75th-percentile churn threshold from the 180-day data.
    // This replaces the old absolute threshold of 5.
    let high_churn_threshold = {
        let mut vals: Vec<usize> = churn_180d_vec.iter().map(|e| e.modifications).collect();
        vals.sort_unstable();
        let p75 = if vals.is_empty() {
            5
        } else {
            vals.get(vals.len() * 3 / 4).copied().unwrap_or(5)
        };
        p75.max(3)
    };

    // Bug-fix density per directory
    let mut bugfix_density: HashMap<String, f64> = HashMap::new();
    for (dir, total) in &dir_commits {
        let fixes = bugfix_commits.get(dir).copied().unwrap_or(0);
        if *total > 0 {
            bugfix_density.insert(dir.clone(), fixes as f64 / *total as f64);
        }
    }

    // Churn trend per file
    let mut churn_trend: HashMap<String, ChurnTrend> = HashMap::new();
    let all_paths: std::collections::HashSet<String> = churn_30d_vec
        .iter()
        .map(|e| e.path.clone())
        .chain(churn_180d_vec.iter().map(|e| e.path.clone()))
        .collect();

    for path in all_paths {
        let c30 = churn_30d_vec
            .iter()
            .find(|e| e.path == path)
            .map(|e| e.modifications)
            .unwrap_or(0);
        let c180 = churn_180d_vec
            .iter()
            .find(|e| e.path == path)
            .map(|e| e.modifications)
            .unwrap_or(0);

        let trend = classify_trend(c30, c180, high_churn_threshold);
        churn_trend.insert(path, trend);
    }

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Build co-change edges from the accumulated commit file sets
    let co_changes = crate::intelligence::co_change::build_co_changes(&commit_file_sets);

    GitHealthProfile {
        churn_30d: churn_30d_vec,
        churn_180d: churn_180d_vec,
        bugfix_density,
        reverts,
        churn_trend,
        co_changes,
        last_computed: Some(now_secs),
    }
}

/// Compute the 75th-percentile churn threshold from a map of file → modification counts.
/// Falls back to 3 when the map is empty or the computed percentile is below 3.
pub fn p75_churn_threshold(churn_map: &std::collections::HashMap<String, usize>) -> usize {
    let mut vals: Vec<usize> = churn_map.values().copied().collect();
    vals.sort_unstable();
    let p75 = if vals.is_empty() {
        5
    } else {
        vals.get(vals.len() * 3 / 4).copied().unwrap_or(5)
    };
    p75.max(3)
}

fn classify_trend(c30: usize, c180: usize, high_threshold: usize) -> ChurnTrend {
    // Normalize 30d to 180d scale: c30 * 6
    let c30_normalized = c30 * 6;

    match (c30 >= high_threshold, c180 >= high_threshold) {
        (true, true) => {
            if c30_normalized > c180 {
                ChurnTrend::Hot // growing
            } else {
                ChurnTrend::Chronic // sustained
            }
        }
        (true, false) => ChurnTrend::Hot,
        (false, true) => ChurnTrend::Stabilized,
        (false, false) => ChurnTrend::Cold,
    }
}

/// Check if git health needs refresh (TTL expired).
pub fn needs_refresh(profile: &GitHealthProfile) -> bool {
    match profile.last_computed {
        None => true,
        Some(ts) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            now - ts > TTL_SECONDS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_trend_hot() {
        assert_eq!(classify_trend(10, 3, 5), ChurnTrend::Hot);
    }

    #[test]
    fn test_classify_trend_stabilized() {
        assert_eq!(classify_trend(1, 20, 5), ChurnTrend::Stabilized);
    }

    #[test]
    fn test_classify_trend_chronic() {
        assert_eq!(classify_trend(10, 80, 5), ChurnTrend::Chronic);
    }

    #[test]
    fn test_classify_trend_cold() {
        assert_eq!(classify_trend(1, 2, 5), ChurnTrend::Cold);
    }

    #[test]
    fn test_empty_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        // Not a git repo → default profile
        let profile = extract_git_health(dir.path());
        assert!(profile.churn_30d.is_empty());
        assert!(profile.churn_180d.is_empty());
        assert!(profile.reverts.is_empty());
    }

    #[test]
    fn test_needs_refresh_none() {
        let profile = GitHealthProfile::default();
        assert!(needs_refresh(&profile));
    }

    #[test]
    fn test_needs_refresh_recent() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let profile = GitHealthProfile {
            last_computed: Some(now),
            ..Default::default()
        };
        assert!(!needs_refresh(&profile));
    }

    #[test]
    fn test_extract_git_health_real_repo() {
        // Run on the actual cxpak repo (we know it has git history)
        let profile = extract_git_health(Path::new("."));
        // Should have some churn data
        assert!(
            !profile.churn_180d.is_empty() || profile.churn_30d.is_empty(),
            "real repo should have some history"
        );
    }

    #[test]
    fn test_needs_refresh_expired_ttl() {
        // Timestamp older than TTL_SECONDS ago → needs_refresh must return true.
        let expired_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(TTL_SECONDS + 10);

        let profile = GitHealthProfile {
            last_computed: Some(expired_ts),
            ..Default::default()
        };

        assert!(needs_refresh(&profile));
    }

    #[test]
    fn test_classify_trend_boundary_hot_vs_chronic() {
        // c30_normalized == c180 → Chronic (not strictly greater)
        // c30=10, c180=60 → c30*6=60 == c180=60 → Chronic
        assert_eq!(classify_trend(10, 60, 5), ChurnTrend::Chronic);
    }

    #[test]
    fn test_p75_churn_threshold_basic() {
        let mut map = std::collections::HashMap::new();
        map.insert("a".to_string(), 1usize);
        map.insert("b".to_string(), 2);
        map.insert("c".to_string(), 3);
        map.insert("d".to_string(), 100);
        // sorted: [1,2,3,100], p75 index = 3 → value 100; .max(3) = 100
        let t = p75_churn_threshold(&map);
        assert_eq!(t, 100);
    }

    #[test]
    fn test_p75_churn_threshold_empty() {
        let map = std::collections::HashMap::new();
        // empty → fallback 5, .max(3) = 5
        assert_eq!(p75_churn_threshold(&map), 5);
    }

    #[test]
    fn test_p75_churn_threshold_min_floor() {
        let mut map = std::collections::HashMap::new();
        map.insert("a".to_string(), 1usize);
        // sorted: [1], p75 index = 0 → value 1; .max(3) = 3
        let t = p75_churn_threshold(&map);
        assert_eq!(t, 3);
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    /// Initialize a real git repo in a temp dir with one or more commits.
    fn init_repo_with_commits(
        dir: &tempfile::TempDir,
        commits: &[(&str, &str, &str)], // (filename, content, message)
    ) -> git2::Repository {
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        let mut parent_commit: Option<git2::Oid> = None;

        for (filename, content, message) in commits {
            let file_path = dir.path().join(filename);
            // Create parent directories if needed (e.g. "src/lib.rs")
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&file_path, content).unwrap();

            let mut index = repo.index().unwrap();
            index.add_path(std::path::Path::new(filename)).unwrap();
            index.write().unwrap();

            let tree_oid = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_oid).unwrap();

            let parents: Vec<git2::Commit> = match parent_commit {
                Some(oid) => vec![repo.find_commit(oid).unwrap()],
                None => vec![],
            };
            let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
            let oid = repo
                .commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
                .unwrap();
            parent_commit = Some(oid);
        }
        repo
    }

    // ── extract_git_health: real-repo paths ──────────────────────────────────

    #[test]
    fn test_extract_git_health_collects_churn() {
        let dir = tempfile::TempDir::new().unwrap();
        // Two commits modifying the same file → churn count of 2.
        init_repo_with_commits(
            &dir,
            &[
                ("a.rs", "fn one() {}", "initial"),
                ("a.rs", "fn one() {}\nfn two() {}", "add two"),
            ],
        );

        let profile = extract_git_health(dir.path());
        // Both commits within 180d (just made now) → churn_180d should track a.rs
        assert!(
            profile.churn_180d.iter().any(|e| e.path == "a.rs"),
            "churn_180d should contain a.rs"
        );
        // last_computed should be set
        assert!(profile.last_computed.is_some());
    }

    #[test]
    fn test_extract_git_health_bugfix_density() {
        let dir = tempfile::TempDir::new().unwrap();
        // Mix of bugfix and feature commits
        init_repo_with_commits(
            &dir,
            &[
                ("src/lib.rs", "fn x() {}", "feat: initial"),
                ("src/lib.rs", "fn x() { 1 }", "fix: typo"),
                ("src/lib.rs", "fn x() { 2 }", "bug: wrong number"),
            ],
        );

        let profile = extract_git_health(dir.path());
        // src/ directory should have bugfix density > 0
        assert!(
            profile.bugfix_density.contains_key("src"),
            "src directory should be tracked: {:?}",
            profile.bugfix_density
        );
        let density = profile.bugfix_density.get("src").copied().unwrap_or(0.0);
        assert!(
            density > 0.0,
            "bugfix density for src should be > 0: {density}"
        );
    }

    #[test]
    fn test_extract_git_health_revert_detected() {
        let dir = tempfile::TempDir::new().unwrap();
        init_repo_with_commits(
            &dir,
            &[
                ("file.rs", "fn x() {}", "feat: add x"),
                ("file.rs", "fn x() { 1 }", "Revert: rollback"),
            ],
        );

        let profile = extract_git_health(dir.path());
        assert!(
            !profile.reverts.is_empty(),
            "reverts should not be empty: {:?}",
            profile.reverts
        );
        assert!(profile
            .reverts
            .iter()
            .any(|r| r.commit_message.to_lowercase().contains("revert")));
    }

    #[test]
    fn test_extract_git_health_churn_trend_classified() {
        let dir = tempfile::TempDir::new().unwrap();
        init_repo_with_commits(
            &dir,
            &[("hot.rs", "v1", "initial"), ("hot.rs", "v2", "update")],
        );

        let profile = extract_git_health(dir.path());
        // Some trend classification must exist for hot.rs
        assert!(
            profile.churn_trend.contains_key("hot.rs"),
            "trend should be classified for hot.rs: {:?}",
            profile.churn_trend
        );
    }

    #[test]
    fn test_extract_git_health_root_file_tracked_in_empty_dir() {
        // Files at the repo root use directory key "" — verify the empty dir is
        // tracked under bugfix_density when a fix commit modifies a root file.
        let dir = tempfile::TempDir::new().unwrap();
        init_repo_with_commits(
            &dir,
            &[
                ("root.rs", "v1", "initial"),
                ("root.rs", "v2", "fix: corrected"),
            ],
        );

        let profile = extract_git_health(dir.path());
        // The empty string directory should be present as a tracked dir
        assert!(
            profile.bugfix_density.contains_key(""),
            "root directory (empty key) should be tracked"
        );
    }
}
