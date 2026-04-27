//! Adversarial tests for Trojan-source / bidi-control-char defenses.
//!
//! A malicious repository can embed Unicode bidi override characters
//! (U+202A–202E, U+2066–2069) in file paths, symbol names, or commit
//! messages.  When a downstream renderer prints them verbatim, the
//! visual order diverges from the semantic order — an identifier
//! `accessLevel = "user\u{202E}//\u{202D}admin"` renders as
//! `accessLevel = "user//admin"` while semantically being an admin
//! grant followed by a comment-out.  cxpak's SPA dashboard, search
//! index, LSP diagnostics, and recent-changes output all consume
//! user-supplied path/symbol strings, so each is a vector unless
//! sanitised.
//!
//! Each test embeds an attack codepoint and asserts the rendered
//! output replaces it with a visible `<U+XXXX>` escape.

use cxpak::util::sanitize_bidi;

#[test]
fn sanitize_bidi_passes_through_innocent_strings() {
    assert_eq!(sanitize_bidi("src/main.rs"), "src/main.rs");
    assert_eq!(sanitize_bidi("compute_health"), "compute_health");
    assert_eq!(
        sanitize_bidi("résumé.rs"),
        "résumé.rs",
        "non-ASCII letters are NOT format chars and pass through unchanged"
    );
}

#[test]
fn sanitize_bidi_escapes_rlo_override() {
    // U+202E RLO is the canonical Trojan-source vector.
    let attack = "user\u{202E}//\u{202D}admin";
    let safe = sanitize_bidi(attack);
    assert!(
        safe.contains("<U+202E>"),
        "RLO must be escaped to a visible token; got: {safe}"
    );
    assert!(
        safe.contains("<U+202D>"),
        "LRO must also be escaped; got: {safe}"
    );
    assert!(
        !safe.chars().any(|c| matches!(c, '\u{202E}' | '\u{202D}')),
        "no raw bidi chars must remain after sanitisation; got: {safe:?}"
    );
}

#[test]
fn sanitize_bidi_escapes_all_directional_format_chars() {
    let chars = [
        ('\u{202A}', "LRE"),
        ('\u{202B}', "RLE"),
        ('\u{202C}', "PDF"),
        ('\u{202D}', "LRO"),
        ('\u{202E}', "RLO"),
        ('\u{2066}', "LRI"),
        ('\u{2067}', "RLI"),
        ('\u{2068}', "FSI"),
        ('\u{2069}', "PDI"),
    ];
    for (c, name) in chars {
        let input = format!("a{c}b");
        let out = sanitize_bidi(&input);
        let expected = format!("a<U+{:04X}>b", c as u32);
        assert_eq!(out, expected, "{name} (U+{:04X}) must be escaped", c as u32);
    }
}

#[test]
fn sanitize_bidi_escapes_zero_width_chars_too() {
    // Zero-width joiner / non-joiner / space are homograph-attack vectors
    // (e.g., `cls\u{200B}assroom` looks like `classroom`).
    let chars = [
        ('\u{200B}', "ZWSP"),
        ('\u{200C}', "ZWNJ"),
        ('\u{200D}', "ZWJ"),
        ('\u{200E}', "LRM"),
        ('\u{200F}', "RLM"),
        ('\u{061C}', "ALM"),
    ];
    for (c, name) in chars {
        let input = format!("foo{c}bar");
        let out = sanitize_bidi(&input);
        let expected = format!("foo<U+{:04X}>bar", c as u32);
        assert_eq!(out, expected, "{name} must be escaped");
    }
}

// ── Boundary tests: search index, LSP diagnostics, recent_changes ──────────

#[test]
#[cfg(feature = "visual")]
fn search_index_sanitises_file_path_with_rlo() {
    use cxpak::budget::counter::TokenCounter;
    use cxpak::index::CodebaseIndex;
    use cxpak::scanner::ScannedFile;
    use std::collections::HashMap;
    let counter = TokenCounter::new();
    // File path embeds RLO between two segments — without sanitisation
    // the SPA palette would render the path with reversed display.
    let evil_path = "src/admin\u{202E}//.rs";
    let files = vec![ScannedFile {
        relative_path: evil_path.to_string(),
        absolute_path: format!("/tmp/{evil_path}").into(),
        language: Some("rust".into()),
        size_bytes: 0,
    }];
    let idx = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, HashMap::new());
    let entries = cxpak::visual::search_index::build_search_index(&idx);
    let file_entry = entries
        .iter()
        .find(|e| e.kind == "file")
        .expect("file entry must be present");
    assert!(
        file_entry.label.contains("<U+202E>"),
        "search-index label must be bidi-sanitised; got: {:?}",
        file_entry.label
    );
    assert!(
        !file_entry.label.contains('\u{202E}'),
        "no raw RLO must remain in the rendered label"
    );
    // Target URL is also rendered to the user — must be sanitised.
    assert!(
        file_entry.target.contains("<U+202E>"),
        "search-index target must be bidi-sanitised; got: {:?}",
        file_entry.target
    );
}

