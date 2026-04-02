use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use cxpak::scanner::ScannedFile;
use std::collections::HashMap;

#[test]
fn test_incremental_rebuild_same_as_full_rebuild() {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();

    let write_file = |name: &str, content: &str| -> ScannedFile {
        let safe = name.replace('/', "_");
        let fp = dir.path().join(&safe);
        std::fs::write(&fp, content).unwrap();
        ScannedFile {
            relative_path: name.to_string(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: content.len() as u64,
        }
    };

    let a = write_file("src/a.rs", "pub fn alpha() {}");
    let b = write_file("src/b.rs", "pub fn beta() {}");

    // Full build with both files
    let full_index = CodebaseIndex::build(vec![a.clone(), b.clone()], HashMap::new(), &counter);

    // Build with only a.rs, then incrementally add b.rs
    let mut incremental = CodebaseIndex::build(vec![a.clone()], HashMap::new(), &counter);
    incremental.incremental_rebuild(&[a, b], &HashMap::new(), &counter);

    assert_eq!(
        incremental.total_files, full_index.total_files,
        "incremental rebuild must produce same file count as full rebuild"
    );
    assert_eq!(
        incremental.total_tokens, full_index.total_tokens,
        "incremental rebuild must produce same total tokens"
    );
}

#[test]
fn test_incremental_rebuild_noop_when_nothing_changed() {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let fp = dir.path().join("a.rs");
    std::fs::write(&fp, "fn a() {}").unwrap();
    let file = ScannedFile {
        relative_path: "a.rs".into(),
        absolute_path: fp,
        language: Some("rust".into()),
        size_bytes: 9,
    };

    let mut index = CodebaseIndex::build(vec![file.clone()], HashMap::new(), &counter);
    let tokens_before = index.total_tokens;
    let files_before = index.total_files;

    index.incremental_rebuild(&[file], &HashMap::new(), &counter);

    assert_eq!(index.total_files, files_before);
    assert_eq!(
        index.total_tokens, tokens_before,
        "noop incremental rebuild must not change token count"
    );
}
