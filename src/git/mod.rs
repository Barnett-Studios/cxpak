use std::collections::HashMap;
use std::path::Path;

/// Information about a single commit.
#[derive(Debug, Clone)]
pub struct CommitInfo {
    /// Abbreviated 7-character commit hash.
    pub hash: String,
    /// First line of the commit message.
    pub message: String,
    /// Author display name.
    pub author: String,
    /// Commit date formatted as YYYY-MM-DD (UTC).
    pub date: String,
}

/// Per-file churn metric: how many commits touched this file.
#[derive(Debug, Clone)]
pub struct FileChurn {
    pub path: String,
    pub commit_count: usize,
}

/// Contributor activity summary.
#[derive(Debug, Clone)]
pub struct ContributorInfo {
    pub name: String,
    pub commit_count: usize,
}

/// Aggregated git context for a repository.
#[derive(Debug)]
pub struct GitContext {
    /// Commits in reverse-chronological order (newest first), capped at `max_commits`.
    pub commits: Vec<CommitInfo>,
    /// Up to 20 most-churned files, sorted descending by commit count.
    pub file_churn: Vec<FileChurn>,
    /// Contributors sorted descending by commit count.
    pub contributors: Vec<ContributorInfo>,
}

/// Format a Unix timestamp (seconds since epoch) as "YYYY-MM-DD" without chrono.
fn format_date(unix_secs: i64) -> String {
    // Use div_euclid so that negative timestamps floor correctly (e.g. -1 → day -1,
    // not day 0 as plain `/` would give).  We then clamp to 0 so that any timestamp
    // before the Unix epoch renders as "1970-01-01" rather than a pre-1970 date.
    let days = unix_secs.div_euclid(86_400).max(0);

    // Compute year/month/day using the proleptic Gregorian calendar algorithm
    // Reference: https://en.wikipedia.org/wiki/Julian_day#Julian_day_number_calculation
    // We use the civil calendar algorithm from Howard Hinnant:
    // https://howardhinnant.github.io/date_algorithms.html#civil_from_days
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Extract git context from the repository at `repo_path`.
///
/// Walks up to `max_commits` commits from HEAD (newest first), diffs each
/// against its parent to collect file-churn statistics, and returns the
/// aggregated `GitContext`.
pub fn extract_git_context(
    repo_path: &Path,
    max_commits: usize,
) -> Result<GitContext, git2::Error> {
    let repo = git2::Repository::open(repo_path)?;

    let mut revwalk = repo.revwalk()?;
    match revwalk.push_head() {
        Ok(_) => {}
        Err(push_head_err) => {
            // Detached HEAD — push the commit HEAD points to directly.
            // If HEAD cannot be resolved (unborn branch / empty repo), propagate the
            // original error so callers get a meaningful failure.
            let fallback_ok = repo
                .head()
                .ok()
                .and_then(|h| h.peel_to_commit().ok())
                .map(|c| revwalk.push(c.id()))
                .is_some();
            if !fallback_ok {
                return Err(push_head_err);
            }
        }
    }
    revwalk.set_sorting(git2::Sort::TIME)?;

    let mut commits: Vec<CommitInfo> = Vec::new();
    let mut file_counts: HashMap<String, usize> = HashMap::new();
    let mut contributor_counts: HashMap<String, usize> = HashMap::new();

    for oid_result in revwalk.take(max_commits) {
        let oid = oid_result?;
        let commit = repo.find_commit(oid)?;

        let hash = format!("{:.7}", commit.id());
        let message = commit.summary().unwrap_or("").to_string();
        let author = commit.author().name().unwrap_or("Unknown").to_string();
        let date = format_date(commit.time().seconds());

        commits.push(CommitInfo {
            hash,
            message,
            author: author.clone(),
            date,
        });

        *contributor_counts.entry(author).or_insert(0) += 1;

        // Diff against first parent to collect changed files
        let parent_tree = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };
        let commit_tree = commit.tree()?;

        let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&commit_tree), None)?;

        diff.foreach(
            &mut |delta, _progress| {
                if let Some(path) = delta.new_file().path() {
                    let key = path.to_string_lossy().into_owned();
                    *file_counts.entry(key).or_insert(0) += 1;
                }
                true
            },
            None,
            None,
            None,
        )?;
    }

    // Build top-20 file churn list
    let mut file_churn: Vec<FileChurn> = file_counts
        .into_iter()
        .map(|(path, commit_count)| FileChurn { path, commit_count })
        .collect();
    file_churn.sort_by(|a, b| {
        b.commit_count
            .cmp(&a.commit_count)
            .then(a.path.cmp(&b.path))
    });
    file_churn.truncate(20);

    // Build contributor list
    let mut contributors: Vec<ContributorInfo> = contributor_counts
        .into_iter()
        .map(|(name, commit_count)| ContributorInfo { name, commit_count })
        .collect();
    contributors.sort_by(|a, b| {
        b.commit_count
            .cmp(&a.commit_count)
            .then(a.name.cmp(&b.name))
    });

    Ok(GitContext {
        commits,
        file_churn,
        contributors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Helper: create a commit in `repo` with the given files written to the
    /// worktree, a commit message, and an optional parent commit id.
    fn make_commit(
        repo: &git2::Repository,
        sig: &git2::Signature,
        message: &str,
        files: &[(&str, &str)],
        parent_id: Option<git2::Oid>,
    ) -> git2::Oid {
        let workdir = repo.workdir().expect("bare repo not supported in test");

        let mut index = repo.index().unwrap();

        for (name, content) in files {
            let file_path = workdir.join(name);
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&file_path, content).unwrap();
            index.add_path(Path::new(name)).unwrap();
        }

        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();

        let parents: Vec<git2::Commit> = match parent_id {
            Some(id) => vec![repo.find_commit(id).unwrap()],
            None => vec![],
        };
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();

        repo.commit(Some("HEAD"), sig, sig, message, &tree, &parent_refs)
            .unwrap()
    }

    #[test]
    fn test_extract_git_context() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        // git2 requires user.name / user.email for signatures; supply them
        // explicitly so the test does not depend on global git config.
        let sig = git2::Signature::now("Test User", "test@test.com").unwrap();

        // Commit 1: initial commit with file.txt
        let c1 = make_commit(
            &repo,
            &sig,
            "initial commit",
            &[("file.txt", "hello world")],
            None,
        );

        // Commit 2: second commit modifying file.txt and adding another.txt
        let _c2 = make_commit(
            &repo,
            &sig,
            "second commit",
            &[("file.txt", "updated content"), ("another.txt", "new file")],
            Some(c1),
        );

        let ctx = extract_git_context(dir.path(), 100).unwrap();

        // Should have exactly 2 commits
        assert_eq!(ctx.commits.len(), 2, "expected 2 commits");

        // Newest commit first (revwalk TIME order = newest first)
        assert_eq!(ctx.commits[0].message, "second commit");
        assert_eq!(ctx.commits[1].message, "initial commit");

        // Author should be present
        assert!(
            ctx.contributors.iter().any(|c| c.name == "Test User"),
            "expected 'Test User' contributor"
        );
        let contributor = ctx
            .contributors
            .iter()
            .find(|c| c.name == "Test User")
            .unwrap();
        assert_eq!(contributor.commit_count, 2);

        // file.txt was touched in both commits — highest churn
        let file_txt = ctx.file_churn.iter().find(|f| f.path == "file.txt");
        assert!(file_txt.is_some(), "file.txt should appear in churn list");
        assert_eq!(file_txt.unwrap().commit_count, 2);

        // another.txt was touched in one commit
        let another = ctx.file_churn.iter().find(|f| f.path == "another.txt");
        assert!(another.is_some(), "another.txt should appear in churn list");
        assert_eq!(another.unwrap().commit_count, 1);

        // Date fields should look like YYYY-MM-DD (10 chars, two dashes)
        for commit in &ctx.commits {
            assert_eq!(commit.date.len(), 10, "date '{}' wrong length", commit.date);
            let parts: Vec<&str> = commit.date.split('-').collect();
            assert_eq!(parts.len(), 3, "date '{}' missing dashes", commit.date);
        }
    }

    #[test]
    fn test_empty_repo_no_commits() {
        let dir = tempfile::TempDir::new().unwrap();
        let _repo = git2::Repository::init(dir.path()).unwrap();
        let result = extract_git_context(dir.path(), 100);
        assert!(result.is_err(), "expected error for repo with no commits");
    }

    #[test]
    fn test_single_commit() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Alice", "alice@test.com").unwrap();
        make_commit(&repo, &sig, "first", &[("hello.txt", "hi")], None);

        let ctx = extract_git_context(dir.path(), 100).unwrap();
        assert_eq!(ctx.commits.len(), 1);
        assert_eq!(ctx.commits[0].message, "first");
        assert_eq!(ctx.contributors.len(), 1);
        assert_eq!(ctx.contributors[0].name, "Alice");
    }

    #[test]
    fn test_max_commits_limit() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "t@t.com").unwrap();
        let c1 = make_commit(&repo, &sig, "c1", &[("a.txt", "1")], None);
        let c2 = make_commit(&repo, &sig, "c2", &[("a.txt", "2")], Some(c1));
        let _c3 = make_commit(&repo, &sig, "c3", &[("a.txt", "3")], Some(c2));

        let ctx = extract_git_context(dir.path(), 2).unwrap();
        assert_eq!(ctx.commits.len(), 2);
        // First commit is always HEAD (c3). The second depends on git2's
        // internal traversal when timestamps are identical, so we only
        // assert the limit is respected.
        assert_eq!(ctx.commits[0].message, "c3");
    }

    #[test]
    fn test_format_date() {
        assert_eq!(format_date(0), "1970-01-01");
        assert_eq!(format_date(-1), "1970-01-01");
        assert_eq!(format_date(1_700_000_000), "2023-11-14");
    }

    #[test]
    fn test_format_date_negative_timestamps_floor_to_epoch() {
        // All negative timestamps should render as 1970-01-01, not a wrong date
        // caused by rounding toward zero with plain `/`.
        assert_eq!(
            format_date(-86_399),
            "1970-01-01",
            "-86399s should floor to epoch"
        );
        assert_eq!(
            format_date(-86_400),
            "1970-01-01",
            "-86400s should floor to epoch"
        );
        assert_eq!(
            format_date(-1_000_000),
            "1970-01-01",
            "large negative should floor to epoch"
        );
    }

    #[test]
    fn test_multiple_contributors() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let alice = git2::Signature::now("Alice", "alice@test.com").unwrap();
        let bob = git2::Signature::now("Bob", "bob@test.com").unwrap();
        let c1 = make_commit(&repo, &alice, "by alice", &[("a.txt", "a")], None);
        let _c2 = make_commit(&repo, &bob, "by bob", &[("b.txt", "b")], Some(c1));

        let ctx = extract_git_context(dir.path(), 100).unwrap();
        assert_eq!(ctx.contributors.len(), 2);
    }

    #[test]
    fn test_file_churn_sorted() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "t@t.com").unwrap();
        let c1 = make_commit(
            &repo,
            &sig,
            "c1",
            &[("hot.txt", "1"), ("cold.txt", "1")],
            None,
        );
        let c2 = make_commit(&repo, &sig, "c2", &[("hot.txt", "2")], Some(c1));
        let _c3 = make_commit(&repo, &sig, "c3", &[("hot.txt", "3")], Some(c2));

        let ctx = extract_git_context(dir.path(), 100).unwrap();
        assert_eq!(ctx.file_churn[0].path, "hot.txt");
        assert_eq!(ctx.file_churn[0].commit_count, 3);
    }

    #[test]
    fn test_not_a_git_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = extract_git_context(dir.path(), 100);
        assert!(result.is_err());
    }

    #[test]
    fn test_detached_head_succeeds() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "t@t.com").unwrap();
        let c1 = make_commit(&repo, &sig, "initial", &[("a.txt", "1")], None);

        // Detach HEAD by pointing it directly at the commit object.
        repo.set_head_detached(c1).unwrap();

        let ctx = extract_git_context(dir.path(), 100).unwrap();
        assert_eq!(ctx.commits.len(), 1);
        assert_eq!(ctx.commits[0].message, "initial");
    }
}
