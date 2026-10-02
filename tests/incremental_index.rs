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

#[test]
fn test_incremental_rebuild_detects_same_size_same_second_edit() {
    // cxpak#36: a same-size edit landing in the same wall-clock second as the
    // prior index build must still trigger a reparse. The old `needs_update`
    // check compared `mtime` truncated to whole seconds with a strict `>`, so
    // a same-second, same-size edit was silently skipped and stale content
    // kept being served.
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let fp = dir.path().join("a.rs");
    std::fs::write(&fp, "fn a() { 111 }").unwrap();
    let file = ScannedFile {
        relative_path: "a.rs".into(),
        absolute_path: fp.clone(),
        language: Some("rust".into()),
        size_bytes: 14,
    };

    let mut index = CodebaseIndex::build(vec![file.clone()], HashMap::new(), &counter);
    assert_eq!(index.files[0].content, "fn a() { 111 }");

    // Same byte size as the original content, written immediately after
    // (microseconds later — deterministically within the same wall-clock
    // second on any filesystem with at least 1s mtime resolution).
    std::fs::write(&fp, "fn a() { 222 }").unwrap();

    index.incremental_rebuild(&[file], &HashMap::new(), &counter);

    assert_eq!(
        index.files[0].content, "fn a() { 222 }",
        "a same-size edit within the same wall-clock second as the prior index \
         must still trigger a reparse (cxpak#36)"
    );
}
