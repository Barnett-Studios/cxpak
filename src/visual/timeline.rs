//! Timeline snapshot computation for the Time Machine view.
//!
//! Walks the git commit log, samples up to `max_snapshots` commits,
//! and extracts lightweight metadata for each — commit SHA, date,
//! message, file list, and heuristic edge/module counts.  Full reparsing
//! is intentionally avoided to keep the operation fast.

use std::collections::HashSet;
use std::path::Path;

/// A lightweight snapshot of the codebase at a single git commit.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TimelineSnapshot {
    pub commit_sha: String,
    pub commit_date: String, // ISO 8601
    pub commit_message: String,
    pub files: Vec<SnapshotFile>,
    pub edge_count: usize,
    pub module_count: usize,
    pub health_composite: Option<f64>,
    pub circular_dep_count: usize,
}

/// A single file entry in a [`TimelineSnapshot`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnapshotFile {
    pub path: String,
    pub imports: Vec<String>,
}

/// Walk the commit log and sample up to `max_snapshots` commits evenly.
///
/// Commits are returned in **chronological order** (oldest first).
///
/// # Errors
/// Returns a `String` error message if the repository cannot be opened or the
/// revwalk fails.
pub fn compute_timeline_snapshots(
    repo_path: &Path,
    max_snapshots: usize,
) -> Result<Vec<TimelineSnapshot>, String> {
    let repo = git2::Repository::open(repo_path).map_err(|e| e.to_string())?;

    // Collect all commits via revwalk starting from HEAD.
    let mut revwalk = repo.revwalk().map_err(|e| e.to_string())?;
    revwalk.push_head().map_err(|e| e.to_string())?;
    revwalk
        .set_sorting(git2::Sort::TIME)
        .map_err(|e| e.to_string())?;

    let oids: Vec<git2::Oid> = revwalk.filter_map(|r| r.ok()).collect();

    if oids.is_empty() {
        return Ok(vec![]);
    }

    // Determine sampling stride so total committed ≤ max_snapshots.
    let total = oids.len();
    let stride = if max_snapshots == 0 {
        return Ok(vec![]);
    } else if total <= max_snapshots {
        1
    } else {
        total / max_snapshots
    };

    // Sample commits.  We want to include the most-recent (index 0) and step
    // backwards, then reverse for chronological order.
    let mut sampled_oids: Vec<git2::Oid> = oids
        .iter()
        .copied()
        .enumerate()
        .filter(|(i, _)| i % stride == 0)
        .take(max_snapshots)
        .map(|(_, oid)| oid)
        .collect();

    // Reverse to chronological order (oldest first).
    sampled_oids.reverse();

    let mut snapshots = Vec::with_capacity(sampled_oids.len());

    for oid in sampled_oids {
        let commit = repo.find_commit(oid).map_err(|e| e.to_string())?;

        let commit_sha = format!("{}", commit.id());
        let commit_date = {
            let t = commit.time();
            // git2 gives seconds since Unix epoch + offset in minutes.
            // We build an ISO 8601 UTC string without pulling in chrono.
            let secs = t.seconds();
            let s = secs.unsigned_abs();
            let y_rem = s % (365 * 24 * 3600); // very rough — good enough for display
            let _ = y_rem; // suppress unused warning; we use a simpler format below
            format_unix_timestamp(secs)
        };
        let commit_message = commit
            .message()
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("")
            .to_string();

        // List files in this commit's tree.
        let files = list_tree_files(&commit)?;

        // Heuristic edge count: pairs of files sharing the same directory are
        // likely connected.  Count files per directory and sum C(n,2) pairs.
        let edge_count = heuristic_edge_count(&files);

        // Module count: unique first-two-segment prefixes.
        let module_count = count_modules(&files);

        snapshots.push(TimelineSnapshot {
            commit_sha,
            commit_date,
            commit_message,
            files,
            edge_count,
            module_count,
            health_composite: None,
            circular_dep_count: 0,
        });
    }

    Ok(snapshots)
}

