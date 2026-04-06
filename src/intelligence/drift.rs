use serde::{Deserialize, Serialize};
use std::path::Path;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureMetrics {
    pub module_count: usize,
    pub mean_coupling: f64,
    pub mean_cohesion: f64,
    pub cycle_count: usize,
    pub boundary_violation_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureSnapshot {
    pub timestamp: String,
    pub metrics: ArchitectureMetrics,
    pub modules: Vec<SnapshotModule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotModule {
    pub prefix: String,
    pub coupling: f64,
    pub cohesion: f64,
    pub edge_count: usize,
}

#[derive(Debug, Serialize)]
pub struct MetricDeltas {
    pub coupling_delta: f64,
    pub cohesion_delta: f64,
    pub new_cycles: i64,
    pub new_boundary_violations: i64,
    pub module_count_delta: i64,
}

#[derive(Debug, Serialize)]
pub struct BaselineComparison {
    pub baseline_date: String,
    pub metrics_then: ArchitectureMetrics,
    pub metrics_now: ArchitectureMetrics,
    pub deltas: MetricDeltas,
}

#[derive(Debug, Serialize)]
pub struct TrendComparison {
    pub window_recent: String,
    pub window_baseline: String,
    pub coupling_trend: f64,
    pub cohesion_trend: f64,
    pub new_cycles: Vec<Vec<String>>,
    pub new_cross_module_imports: Vec<crate::intelligence::architecture::BoundaryViolation>,
}

#[derive(Debug, Serialize)]
pub struct DriftHotspot {
    pub module: String,
    pub issue: String,
    pub severity: f64,
    pub contributing_commits: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DriftReport {
    pub baseline: Option<BaselineComparison>,
    pub trend: Option<TrendComparison>,
    pub hotspots: Vec<DriftHotspot>,
}

// ---------------------------------------------------------------------------
// Snapshot logic
// ---------------------------------------------------------------------------

pub fn snapshot_filename(timestamp: &str) -> String {
    let safe = timestamp.replace(':', "-");
    format!("snapshot-{safe}.json")
}

pub fn compute_metric_deltas(
    then: &ArchitectureMetrics,
    now: &ArchitectureMetrics,
) -> MetricDeltas {
    MetricDeltas {
        coupling_delta: now.mean_coupling - then.mean_coupling,
        cohesion_delta: now.mean_cohesion - then.mean_cohesion,
        new_cycles: now.cycle_count as i64 - then.cycle_count as i64,
        new_boundary_violations: now.boundary_violation_count as i64
            - then.boundary_violation_count as i64,
        module_count_delta: now.module_count as i64 - then.module_count as i64,
    }
}

pub fn compute_trend(
    baseline: &ArchitectureSnapshot,
    current: &ArchitectureSnapshot,
) -> TrendComparison {
    TrendComparison {
        window_recent: "last 30 days".to_string(),
        window_baseline: "30-180 days ago".to_string(),
        coupling_trend: current.metrics.mean_coupling - baseline.metrics.mean_coupling,
        cohesion_trend: current.metrics.mean_cohesion - baseline.metrics.mean_cohesion,
        new_cycles: vec![],
        new_cross_module_imports: vec![],
    }
}

/// Select the most recent and oldest snapshot to compute a trend.
/// Returns None when < 2 snapshots exist.
pub fn compute_trend_from_snapshots(snapshots: &[ArchitectureSnapshot]) -> Option<TrendComparison> {
    if snapshots.len() < 2 {
        return None;
    }
    let current = &snapshots[0];
    let baseline = &snapshots[snapshots.len() - 1];
    Some(compute_trend(baseline, current))
}

// ---------------------------------------------------------------------------
// Persistence helpers
// ---------------------------------------------------------------------------

pub fn save_snapshot(
    repo_root: &Path,
    snapshot: &ArchitectureSnapshot,
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = repo_root.join(".cxpak").join("snapshots");
    std::fs::create_dir_all(&dir)?;
    let filename = snapshot_filename(&snapshot.timestamp);
    let path = dir.join(filename);
    let json = serde_json::to_string_pretty(snapshot)?;
    std::fs::write(path, json)?;
    Ok(())
}

pub fn load_snapshots(repo_root: &Path) -> Vec<ArchitectureSnapshot> {
    let dir = repo_root.join(".cxpak").join("snapshots");
    if !dir.exists() {
        return vec![];
    }
    let mut snapshots: Vec<ArchitectureSnapshot> = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension()?.to_str()? != "json" {
                return None;
            }
            let content = std::fs::read_to_string(&path).ok()?;
            serde_json::from_str(&content).ok()
        })
        .collect();
    snapshots.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    snapshots
}

pub fn load_baseline(repo_root: &Path) -> Option<ArchitectureSnapshot> {
    let path = repo_root.join(".cxpak").join("baseline.json");
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn save_baseline(
    repo_root: &Path,
    snapshot: &ArchitectureSnapshot,
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = repo_root.join(".cxpak");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("baseline.json");
    let json = serde_json::to_string_pretty(snapshot)?;
    std::fs::write(path, json)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Snapshot from index
// ---------------------------------------------------------------------------

pub fn snapshot_from_index(
    index: &crate::index::CodebaseIndex,
    timestamp: &str,
) -> ArchitectureSnapshot {
    use std::collections::HashMap;

    let mut module_files: HashMap<String, Vec<String>> = HashMap::new();
    for file in &index.files {
        let prefix = module_prefix(&file.relative_path, 2);
        module_files
            .entry(prefix)
            .or_default()
            .push(file.relative_path.clone());
    }

    let module_count = module_files.len();
    let mut coupling_sum = 0.0;
    let mut cohesion_sum = 0.0;
    let mut module_count_qualifying = 0usize;
    let mut snapshot_modules: Vec<SnapshotModule> = vec![];

    for (prefix, files) in &module_files {
        if files.len() < 3 {
            continue;
        }
        let file_set: std::collections::HashSet<&str> = files.iter().map(|s| s.as_str()).collect();

        let mut intra_edges = 0usize;
        let mut cross_edges = 0usize;

        for file in files {
            if let Some(deps) = index.graph.dependencies(file) {
                for dep in deps {
                    if file_set.contains(dep.target.as_str()) {
                        intra_edges += 1;
                    } else {
                        cross_edges += 1;
                    }
                }
            }
        }

        let total_edges = intra_edges + cross_edges;
        let coupling = if total_edges == 0 {
            0.0
        } else {
            cross_edges as f64 / total_edges as f64
        };

        let max_intra = files.len() * files.len().saturating_sub(1);
        let cohesion = if max_intra == 0 {
            0.0
        } else {
            intra_edges as f64 / max_intra as f64
        };

        coupling_sum += coupling;
        cohesion_sum += cohesion;
        module_count_qualifying += 1;

        snapshot_modules.push(SnapshotModule {
            prefix: prefix.clone(),
            coupling,
            cohesion,
            edge_count: total_edges,
        });
    }

    let mean_coupling = if module_count_qualifying == 0 {
        0.0
    } else {
        coupling_sum / module_count_qualifying as f64
    };
    let mean_cohesion = if module_count_qualifying == 0 {
        0.0
    } else {
        cohesion_sum / module_count_qualifying as f64
    };

    ArchitectureSnapshot {
        timestamp: timestamp.to_string(),
        metrics: ArchitectureMetrics {
            module_count,
            mean_coupling,
            mean_cohesion,
            cycle_count: 0,
            boundary_violation_count: 0,
        },
        modules: snapshot_modules,
    }
}

fn module_prefix(path: &str, depth: usize) -> String {
    path.split('/').take(depth).collect::<Vec<_>>().join("/")
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

pub fn build_drift_report(
    index: &crate::index::CodebaseIndex,
    repo_root: &Path,
    save_baseline_flag: bool,
) -> DriftReport {
    let now = chrono::Utc::now().to_rfc3339();
    let current_snapshot = snapshot_from_index(index, &now);

    if save_baseline_flag {
        let _ = save_baseline(repo_root, &current_snapshot);
    }

    let _ = save_snapshot(repo_root, &current_snapshot);

    let baseline_comparison = load_baseline(repo_root).map(|baseline_snap| {
        let deltas = compute_metric_deltas(&baseline_snap.metrics, &current_snapshot.metrics);
        BaselineComparison {
            baseline_date: baseline_snap.timestamp.clone(),
            metrics_then: baseline_snap.metrics,
            metrics_now: current_snapshot.metrics.clone(),
            deltas,
        }
    });

    let snapshots = load_snapshots(repo_root);
    let trend = compute_trend_from_snapshots(&snapshots);

    let hotspots: Vec<DriftHotspot> = current_snapshot
        .modules
        .iter()
        .filter(|m| m.coupling > 0.6)
        .map(|m| DriftHotspot {
            module: m.prefix.clone(),
            issue: format!("High coupling: {:.2}", m.coupling),
            severity: m.coupling,
            contributing_commits: vec![],
        })
        .collect();

    DriftReport {
        baseline: baseline_comparison,
        trend,
        hotspots,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_snapshot(coupling: f64, cohesion: f64, cycle_count: usize) -> ArchitectureSnapshot {
        ArchitectureSnapshot {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            metrics: ArchitectureMetrics {
                module_count: 5,
                mean_coupling: coupling,
                mean_cohesion: cohesion,
                cycle_count,
                boundary_violation_count: 2,
            },
            modules: vec![],
        }
    }

    #[test]
    fn test_snapshot_serialization_roundtrip() {
        let snap = make_snapshot(0.3, 0.7, 2);
        let json = serde_json::to_string(&snap).unwrap();
        let decoded: ArchitectureSnapshot = serde_json::from_str(&json).unwrap();
        assert!((decoded.metrics.mean_coupling - 0.3).abs() < 1e-9);
        assert_eq!(decoded.metrics.cycle_count, 2);
    }

    #[test]
    fn test_trend_coupling_worsening() {
        let old = make_snapshot(0.2, 0.8, 0);
        let new = make_snapshot(0.5, 0.6, 1);
        let trend = compute_trend(&old, &new);
        assert!(
            trend.coupling_trend > 0.0,
            "coupling increased → positive trend (worse)"
        );
        assert!(
            trend.cohesion_trend < 0.0,
            "cohesion decreased → negative trend (worse)"
        );
    }

    #[test]
    fn test_trend_improving() {
        let old = make_snapshot(0.5, 0.4, 3);
        let new = make_snapshot(0.2, 0.7, 1);
        let trend = compute_trend(&old, &new);
        assert!(trend.coupling_trend < 0.0, "coupling decreased → improving");
        assert!(trend.cohesion_trend > 0.0, "cohesion increased → improving");
    }

    #[test]
    fn test_insufficient_history_returns_null_trend() {
        let snapshots: Vec<ArchitectureSnapshot> = vec![];
        let result = compute_trend_from_snapshots(&snapshots);
        assert!(result.is_none(), "no snapshots → trend must be None");
    }

    #[test]
    fn test_snapshot_filename_contains_timestamp() {
        let name = snapshot_filename("2026-03-15T12:00:00Z");
        assert!(name.contains("2026-03-15"), "filename must embed date");
        assert!(name.ends_with(".json"), "must be .json");
    }

    #[test]
    fn test_snapshot_filename_no_colons() {
        let name = snapshot_filename("2026-01-01T12:30:00Z");
        assert!(!name.contains(':'), "filename must not contain colons");
    }

    #[test]
    fn test_baseline_comparison_deltas() {
        let then = ArchitectureMetrics {
            module_count: 4,
            mean_coupling: 0.2,
            mean_cohesion: 0.8,
            cycle_count: 0,
            boundary_violation_count: 1,
        };
        let now = ArchitectureMetrics {
            module_count: 6,
            mean_coupling: 0.4,
            mean_cohesion: 0.6,
            cycle_count: 2,
            boundary_violation_count: 3,
        };
        let deltas = compute_metric_deltas(&then, &now);
        assert!((deltas.coupling_delta - 0.2).abs() < 1e-9);
        assert!((deltas.cohesion_delta - (-0.2)).abs() < 1e-9);
        assert_eq!(deltas.new_cycles, 2);
        assert_eq!(deltas.new_boundary_violations, 2);
    }

    #[test]
    fn test_metric_deltas_zero_when_same() {
        let m = ArchitectureMetrics {
            module_count: 5,
            mean_coupling: 0.3,
            mean_cohesion: 0.7,
            cycle_count: 1,
            boundary_violation_count: 2,
        };
        let deltas = compute_metric_deltas(&m, &m);
        assert_eq!(deltas.new_cycles, 0);
        assert!((deltas.coupling_delta).abs() < 1e-9);
    }

    #[test]
    fn test_snapshot_load_save_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let snap = make_snapshot(0.3, 0.7, 1);
        save_snapshot(dir.path(), &snap).unwrap();
        let loaded = load_snapshots(dir.path());
        assert_eq!(loaded.len(), 1);
        assert!((loaded[0].metrics.mean_coupling - 0.3).abs() < 1e-9);
    }

    #[test]
    fn test_baseline_save_and_load() {
        let dir = tempfile::TempDir::new().unwrap();
        let snap = make_snapshot(0.4, 0.6, 2);
        save_baseline(dir.path(), &snap).unwrap();
        let loaded = load_baseline(dir.path());
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().metrics.cycle_count, 2);
    }

    #[test]
    fn test_baseline_absent_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = load_baseline(dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_load_snapshots_nonexistent_dir_returns_empty() {
        let result = load_snapshots(std::path::Path::new("/nonexistent/snapshots"));
        assert!(result.is_empty());
    }

    #[test]
    fn test_module_prefix_depth_two() {
        assert_eq!(
            module_prefix("src/intelligence/predict.rs", 2),
            "src/intelligence"
        );
        assert_eq!(module_prefix("main.rs", 2), "main.rs");
        assert_eq!(module_prefix("src/lib.rs", 2), "src/lib.rs");
    }
}
