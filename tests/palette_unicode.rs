#![cfg(feature = "visual")]

#[test]
fn palette_rejects_nul_byte_in_file_path() {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let files = vec![cxpak::scanner::ScannedFile {
        relative_path: "src/nul\0file.rs".into(),
        absolute_path: "/tmp/bad".into(),
        language: Some("rust".into()),
        size_bytes: 10,
    }];
    let mut c = std::collections::HashMap::new();
    c.insert("src/nul\0file.rs".into(), "".into());
    let idx = cxpak::index::CodebaseIndex::build_with_content(
        files,
        std::collections::HashMap::new(),
        &counter,
        c,
    );
    let entries = cxpak::visual::search_index::build_search_index(&idx);
    assert!(
        !entries.iter().any(|e| e.label.contains('\0')),
        "NUL-byte paths must be rejected"
    );
}
