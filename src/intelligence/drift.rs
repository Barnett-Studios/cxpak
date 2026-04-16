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
    let path = dir.join(&filename);
    let json = serde_json::to_string_pretty(snapshot)?;
    // Atomic write: write to a tmp file then rename to prevent partial reads.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &path)?;

    // Cap snapshots directory at 100 entries: remove the oldest beyond the limit.
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    files.sort_by_key(|e| e.file_name());
    while files.len() > 100 {
        if let Some(oldest) = files.first() {
            let _ = std::fs::remove_file(oldest.path());
        }
        files.remove(0);
    }
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
    snapshots.sort_by(|a, b| {
        let pa = chrono::DateTime::parse_from_rfc3339(&a.timestamp).ok();
        let pb = chrono::DateTime::parse_from_rfc3339(&b.timestamp).ok();
        match (pa, pb) {
            (Some(ta), Some(tb)) => tb.cmp(&ta), // descending: newest first
            _ => b.timestamp.cmp(&a.timestamp),  // fallback to lexicographic
        }
    });
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
    // Atomic write: write to a tmp file then rename to prevent partial reads.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &path)?;
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

    let module_count = module_count_qualifying;

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

    #[test]
    fn test_snapshot_from_index_empty() {
        let index = crate::index::CodebaseIndex::empty();
        let snap = snapshot_from_index(&index, "2026-04-01T00:00:00Z");
        assert_eq!(snap.timestamp, "2026-04-01T00:00:00Z");
        assert_eq!(snap.metrics.module_count, 0);
        assert!((snap.metrics.mean_coupling - 0.0).abs() < 1e-9);
        assert!((snap.metrics.mean_cohesion - 0.0).abs() < 1e-9);
        assert!(snap.modules.is_empty());
    }

    #[test]
    fn test_snapshot_from_index_with_files_and_edges() {
        use crate::index::{CodebaseIndex, IndexedFile};
        use crate::schema::EdgeType;

        let mut index = CodebaseIndex::empty();

        // Create a module with 3+ files so it qualifies for coupling/cohesion
        for name in &[
            "src/api/handler.rs",
            "src/api/router.rs",
            "src/api/middleware.rs",
        ] {
            index.files.push(IndexedFile {
                relative_path: name.to_string(),
                language: Some("rust".into()),
                size_bytes: 100,
                token_count: 50,
                parse_result: None,
                content: "fn f() {}".to_string(),
                mtime_secs: None,
            });
        }

        // Add an intra-module edge (both within src/api)
        index
            .graph
            .add_edge("src/api/handler.rs", "src/api/router.rs", EdgeType::Import);
        // Add a cross-module edge (handler -> external)
        index
            .graph
            .add_edge("src/api/handler.rs", "src/db/query.rs", EdgeType::Import);

        let snap = snapshot_from_index(&index, "2026-04-01T12:00:00Z");
        assert_eq!(snap.metrics.module_count, 1, "one module prefix (src/api)");
        assert_eq!(snap.modules.len(), 1, "one qualifying module");
        let m = &snap.modules[0];
        assert_eq!(m.prefix, "src/api");
        assert_eq!(m.edge_count, 2, "1 intra + 1 cross = 2 total edges");
        // coupling = cross / total = 1/2 = 0.5
        assert!((m.coupling - 0.5).abs() < 1e-9);
        // cohesion = intra / (n*(n-1)) = 1 / (3*2) = 1/6
        assert!((m.cohesion - 1.0 / 6.0).abs() < 1e-9);
        assert!((snap.metrics.mean_coupling - 0.5).abs() < 1e-9);
        assert!((snap.metrics.mean_cohesion - 1.0 / 6.0).abs() < 1e-9);
    }

    #[test]
    fn test_snapshot_from_index_module_below_min_files_skipped() {
        use crate::index::{CodebaseIndex, IndexedFile};

        let mut index = CodebaseIndex::empty();

        // Only 2 files in same prefix -> below the "< 3" threshold
        for name in &["src/tiny/a.rs", "src/tiny/b.rs"] {
            index.files.push(IndexedFile {
                relative_path: name.to_string(),
                language: Some("rust".into()),
                size_bytes: 50,
                token_count: 25,
                parse_result: None,
                content: "fn f() {}".to_string(),
                mtime_secs: None,
            });
        }

        let snap = snapshot_from_index(&index, "2026-04-02T00:00:00Z");
        // module_count reflects qualifying modules only (same denominator as mean_coupling)
        assert_eq!(
            snap.metrics.module_count, 0,
            "no qualifying modules (< 3 files)"
        );
        // modules vec is empty
        assert!(
            snap.modules.is_empty(),
            "modules with < 3 files must be skipped"
        );
        assert!((snap.metrics.mean_coupling - 0.0).abs() < 1e-9);
        assert!((snap.metrics.mean_cohesion - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_snapshot_from_index_no_edges_zero_coupling() {
        use crate::index::{CodebaseIndex, IndexedFile};

        let mut index = CodebaseIndex::empty();

        // 3 files in same module but no edges -> coupling 0, cohesion 0
        for name in &["src/lib/a.rs", "src/lib/b.rs", "src/lib/c.rs"] {
            index.files.push(IndexedFile {
                relative_path: name.to_string(),
                language: Some("rust".into()),
                size_bytes: 100,
                token_count: 50,
                parse_result: None,
                content: "fn f() {}".to_string(),
                mtime_secs: None,
            });
        }

        let snap = snapshot_from_index(&index, "2026-04-03T00:00:00Z");
        assert_eq!(snap.modules.len(), 1);
        let m = &snap.modules[0];
        assert_eq!(m.edge_count, 0);
        assert!((m.coupling - 0.0).abs() < 1e-9);
        assert!((m.cohesion - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_build_drift_report_with_empty_index() {
        let index = crate::index::CodebaseIndex::empty();
        let dir = tempfile::TempDir::new().unwrap();
        let report = build_drift_report(&index, dir.path(), false);
        // No baseline saved before -> baseline comparison is None
        assert!(report.baseline.is_none());
        // Only one snapshot saved -> trend needs >= 2
        assert!(report.trend.is_none());
        assert!(report.hotspots.is_empty());
    }

    #[test]
    fn test_build_drift_report_saves_baseline_when_flagged() {
        let index = crate::index::CodebaseIndex::empty();
        let dir = tempfile::TempDir::new().unwrap();
        let _report = build_drift_report(&index, dir.path(), true);
        // Baseline file should now exist
        let baseline = load_baseline(dir.path());
        assert!(
            baseline.is_some(),
            "baseline must be saved when save_baseline_flag is true"
        );
    }

    #[test]
    fn test_build_drift_report_hotspots_from_high_coupling() {
        use crate::index::{CodebaseIndex, IndexedFile};
        use crate::schema::EdgeType;

        let mut index = CodebaseIndex::empty();

        // Create a module with 3 files, mostly cross-module edges -> high coupling
        for name in &["src/hot/a.rs", "src/hot/b.rs", "src/hot/c.rs"] {
            index.files.push(IndexedFile {
                relative_path: name.to_string(),
                language: Some("rust".into()),
                size_bytes: 100,
                token_count: 50,
                parse_result: None,
                content: "fn f() {}".to_string(),
                mtime_secs: None,
            });
        }

        // All edges are cross-module -> coupling = 1.0 (> 0.6 threshold)
        index
            .graph
            .add_edge("src/hot/a.rs", "src/other/x.rs", EdgeType::Import);
        index
            .graph
            .add_edge("src/hot/b.rs", "src/other/y.rs", EdgeType::Import);
        index
            .graph
            .add_edge("src/hot/c.rs", "src/other/z.rs", EdgeType::Import);

        let dir = tempfile::TempDir::new().unwrap();
        let report = build_drift_report(&index, dir.path(), false);
        assert_eq!(
            report.hotspots.len(),
            1,
            "module with coupling > 0.6 must appear as hotspot"
        );
        assert_eq!(report.hotspots[0].module, "src/hot");
        assert!(report.hotspots[0].severity > 0.6);
    }

    #[test]
    fn test_build_drift_report_with_existing_baseline() {
        use crate::index::{CodebaseIndex, IndexedFile};

        let dir = tempfile::TempDir::new().unwrap();

        // Save a baseline with known metrics
        let baseline_snap = ArchitectureSnapshot {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            metrics: ArchitectureMetrics {
                module_count: 2,
                mean_coupling: 0.1,
                mean_cohesion: 0.9,
                cycle_count: 0,
                boundary_violation_count: 0,
            },
            modules: vec![],
        };
        save_baseline(dir.path(), &baseline_snap).unwrap();

        // Build report with empty index (different metrics)
        let mut index = CodebaseIndex::empty();
        for name in &["src/mod/a.rs", "src/mod/b.rs", "src/mod/c.rs"] {
            index.files.push(IndexedFile {
                relative_path: name.to_string(),
                language: Some("rust".into()),
                size_bytes: 100,
                token_count: 50,
                parse_result: None,
                content: "fn f() {}".to_string(),
                mtime_secs: None,
            });
        }

        let report = build_drift_report(&index, dir.path(), false);
        assert!(
            report.baseline.is_some(),
            "baseline comparison must be present when baseline file exists"
        );
        let bc = report.baseline.unwrap();
        assert_eq!(bc.baseline_date, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn test_load_snapshots_sorted_by_parsed_timestamp() {
        let dir = tempfile::TempDir::new().unwrap();

        // Write three snapshots with timestamps that sort differently
        // lexicographically vs chronologically (UTC vs +01:00 offset).
        let older = ArchitectureSnapshot {
            // UTC: 2026-01-01T11:00:00Z — earlier instant
            timestamp: "2026-01-01T11:00:00Z".to_string(),
            metrics: ArchitectureMetrics {
                module_count: 1,
                mean_coupling: 0.1,
                mean_cohesion: 0.9,
                cycle_count: 0,
                boundary_violation_count: 0,
            },
            modules: vec![],
        };
        let newer = ArchitectureSnapshot {
            // offset +01:00: 2026-01-01T12:30:00+01:00 == 2026-01-01T11:30:00Z — later instant
            timestamp: "2026-01-01T12:30:00+01:00".to_string(),
            metrics: ArchitectureMetrics {
                module_count: 2,
                mean_coupling: 0.2,
                mean_cohesion: 0.8,
                cycle_count: 0,
                boundary_violation_count: 0,
            },
            modules: vec![],
        };

        save_snapshot(dir.path(), &older).unwrap();
        save_snapshot(dir.path(), &newer).unwrap();

        let loaded = load_snapshots(dir.path());
        assert_eq!(loaded.len(), 2, "both snapshots must load");
        // Descending order: newest first — newer has later instant (11:30Z > 11:00Z)
        assert_eq!(
            loaded[0].metrics.module_count, 2,
            "newest snapshot (12:30+01:00 == 11:30Z) must be first"
        );
        assert_eq!(
            loaded[1].metrics.module_count, 1,
            "oldest snapshot (11:00Z) must be second"
        );
    }

    #[test]
    fn test_module_count_matches_mean_coupling_denominator() {
        use crate::index::{CodebaseIndex, IndexedFile};
        use crate::schema::EdgeType;

        let mut index = CodebaseIndex::empty();

        // Module A: 3 files → qualifies
        for name in &["src/a/x.rs", "src/a/y.rs", "src/a/z.rs"] {
            index.files.push(IndexedFile {
                relative_path: name.to_string(),
                language: Some("rust".into()),
                size_bytes: 100,
                token_count: 50,
                parse_result: None,
                content: "fn f() {}".to_string(),
                mtime_secs: None,
            });
        }
        // Module B: 2 files → does NOT qualify
        for name in &["src/b/p.rs", "src/b/q.rs"] {
            index.files.push(IndexedFile {
                relative_path: name.to_string(),
                language: Some("rust".into()),
                size_bytes: 50,
                token_count: 25,
                parse_result: None,
                content: "fn g() {}".to_string(),
                mtime_secs: None,
            });
        }

        index
            .graph
            .add_edge("src/a/x.rs", "src/a/y.rs", EdgeType::Import);
        index
            .graph
            .add_edge("src/a/x.rs", "src/ext/lib.rs", EdgeType::Import);

        let snap = snapshot_from_index(&index, "2026-04-10T00:00:00Z");

        // Only module A qualifies — module_count must equal 1 (the qualifying count).
        assert_eq!(
            snap.metrics.module_count, 1,
            "module_count must reflect qualifying modules, not total prefixes"
        );
        // mean_coupling was computed over 1 qualifying module, so module_count
        // is the correct denominator.
        assert!(
            snap.metrics.mean_coupling >= 0.0 && snap.metrics.mean_coupling <= 1.0,
            "mean_coupling must be in [0,1]"
        );
        // The qualifying module's coupling equals mean_coupling (only 1 module).
        let m = &snap.modules[0];
        assert!(
            (snap.metrics.mean_coupling - m.coupling).abs() < 1e-9,
            "mean_coupling must equal the single qualifying module's coupling"
        );
    }
}