/// Format a Unix timestamp (seconds) as an ISO 8601 UTC string.
///
/// We implement a minimal version without external date libraries to avoid new
/// dependencies.  Accuracy is sufficient for display purposes.
fn format_unix_timestamp(secs: i64) -> String {
    // Use a simple approach: just emit the raw offset from epoch in a compact form.
    // We compute year/month/day/hour/min/sec via integer arithmetic.
    let secs_u = if secs < 0 { 0u64 } else { secs as u64 };

    let s_per_min: u64 = 60;
    let s_per_hour: u64 = 3600;
    let s_per_day: u64 = 86400;

    let sec = secs_u % s_per_min;
    let min = (secs_u / s_per_min) % 60;
    let hour = (secs_u / s_per_hour) % 24;
    let mut days = secs_u / s_per_day;

    // Compute year.
    let mut year: u64 = 1970;
    loop {
        let days_in_year: u64 = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    // Compute month.
    let leap = is_leap(year);
    let month_days: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month: u64 = 1;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    let day = days + 1;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Collect the paths of all blob entries in a commit's tree (recursive).
fn list_tree_files(commit: &git2::Commit<'_>) -> Result<Vec<SnapshotFile>, String> {
    let tree = commit.tree().map_err(|e| e.to_string())?;
    let mut files = Vec::new();

    tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
        if entry.kind() == Some(git2::ObjectType::Blob) {
            let name = entry.name().unwrap_or("");
            let path = if root.is_empty() {
                name.to_string()
            } else {
                format!("{root}{name}")
            };
            files.push(SnapshotFile {
                path,
                imports: vec![],
            });
        }
        git2::TreeWalkResult::Ok
    })
    .map_err(|e| e.to_string())?;

    Ok(files)
}

/// Heuristic edge count: for each directory, the number of ordered pairs of
/// files within it (i.e. files in the same directory are assumed connected).
fn heuristic_edge_count(files: &[SnapshotFile]) -> usize {
    let mut dir_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for f in files {
        let dir = f
            .path
            .rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_default();
        *dir_counts.entry(dir).or_insert(0) += 1;
    }

    dir_counts
        .values()
        .map(|&n| if n > 1 { n * (n - 1) / 2 } else { 0 })
        .sum()
}

/// Count unique module prefixes (first two path segments).
fn count_modules(files: &[SnapshotFile]) -> usize {
    let mut prefixes: HashSet<String> = HashSet::new();
    for f in files {
        let parts: Vec<&str> = f.path.splitn(3, '/').collect();
        let prefix = match parts.as_slice() {
            [a, b, _] => format!("{a}/{b}"),
            [a, _] => a.to_string(),
            [a] => a.to_string(),
            _ => continue,
        };
        prefixes.insert(prefix);
    }
    prefixes.len()
}

/// Try to read `.cxpak/timeline/snapshots.json`.  Returns `None` if the file
/// is missing or cannot be deserialised.
pub fn load_cached_snapshots(repo_path: &Path) -> Option<Vec<TimelineSnapshot>> {
    let path = repo_path.join(".cxpak/timeline/snapshots.json");
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Write snapshots to `.cxpak/timeline/snapshots.json`, creating the directory
/// if necessary.
pub fn save_snapshots(
    repo_path: &Path,
    snapshots: &[TimelineSnapshot],
) -> Result<(), std::io::Error> {
    let dir = repo_path.join(".cxpak/timeline");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("snapshots.json");
    let json = serde_json::to_string(snapshots).map_err(std::io::Error::other)?;
    std::fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeline_snapshot_roundtrip() {
        let snap = TimelineSnapshot {
            commit_sha: "abc123".into(),
            commit_date: "2026-01-01T00:00:00Z".into(),
            commit_message: "initial commit".into(),
            files: vec![SnapshotFile {
                path: "src/main.rs".into(),
                imports: vec![],
            }],
            edge_count: 0,
            module_count: 1,
            health_composite: Some(0.85),
            circular_dep_count: 0,
        };
        let json = serde_json::to_string(&snap).unwrap();
        let deserialized: TimelineSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.commit_sha, "abc123");
        assert_eq!(deserialized.files.len(), 1);
    }

    #[test]
    fn test_compute_timeline_snapshots_on_current_repo() {
        // This repo has commits — should succeed.
        let path = std::path::Path::new(".");
        let snapshots = compute_timeline_snapshots(path, 10).unwrap();
        assert!(!snapshots.is_empty());
        assert!(snapshots.len() <= 10);
        // Snapshots should be in chronological order.
        for window in snapshots.windows(2) {
            assert!(window[0].commit_date <= window[1].commit_date);
        }
    }

    #[test]
    fn test_snapshot_cache_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let snapshots = vec![TimelineSnapshot {
            commit_sha: "abc123".into(),
            commit_date: "2026-01-01T00:00:00Z".into(),
            commit_message: "test commit".into(),
            files: vec![SnapshotFile {
                path: "src/main.rs".into(),
                imports: vec!["src/lib.rs".into()],
            }],
            edge_count: 2,
            module_count: 1,
            health_composite: Some(0.85),
            circular_dep_count: 0,
        }];
        save_snapshots(dir.path(), &snapshots).unwrap();
        let loaded = load_cached_snapshots(dir.path()).expect("should load cached snapshots");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].commit_sha, "abc123");
        assert_eq!(loaded[0].files.len(), 1);
        assert_eq!(loaded[0].files[0].imports, vec!["src/lib.rs".to_string()]);
    }

    #[test]
    fn test_load_cached_snapshots_missing_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(load_cached_snapshots(dir.path()).is_none());
    }
}
