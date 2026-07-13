//! N4 acceptance: the Timeline is wired — snapshots are computed, backfilled
//! with each commit's OWN per-commit health/cycles (not the current tree), and
//! injected into the SPA render (never read from a live cache in the render
//! path, so determinism holds).
#![cfg(feature = "visual")]

use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use cxpak::visual::render::RenderMetadata;
use cxpak::visual::spa::render_spa_with_timeline;
use cxpak::visual::timeline::{
    compute_timeline_snapshots, enrich_snapshots_with_health, TimelineSnapshot,
};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git available")
        .success();
    assert!(ok, "git {args:?} failed");
}

/// Two commits with *divergent* structure so the test can prove each snapshot
/// reflects its OWN tree, not the current (checked-out) one:
/// - c1: `src/a.rs` ↔ `src/b.rs` form an import cycle (`use crate::b;` /
///   `use crate::a;`), which the file-level SCC detector reports as one cycle.
/// - c2: `src/b.rs` drops its `use crate::a;`, breaking the cycle.
///
/// The working tree ends on c2 (acyclic); a regression that stamped the
/// current tree on every snapshot would show 0 cycles for BOTH commits.
fn tiny_repo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "--quiet"]);
    git(p, &["config", "user.email", "t@example.com"]);
    git(p, &["config", "user.name", "t"]);
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(
        p.join("src/a.rs"),
        "use crate::b::thing_b;\npub fn thing_a() { thing_b(); }\n",
    )
    .unwrap();
    std::fs::write(
        p.join("src/b.rs"),
        "use crate::a::thing_a;\npub fn thing_b() { thing_a(); }\n",
    )
    .unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "c1"]);
    // c2: break the cycle — b no longer imports a.
    std::fs::write(p.join("src/b.rs"), "pub fn thing_b() {}\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "c2"]);
    dir
}

#[test]
fn enrich_populates_per_commit_health_and_cycles() {
    let dir = tiny_repo();
    let mut snaps = compute_timeline_snapshots(dir.path(), 10).unwrap();
    assert!(!snaps.is_empty(), "snapshots computed from the repo");
    // Base compute leaves health unset.
    assert!(snaps.iter().all(|s| s.health_composite.is_none()));

    enrich_snapshots_with_health(dir.path(), &mut snaps);
    assert!(
        snaps.iter().all(|s| s.health_composite.is_some()),
        "every snapshot carries its own per-commit health after backfill"
    );
    // Per-commit divergence is the real guard: c1's tree has an a↔b cycle,
    // c2's does not. This fails a regression that reconstructs the wrong tree
    // (e.g. stamps the current checked-out tree on every snapshot → 0 cycles
    // everywhere), which the previous trivial-acyclic fixture could not detect.
    let c1 = snaps
        .iter()
        .find(|s| s.commit_message.trim() == "c1")
        .expect("c1 snapshot present");
    let c2 = snaps
        .iter()
        .find(|s| s.commit_message.trim() == "c2")
        .expect("c2 snapshot present");
    assert!(
        c1.circular_dep_count > 0,
        "c1's tree has an a<->b import cycle; got {}",
        c1.circular_dep_count
    );
    assert_eq!(
        c2.circular_dep_count, 0,
        "c2's tree broke the cycle, so it must report zero"
    );
}

fn tiny_index() -> CodebaseIndex {
    let counter = TokenCounter::new();
    let mut content = HashMap::new();
    content.insert("src/main.rs".to_string(), "fn main() {}".to_string());
    let files = vec![cxpak::scanner::ScannedFile {
        relative_path: "src/main.rs".into(),
        absolute_path: "/tmp/cxpak-tl/src/main.rs".into(),
        language: Some("rust".into()),
        size_bytes: 12,
    }];
    CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content)
}

fn meta() -> RenderMetadata {
    RenderMetadata {
        repo_name: "tl".into(),
        generated_at: "2026-07-11T00:00:00Z".into(),
        health_score: Some(7.0),
        node_count: 1,
        edge_count: 0,
        cxpak_version: "3.1.0".into(),
    }
}

#[test]
fn render_embeds_injected_timeline_and_defaults_to_null() {
    let idx = tiny_index();
    let snap = TimelineSnapshot {
        commit_sha: "abc123".into(),
        commit_date: "2026-01-01T00:00:00Z".into(),
        commit_message: "c".into(),
        files: vec![],
        edge_count: 1,
        module_count: 1,
        health_composite: Some(7.0),
        circular_dep_count: 0,
    };

    let with = render_spa_with_timeline(&idx, &meta(), Some(std::slice::from_ref(&snap))).unwrap();
    assert!(
        with.contains("abc123"),
        "injected timeline snapshot is embedded"
    );
    // The renderer (timeline_js) consumes the {steps, current_index,
    // health_sparkline} view-model, NOT a raw snapshot array. Assert the injected
    // JSON carries that shape, so a raw-array regression (which shows the empty
    // state despite embedded snapshots) fails here instead of only in the browser.
    let tag = with
        .split("id=\"cxpak-timeline\" type=\"application/json\">")
        .nth(1)
        .and_then(|s| s.split("</script>").next())
        .expect("timeline data tag present");
    assert!(
        tag.contains("\"steps\"") && tag.contains("\"health_sparkline\""),
        "timeline injected as the TimeMachineData view-model, not a raw snapshot array; got: {}",
        &tag[..tag.len().min(120)]
    );

    let without = cxpak::visual::spa::render_spa(&idx, &meta()).unwrap();
    assert!(
        !without.contains("abc123"),
        "the default render embeds no timeline (deterministic)"
    );
}
