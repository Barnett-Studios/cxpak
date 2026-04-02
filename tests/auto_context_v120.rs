use cxpak::auto_context::{auto_context, AutoContextOpts};
use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use cxpak::scanner::ScannedFile;
use std::collections::HashMap;

fn make_test_index() -> (CodebaseIndex, tempfile::TempDir) {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();

    let files_data = [
        ("src/api/handler.rs", "pub fn handle() {}"),
        ("src/api/router.rs", "pub fn route() {}"),
        ("src/api/auth.rs", "pub fn authenticate() -> bool { true }"),
        ("src/db/query.rs", "pub fn run() {}"),
        ("src/db/models.rs", "pub struct User {}"),
        ("src/db/connection.rs", "pub fn connect() {}"),
        ("tests/handler_test.rs", "fn test_handle() {}"),
    ];

    let files: Vec<ScannedFile> = files_data
        .iter()
        .map(|(rel, content)| {
            let safe = rel.replace('/', "_");
            let abs = dir.path().join(&safe);
            std::fs::write(&abs, content).unwrap();
            ScannedFile {
                relative_path: rel.to_string(),
                absolute_path: abs,
                language: Some("rust".into()),
                size_bytes: content.len() as u64,
            }
        })
        .collect();

    let index = CodebaseIndex::build(files, HashMap::new(), &counter);
    (index, dir)
}

#[test]
fn test_v120_auto_context_result_has_all_fields() {
    let (index, _dir) = make_test_index();
    let opts = AutoContextOpts {
        tokens: 50_000,
        focus: None,
        include_tests: false,
        include_blast_radius: false,
        mode: "full".to_string(),
    };
    let result = auto_context("handle authentication request", &index, &opts);

    // Health score: valid range
    assert!(
        result.health.composite >= 0.0 && result.health.composite <= 10.0,
        "composite out of range: {}",
        result.health.composite
    );
    assert!(
        result.health.dead_code.is_none(),
        "dead_code must be None in v1.2.0"
    );

    // Risks: capped at 10
    assert!(result.risks.len() <= 10);
    for risk in &result.risks {
        assert!(
            risk.risk_score > 0.0,
            "risk score must be positive for {}",
            risk.path
        );
        assert!(
            risk.risk_score <= 1.0,
            "risk score must be <= 1.0 for {}",
            risk.path
        );
    }

    // Architecture: modules present
    assert!(
        !result.architecture.modules.is_empty(),
        "architecture map must have modules"
    );

    // Full mode: all target file content is Some
    for file in &result.sections.target_files.files {
        assert!(
            file.content.is_some(),
            "full mode: content must be Some for {}",
            file.path
        );
    }

    // Budget invariant
    assert_eq!(
        result.budget.used + result.budget.remaining,
        result.budget.total,
        "budget invariant violated"
    );
}

#[test]
fn test_v120_briefing_mode_content_is_none() {
    let (index, _dir) = make_test_index();
    let opts = AutoContextOpts {
        tokens: 50_000,
        focus: None,
        include_tests: false,
        include_blast_radius: false,
        mode: "briefing".to_string(),
    };
    let result = auto_context("handle authentication", &index, &opts);

    // Briefing mode: all target file content is None
    for file in &result.sections.target_files.files {
        assert!(
            file.content.is_none(),
            "briefing mode: content must be None for {}",
            file.path
        );
    }

    // Health + risks + architecture still present in briefing mode
    assert!(result.health.composite >= 0.0);
    assert!(result.risks.len() <= 10);
}

#[test]
fn test_v120_risks_sorted_descending() {
    let (index, _dir) = make_test_index();
    let opts = AutoContextOpts {
        tokens: 50_000,
        focus: None,
        include_tests: false,
        include_blast_radius: false,
        mode: "full".to_string(),
    };
    let result = auto_context("handle", &index, &opts);
    for i in 1..result.risks.len() {
        assert!(
            result.risks[i - 1].risk_score >= result.risks[i].risk_score,
            "risks not sorted at index {i}"
        );
    }
}
