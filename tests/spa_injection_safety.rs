#![cfg(feature = "visual")]

#[test]
fn spa_survives_malicious_filename() {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let evil = r"</script><img src=x onerror=alert(1)>.rs";
    let files = vec![cxpak::scanner::ScannedFile {
        relative_path: evil.into(),
        absolute_path: format!("/tmp/{evil}").into(),
        language: Some("rust".into()),
        size_bytes: 10,
    }];
    let mut c = std::collections::HashMap::new();
    c.insert(evil.into(), "//".into());
    let idx = cxpak::index::CodebaseIndex::build_with_content(
        files,
        std::collections::HashMap::new(),
        &counter,
        c,
    );
    let meta = cxpak::visual::render::RenderMetadata {
        repo_name: "injection-test".into(),
        generated_at: "2026-04-17T12:00:00Z".into(),
        health_score: None,
        node_count: 1,
        edge_count: 0,
        cxpak_version: "2.1.0".into(),
    };
    let html = cxpak::visual::spa::render_spa(&idx, &meta).unwrap();
    assert!(!html.contains("onerror=alert"), "raw payload leaked");
    assert!(
        !html.contains(r#"</script><img"#),
        "script-break sequence leaked"
    );
}
