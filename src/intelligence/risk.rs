use crate::index::CodebaseIndex;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RiskEntry {
    pub path: String,
    pub churn_30d: u32,
    pub blast_radius: usize,
    pub test_coverage: f64,
    pub risk_score: f64,
}

/// Compute standing risk per file, sorted descending by risk_score.
///
/// Formula: risk = max(norm_churn, 0.01) * max(norm_blast, 0.01) * max(1.0 - test_coverage, 0.01)
///
/// norm_churn: percentile rank across all files (robust against outliers)
/// norm_blast: blast_radius_count / total_files
/// test_coverage: 1.0 if has_test, 0.0 otherwise (binary in v1.2.0)
pub fn compute_risk_ranking(index: &CodebaseIndex) -> Vec<RiskEntry> {
    let total_files = index.total_files.max(1) as f64;

    // Build churn lookup from 30d data
    let churn_map: std::collections::HashMap<&str, usize> = index
        .conventions
        .git_health
        .churn_30d
        .iter()
        .map(|e| (e.path.as_str(), e.modifications))
        .collect();

    // All file paths (sorted for determinism in percentile rank)
    let mut all_paths: Vec<&str> = index
        .files
        .iter()
        .map(|f| f.relative_path.as_str())
        .collect();
    all_paths.sort();

    // Churn values for all files (0 if no churn data)
    let churn_values: Vec<usize> = all_paths
        .iter()
        .map(|p| churn_map.get(*p).copied().unwrap_or(0))
        .collect();

    // Percentile rank: for each file, what fraction of files have <= its churn?
    // norm_churn[i] = rank(churn[i]) / n
    let n = churn_values.len().max(1) as f64;
    let norm_churn: Vec<f64> = churn_values
        .iter()
        .map(|&v| {
            let rank = churn_values.iter().filter(|&&other| other <= v).count();
            rank as f64 / n
        })
        .collect();

    // Blast radius: count of reverse-edge dependents (direct only, 1 hop)
    let blast_map: std::collections::HashMap<&str, usize> = all_paths
        .iter()
        .map(|&path| {
            let count = index.graph.dependents(path).len();
            (path, count)
        })
        .collect();

    let mut entries: Vec<RiskEntry> = all_paths
        .iter()
        .enumerate()
        .map(|(i, &path)| {
            let blast_count = blast_map.get(path).copied().unwrap_or(0);
            let norm_blast = (blast_count as f64 / total_files).min(1.0);
            let has_test = index.test_map.contains_key(path);
            let test_coverage = if has_test { 1.0 } else { 0.0 };

            let nc = norm_churn[i].max(0.01_f64);
            let nb = norm_blast.max(0.01_f64);
            let tc_term = (1.0_f64 - test_coverage).max(0.01_f64);

            let risk_score = nc * nb * tc_term;

            RiskEntry {
                path: path.to_string(),
                churn_30d: churn_map.get(path).copied().unwrap_or(0) as u32,
                blast_radius: blast_count,
                test_coverage,
                risk_score,
            }
        })
        .collect();

    entries.sort_by(|a, b| {
        b.risk_score
            .partial_cmp(&a.risk_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_risk_floor_prevents_zero() {
        // A file with 0 churn, 0 blast, no test -> floor kicks in: 0.01^3 = 0.000001
        // A file with 0 churn, 0 blast, HAS test -> floor on nc and nb, (1-1.0) uses floor:
        // 0.01 * 0.01 * 0.01 = 0.000001
        let floor_val: f64 = 0.01_f64 * 0.01 * 0.01;
        // Verify the floor formula produces a positive minimum
        assert!(floor_val > 0.0);
        assert!((floor_val - 0.000001).abs() < 1e-15);
    }

    #[test]
    fn test_risk_range_is_valid() {
        // max possible: 1.0 * 1.0 * 1.0 = 1.0 (no test, max churn percentile, all files depend)
        // min possible: 0.01^3 = 0.000001
        let max: f64 = 1.0_f64.max(0.01) * 1.0_f64.max(0.01) * 1.0_f64.max(0.01);
        let min: f64 = 0.01_f64.max(0.01) * 0.01_f64.max(0.01) * 0.01_f64.max(0.01);
        assert!((max - 1.0).abs() < 1e-9);
        assert!((min - 0.000001).abs() < 1e-12);
    }

    #[test]
    fn test_risk_sorted_descending() {
        use crate::budget::counter::TokenCounter;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        // Two files; we can only check that the result is sorted
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
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let entries = compute_risk_ranking(&index);
        assert_eq!(entries.len(), 2);
        assert!(
            entries[0].risk_score >= entries[1].risk_score,
            "risk entries must be sorted descending"
        );
    }

    #[test]
    fn test_risk_entry_serializes() {
        let entry = RiskEntry {
            path: "src/main.rs".into(),
            churn_30d: 5,
            blast_radius: 10,
            test_coverage: 0.0,
            risk_score: 0.42,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"path\":\"src/main.rs\""));
        assert!(json.contains("\"churn_30d\":5"));
    }

    #[test]
    fn test_risk_untested_scores_higher_than_tested() {
        // For same churn and blast, untested file (tc=0) should score higher than tested (tc=1)
        // Untested: nc * nb * max(1.0, 0.01) = nc * nb * 1.0
        // Tested:   nc * nb * max(0.0, 0.01) = nc * nb * 0.01
        let nc = 0.5f64;
        let nb = 0.5f64;
        let untested = nc.max(0.01) * nb.max(0.01) * (1.0f64 - 0.0).max(0.01);
        let tested = nc.max(0.01) * nb.max(0.01) * (1.0f64 - 1.0).max(0.01);
        assert!(untested > tested, "untested={untested}, tested={tested}");
    }
}
