#![cfg(feature = "visual")]

use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use cxpak::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
use cxpak::scanner::ScannedFile;
use std::collections::HashMap;

#[test]
fn invariant_onboarding_symbols_top_5() {
    let counter = TokenCounter::new();
    let files = vec![ScannedFile {
        relative_path: "src/main.rs".into(),
        absolute_path: "/tmp/src/main.rs".into(),
        language: Some("rust".into()),
        size_bytes: 500,
    }];
    let symbols: Vec<Symbol> = (0..7)
        .map(|i| Symbol {
            name: format!("pub_fn_{i}"),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: format!("fn pub_fn_{i}()"),
            body: "{}".into(),
            start_line: i * 3 + 1,
            end_line: i * 3 + 3,
        })
        .collect();
    let mut pr = HashMap::new();
    pr.insert(
        "src/main.rs".into(),
        ParseResult {
            symbols,
            imports: vec![],
            exports: vec![],
        },
    );
    let mut c = HashMap::new();
    c.insert("src/main.rs".into(), "fn x(){}".into());
    let idx = CodebaseIndex::build_with_content(files, pr, &counter, c);
    let map = cxpak::visual::onboard::compute_onboarding_map(&idx, None);
    for p in &map.phases {
        for f in &p.files {
            if f.path == "src/main.rs" {
                assert_eq!(
                    f.symbols_to_focus_on.len(),
                    5,
                    "expected top-5, got {}",
                    f.symbols_to_focus_on.len()
                );
            }
        }
    }
}

#[test]
fn invariant_onboarding_excludes_test_files() {
    let counter = TokenCounter::new();
    let files = vec![
        ScannedFile {
            relative_path: "src/main.rs".into(),
            absolute_path: "/tmp/src/main.rs".into(),
            language: Some("rust".into()),
            size_bytes: 10,
        },
        ScannedFile {
            relative_path: "tests/it_test.rs".into(),
            absolute_path: "/tmp/tests/it_test.rs".into(),
            language: Some("rust".into()),
            size_bytes: 10,
        },
    ];
    let mut c = HashMap::new();
    c.insert("src/main.rs".into(), "fn main(){}".into());
    c.insert("tests/it_test.rs".into(), "#[test] fn t(){}".into());
    let idx = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, c);
    let map = cxpak::visual::onboard::compute_onboarding_map(&idx, None);
    let paths: Vec<&str> = map
        .phases
        .iter()
        .flat_map(|p| p.files.iter().map(|f| f.path.as_str()))
        .collect();
    assert!(
        !paths.iter().any(|p| p.starts_with("tests/")),
        "test files leaked: {paths:?}"
    );
}

#[test]
fn invariant_mcp_inline_limit_constant_present() {
    // Grep-style check against the source file — this test catches accidental removal.
    let src = std::fs::read_to_string("src/commands/serve.rs").unwrap();
    assert!(
        src.contains("MCP_INLINE_LIMIT"),
        "MCP_INLINE_LIMIT must remain defined in serve.rs"
    );
    assert!(src.contains("1_048_576"), "1 MiB threshold must remain");
    assert!(
        src.contains(".cxpak/visual"),
        "write-to-file target directory must remain"
    );
}
