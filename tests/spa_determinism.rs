#![cfg(feature = "visual")]

use std::path::PathBuf;

fn load_fixture_index() -> cxpak::index::CodebaseIndex {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let fixture_root = PathBuf::from(&manifest).join("tests/fixtures/determinism_repo");
    // Scanner requires a .git directory; create it if the embedded git was stripped.
    std::fs::create_dir_all(fixture_root.join(".git")).expect("create fixture .git");
    cxpak::commands::serve::build_index(&fixture_root).expect("fixture index builds")
}

fn fixture_metadata() -> cxpak::visual::render::RenderMetadata {
    cxpak::visual::render::RenderMetadata {
        repo_name: "determinism_repo".to_string(),
        generated_at: "[REDACTED]".to_string(),
        health_score: None,
        node_count: 0,
        edge_count: 0,
        cxpak_version: "[REDACTED]".to_string(),
    }
}

fn redact_html(html: &str) -> String {
    let re =
        regex::Regex::new(r#""(generated_at|timestamp|baseline_date)"\s*:\s*"[^"]+""#).unwrap();
    re.replace_all(html, r#""$1":"[REDACTED]""#).to_string()
}

#[test]
fn spa_output_matches_golden_fixture() {
    let idx = load_fixture_index();
    let meta = fixture_metadata();
    let actual = cxpak::visual::spa::render_spa(&idx, &meta).unwrap();
    let actual_redacted = redact_html(&actual);

    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let golden_path = PathBuf::from(&manifest).join("tests/snapshots/spa_golden.html");
    if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
        std::fs::write(&golden_path, &actual_redacted).unwrap();
        return;
    }
    let golden = match std::fs::read_to_string(&golden_path) {
        Ok(g) => g,
        Err(_) => panic!(
            "run UPDATE_SNAPSHOTS=1 cargo test to bootstrap {}",
            golden_path.display()
        ),
    };
    assert_eq!(
        actual_redacted, golden,
        "SPA output drift detected; run UPDATE_SNAPSHOTS=1 to accept"
    );
}