#[test]
#[cfg(feature = "visual")]
fn search_index_sanitises_symbol_name_with_zwj() {
    use cxpak::budget::counter::TokenCounter;
    use cxpak::index::CodebaseIndex;
    use cxpak::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use cxpak::scanner::ScannedFile;
    use std::collections::HashMap;
    let counter = TokenCounter::new();
    // Symbol name embeds zero-width joiner — visually identical to a
    // legitimate name, semantically distinct.
    let evil_sym = "admin\u{200D}_check";
    let files = vec![ScannedFile {
        relative_path: "src/auth.rs".into(),
        absolute_path: "/tmp/src/auth.rs".into(),
        language: Some("rust".into()),
        size_bytes: 0,
    }];
    let mut parses = HashMap::new();
    parses.insert(
        "src/auth.rs".to_string(),
        ParseResult {
            symbols: vec![Symbol {
                name: evil_sym.to_string(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: format!("fn {evil_sym}()"),
                body: "{}".into(),
                start_line: 1,
                end_line: 1,
            }],
            imports: vec![],
            exports: vec![],
        },
    );
    let idx = CodebaseIndex::build_with_content(files, parses, &counter, HashMap::new());
    let entries = cxpak::visual::search_index::build_search_index(&idx);
    let sym_entry = entries
        .iter()
        .find(|e| e.kind == "symbol")
        .expect("symbol entry");
    assert!(
        sym_entry.label.contains("<U+200D>"),
        "ZWJ in symbol name must be escaped; got: {:?}",
        sym_entry.label
    );
}

#[test]
#[cfg(feature = "lsp")]
fn lsp_diagnostic_message_sanitises_bidi_in_symbol_name() {
    use cxpak::budget::counter::TokenCounter;
    use cxpak::index::CodebaseIndex;
    use cxpak::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use cxpak::scanner::ScannedFile;
    use std::collections::HashMap;
    let counter = TokenCounter::new();
    let evil_sym = "dead\u{202E}_fn";
    let file = ScannedFile {
        relative_path: "src/x.rs".into(),
        absolute_path: "/tmp/src/x.rs".into(),
        language: Some("rust".into()),
        size_bytes: 0,
    };
    let mut parses = HashMap::new();
    parses.insert(
        "src/x.rs".into(),
        ParseResult {
            symbols: vec![Symbol {
                name: evil_sym.to_string(),
                kind: SymbolKind::Function,
                visibility: Visibility::Private,
                signature: format!("fn {evil_sym}()"),
                body: "{}".into(),
                start_line: 1,
                end_line: 1,
            }],
            imports: vec![],
            exports: vec![],
        },
    );
    let mut content = HashMap::new();
    content.insert("src/x.rs".into(), format!("fn {evil_sym}() {{}}\n"));
    let idx = CodebaseIndex::build_with_content(vec![file], parses, &counter, content);
    let diags =
        cxpak::lsp::methods::diagnostics_for_file("src/x.rs", &idx, std::path::Path::new("/tmp"));
    let msg = diags
        .iter()
        .find(|d| d.message.contains("dead code"))
        .map(|d| d.message.clone())
        .expect("dead-code diagnostic for the planted symbol");
    assert!(
        msg.contains("<U+202E>"),
        "LSP diagnostic must escape RLO so editors don't flip rendering; got: {msg}"
    );
    assert!(
        !msg.contains('\u{202E}'),
        "no raw RLO must remain in the diagnostic message"
    );
}

#[test]
fn recent_change_sanitises_bidi_in_path() {
    use cxpak::budget::counter::TokenCounter;
    use cxpak::conventions::git_health::ChurnEntry;
    use cxpak::index::CodebaseIndex;
    use std::collections::HashMap;
    let counter = TokenCounter::new();
    let mut idx = CodebaseIndex::build(vec![], HashMap::new(), &counter);
    let evil_path = "src/innocent\u{202E}_file.rs";
    idx.conventions.git_health.churn_30d.push(ChurnEntry {
        path: evil_path.to_string(),
        modifications: 1,
        last_commit_epoch: None,
    });
    let changes = cxpak::intelligence::recent_changes::compute_recent_changes(&idx);
    let entry = changes.first().expect("at least one entry");
    assert!(
        entry.path.contains("<U+202E>"),
        "recent_changes path must be bidi-sanitised; got: {:?}",
        entry.path
    );
}
