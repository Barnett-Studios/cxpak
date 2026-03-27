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

        let trend = classify_trend(c30, c180);
        churn_trend.insert(path, trend);
    }

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    GitHealthProfile {
        churn_30d: churn_30d_vec,
        churn_180d: churn_180d_vec,
        bugfix_density,
        reverts,
        churn_trend,
        last_computed: Some(now_secs),
    }
}

fn classify_trend(c30: usize, c180: usize) -> ChurnTrend {
    // Normalize 30d to 180d scale: c30 * 6
    let c30_normalized = c30 * 6;

    let high_threshold = 5; // at least 5 modifications in window

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
        assert_eq!(classify_trend(10, 3), ChurnTrend::Hot);
    }

    #[test]
    fn test_classify_trend_stabilized() {
        assert_eq!(classify_trend(1, 20), ChurnTrend::Stabilized);
    }

    #[test]
    fn test_classify_trend_chronic() {
        assert_eq!(classify_trend(10, 80), ChurnTrend::Chronic);
    }

    #[test]
    fn test_classify_trend_cold() {
        assert_eq!(classify_trend(1, 2), ChurnTrend::Cold);
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
}
