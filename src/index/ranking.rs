use std::collections::HashMap;

use crate::git::GitContext;
use crate::index::graph::DependencyGraph;

#[derive(Debug, Clone)]
pub struct FileScore {
    pub path: String,
    pub in_degree: usize,
    pub out_degree: usize,
    pub git_recency: f64,
    pub git_churn: f64,
    pub composite: f64,
}

/// Compute importance scores for all files.
///
/// Weights: in_degree * 0.4 + out_degree * 0.1 + git_recency * 0.3 + git_churn * 0.2
pub fn rank_files(
    file_paths: &[String],
    graph: &DependencyGraph,
    git_context: Option<&GitContext>,
) -> Vec<FileScore> {
    let churn_map: HashMap<&str, usize> = git_context
        .map(|g| {
            g.file_churn
                .iter()
                .map(|f| (f.path.as_str(), f.commit_count))
                .collect()
        })
        .unwrap_or_default();

    let max_churn = churn_map.values().copied().max().unwrap_or(1) as f64;

    // Recency: 1.0 if there are recent commits, 0.0 otherwise.
    // A more sophisticated version could track per-file last-modified dates.
    let git_recency = git_context
        .and_then(|g| g.commits.first())
        .map(|_| 1.0)
        .unwrap_or(0.0);

    file_paths
        .iter()
        .map(|path| {
            let in_degree = graph.dependents(path).len();
            let out_degree = graph.dependencies(path).map(|d| d.len()).unwrap_or(0);
            let file_churn = churn_map.get(path.as_str()).copied().unwrap_or(0) as f64 / max_churn;

            let composite = in_degree as f64 * 0.4
                + out_degree as f64 * 0.1
                + git_recency * 0.3
                + file_churn * 0.2;

            FileScore {
                path: path.clone(),
                in_degree,
                out_degree,
                git_recency,
                git_churn: file_churn,
                composite,
            }
        })
        .collect()
}

/// Apply focus boost: 2x for files under `focus_path`, 1.5x for their direct dependencies.
pub fn apply_focus(scores: &mut [FileScore], focus_path: &str, graph: &DependencyGraph) {
    let focus_files: Vec<String> = scores
        .iter()
        .filter(|s| s.path.starts_with(focus_path))
        .map(|s| s.path.clone())
        .collect();

    let mut dep_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in &focus_files {
        if let Some(deps) = graph.dependencies(f) {
            dep_files.extend(deps.iter().cloned());
        }
        for dep in graph.dependents(f) {
            dep_files.insert(dep.to_string());
        }
    }
    for f in &focus_files {
        dep_files.remove(f);
    }

    for score in scores.iter_mut() {
        if focus_files.contains(&score.path) {
            score.composite *= 2.0;
        } else if dep_files.contains(&score.path) {
            score.composite *= 1.5;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{CommitInfo, ContributorInfo, FileChurn};

    fn make_git_context(file_churns: Vec<(&str, usize)>, dates: Vec<&str>) -> GitContext {
        GitContext {
            commits: dates
                .into_iter()
                .enumerate()
                .map(|(i, d)| CommitInfo {
                    hash: format!("{:07}", i),
                    message: format!("commit {}", i),
                    author: "Test".into(),
                    date: d.to_string(),
                })
                .collect(),
            file_churn: file_churns
                .into_iter()
                .map(|(p, c)| FileChurn {
                    path: p.to_string(),
                    commit_count: c,
                })
                .collect(),
            contributors: vec![ContributorInfo {
                name: "Test".into(),
                commit_count: 1,
            }],
        }
    }

    #[test]
    fn test_rank_files_basic() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("app.rs", "lib.rs");
        graph.add_edge("cli.rs", "lib.rs");

        let paths = vec!["app.rs".into(), "cli.rs".into(), "lib.rs".into()];
        let scores = rank_files(&paths, &graph, None);

        assert_eq!(scores.len(), 3);
        let lib_score = scores.iter().find(|s| s.path == "lib.rs").unwrap();
        let app_score = scores.iter().find(|s| s.path == "app.rs").unwrap();
        assert!(
            lib_score.composite > app_score.composite,
            "lib.rs should rank higher (more dependents)"
        );
        assert_eq!(lib_score.in_degree, 2);
    }

    #[test]
    fn test_rank_files_with_git() {
        let graph = DependencyGraph::new();
        let git = make_git_context(vec![("hot.rs", 10), ("cold.rs", 1)], vec!["2026-03-12"]);

        let paths = vec!["hot.rs".into(), "cold.rs".into()];
        let scores = rank_files(&paths, &graph, Some(&git));

        let hot = scores.iter().find(|s| s.path == "hot.rs").unwrap();
        let cold = scores.iter().find(|s| s.path == "cold.rs").unwrap();
        assert!(hot.git_churn > cold.git_churn);
        assert!(hot.composite > cold.composite);
    }

    #[test]
    fn test_rank_files_empty() {
        let graph = DependencyGraph::new();
        let scores = rank_files(&[], &graph, None);
        assert!(scores.is_empty());
    }

    #[test]
    fn test_rank_files_no_graph_no_git() {
        let graph = DependencyGraph::new();
        let paths = vec!["a.rs".into(), "b.rs".into()];
        let scores = rank_files(&paths, &graph, None);
        assert_eq!(scores.len(), 2);
        for s in &scores {
            assert_eq!(s.composite, 0.0);
        }
    }

    #[test]
    fn test_apply_focus() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("src/auth/mod.rs", "src/db/users.rs");

        let mut scores = vec![
            FileScore {
                path: "src/auth/mod.rs".into(),
                in_degree: 0,
                out_degree: 1,
                git_recency: 0.0,
                git_churn: 0.0,
                composite: 1.0,
            },
            FileScore {
                path: "src/db/users.rs".into(),
                in_degree: 1,
                out_degree: 0,
                git_recency: 0.0,
                git_churn: 0.0,
                composite: 0.5,
            },
            FileScore {
                path: "src/other.rs".into(),
                in_degree: 0,
                out_degree: 0,
                git_recency: 0.0,
                git_churn: 0.0,
                composite: 0.3,
            },
        ];

        apply_focus(&mut scores, "src/auth", &graph);

        let auth = scores.iter().find(|s| s.path == "src/auth/mod.rs").unwrap();
        let db = scores.iter().find(|s| s.path == "src/db/users.rs").unwrap();
        let other = scores.iter().find(|s| s.path == "src/other.rs").unwrap();

        assert!(
            (auth.composite - 2.0).abs() < 0.01,
            "focus path should be 2x: {}",
            auth.composite
        );
        assert!(
            (db.composite - 0.75).abs() < 0.01,
            "dependency should be 1.5x: {}",
            db.composite
        );
        assert!(
            (other.composite - 0.3).abs() < 0.01,
            "unrelated should be unchanged: {}",
            other.composite
        );
    }

    #[test]
    fn test_apply_focus_no_match() {
        let graph = DependencyGraph::new();
        let mut scores = vec![FileScore {
            path: "a.rs".into(),
            in_degree: 0,
            out_degree: 0,
            git_recency: 0.0,
            git_churn: 0.0,
            composite: 1.0,
        }];
        let original = scores[0].composite;
        apply_focus(&mut scores, "nonexistent/path", &graph);
        assert_eq!(scores[0].composite, original, "should be unchanged");
    }
}
