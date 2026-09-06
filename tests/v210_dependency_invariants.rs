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

/// The `visual` spill invariant, repointed from ADR-0135 to ADR-0208 (#62).
///
/// This test did its job: it is the tripwire that noticed `MCP_INLINE_LIMIT`
/// leaving the tree, which is exactly what a pinned invariant is for. What it
/// pinned is what changed — the ceiling is now a TOKEN budget rather than a
/// 1 MiB byte threshold, because 1 MiB is a transport figure and a 340 KB
/// dashboard from a three-file repo sailed under it into a caller's context.
///
/// Deliberately NOT deleted along with the constant. The mechanism it guards —
/// spill to `.cxpak/visual/`, do not stream an artifact into a context window —
/// survived; only the number and the units moved. Removing the guard because
/// the constant was renamed would retire a real invariant on a technicality.
///
/// It stays a grep over the source for the same reason it always was: it
/// catches ACCIDENTAL REMOVAL, which a behavioural test cannot, because a
/// behavioural test for "the limit still exists" passes on any limit at all.
#[test]
fn invariant_mcp_visual_token_ceiling_present() {
    let src = std::fs::read_to_string("src/commands/serve.rs").unwrap();
    assert!(
        src.contains("MAX_MCP_VISUAL_TOKENS"),
        "the visual token ceiling must remain defined in serve.rs (ADR-0208)"
    );
    assert!(
        !src.contains("MCP_INLINE_LIMIT"),
        "the superseded byte threshold must not come back alongside the token one — \
         two ceilings on one payload is how they drift"
    );
    assert!(
        src.contains(".cxpak/visual"),
        "write-to-file target directory must remain"
    );
    assert!(
        src.contains("visual_format_extension"),
        "a spilled artifact must carry the extension of what was written"
    );
}
