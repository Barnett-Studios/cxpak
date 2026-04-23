//! Adversarial tests for dead-code detection that specifically probe the
//! over-permissive heuristic introduced in v2.1.0 commit `fe012ba` and
//! partially rolled back in `6f25908`. Every test in this file constructs a
//! fixture that deliberately trips the OLD broken `has_qualified_reference`
//! fallback (`use ` + any `::name` substring) while the target symbol has
//! no actual qualified reference.
//!
//! If any of these tests pass without our fixed detector flagging the
//! deliberately-dead symbol, the heuristic is still dishonest.

use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use cxpak::intelligence::dead_code::detect_dead_code;
use cxpak::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
use cxpak::scanner::ScannedFile;
use std::collections::HashMap;

type FixtureEntry = (ScannedFile, (String, ParseResult), (String, String));

fn make_file(path: &str, content: &str, symbols: Vec<Symbol>) -> FixtureEntry {
    let scanned = ScannedFile {
        relative_path: path.into(),
        absolute_path: format!("/tmp/{path}").into(),
        language: Some("rust".into()),
        size_bytes: content.len() as u64,
    };
    let pr = ParseResult {
        symbols,
        imports: vec![],
        exports: vec![],
    };
    (scanned, (path.into(), pr), (path.into(), content.into()))
}

fn build_index(entries: Vec<FixtureEntry>) -> CodebaseIndex {
    let mut files = Vec::new();
    let mut prs = HashMap::new();
    let mut contents = HashMap::new();
    for (f, pr, c) in entries {
        files.push(f);
        prs.insert(pr.0, pr.1);
        contents.insert(c.0, c.1);
    }
    CodebaseIndex::build_with_content(files, prs, &TokenCounter::new(), contents)
}

fn fn_symbol(name: &str, start_line: usize, is_public: bool) -> Symbol {
    Symbol {
        name: name.into(),
        kind: SymbolKind::Function,
        visibility: if is_public {
            Visibility::Public
        } else {
            Visibility::Private
        },
        signature: format!("fn {name}()"),
        body: "{}".into(),
        start_line,
        end_line: start_line + 2,
    }
}

/// A function named `run` defined in a file where NO other file contains a
/// qualified reference to it (no `module::run`, no `pub use`, no call graph
/// caller) MUST be flagged dead.
///
/// The fixture deliberately creates a second file with `use clap::Parser;`
/// and an unrelated `::other` substring to trip the old broken heuristic
/// (`use ` + any `::name`). If the symbol survives despite this, the
/// heuristic is still cheating.
#[test]
fn run_with_no_qualified_reference_is_flagged_dead() {
    // file_a.rs defines `pub fn run()` with no internal other references.
    let file_a_content = "pub fn run() {}\n";
    let file_a_symbols = vec![fn_symbol("run", 1, true)];
    // file_b.rs has `use ` (trips old heuristic's `use_pattern`) AND `::run`
    // (trips old heuristic's `bare_qualified`). Critically, `some_other_mod::run`
    // is NOT a reference to `file_a::run` — it's a DIFFERENT run belonging to
    // `some_other_mod`. The old `use ` + `::run` heuristic would incorrectly
    // mark `file_a::run` alive on this fixture.
    let file_b_content = "use clap::Parser;\nfn caller() { some_other_mod::run(); }\n";
    let file_b_symbols = vec![fn_symbol("caller", 2, false)];

    let idx = build_index(vec![
        make_file("src/file_a.rs", file_a_content, file_a_symbols),
        make_file("src/file_b.rs", file_b_content, file_b_symbols),
    ]);

    let dead = detect_dead_code(&idx, None);
    let dead_names: Vec<&str> = dead.iter().map(|d| d.symbol.as_str()).collect();
    assert!(
        dead_names.contains(&"run"),
        "`run` must be flagged dead — no caller, no qualified ref, no pub use. \
         Got dead list: {dead_names:?}"
    );
}

/// A function WITH a qualified reference (`file_a::run` appears in another
/// file) MUST be alive.
#[test]
fn run_with_qualified_reference_is_alive() {
    let file_a_content = "pub fn run() {}\n";
    let file_a_symbols = vec![fn_symbol("run", 1, true)];
    // file_b explicitly calls `file_a::run`.
    let file_b_content = "use crate::file_a;\nfn caller() { file_a::run(); }\n";
    let file_b_symbols = vec![fn_symbol("caller", 2, false)];

    let idx = build_index(vec![
        make_file("src/file_a.rs", file_a_content, file_a_symbols),
        make_file("src/file_b.rs", file_b_content, file_b_symbols),
    ]);

    let dead = detect_dead_code(&idx, None);
    let dead_names: Vec<&str> = dead.iter().map(|d| d.symbol.as_str()).collect();
    assert!(
        !dead_names.contains(&"run"),
        "`run` must be alive — `file_a::run` appears in file_b. \
         Got dead list: {dead_names:?}"
    );
}

/// A function re-exported via `pub use foo::module::run;` in some mod.rs
/// MUST be alive (the re-export substring `module::run` matches the direct
/// module-path check).
#[test]
fn run_reexported_via_pub_use_is_alive() {
    let file_a_content = "pub fn run() {}\n";
    let file_a_symbols = vec![fn_symbol("run", 1, true)];
    // mod.rs re-exports `file_a::run` under a different name.
    let mod_rs_content = "pub use crate::file_a::run as entry;\n";
    let mod_rs_symbols: Vec<Symbol> = vec![];

    let idx = build_index(vec![
        make_file("src/file_a.rs", file_a_content, file_a_symbols),
        make_file("src/mod.rs", mod_rs_content, mod_rs_symbols),
    ]);

    let dead = detect_dead_code(&idx, None);
    let dead_names: Vec<&str> = dead.iter().map(|d| d.symbol.as_str()).collect();
    assert!(
        !dead_names.contains(&"run"),
        "`run` must be alive — re-exported via `pub use crate::file_a::run as entry`. \
         Got dead list: {dead_names:?}"
    );
}

/// A method called via receiver syntax `obj.method(...)` in another file
/// MUST be alive. This captures common Rust calling convention that the
/// call graph's name-based extractor cannot resolve (it sees `method` as
/// a bare name with no type receiver).
#[test]
fn method_called_via_receiver_syntax_is_alive() {
    // file_a defines an inherent-impl method `process` on `Handler`.
    let file_a_content = "pub struct Handler;\nimpl Handler {\n    pub fn process(&self) {}\n}\n";
    let process_sym = Symbol {
        name: "process".into(),
        kind: SymbolKind::Method,
        visibility: Visibility::Public,
        signature: "pub fn process(&self)".into(),
        body: "{}".into(),
        start_line: 3,
        end_line: 3,
    };
    // file_b uses receiver-style `handler.process()` with no type qualification.
    let file_b_content =
        "use crate::file_a::Handler;\nfn caller(handler: &Handler) { handler.process(); }\n";
    let caller_sym = fn_symbol("caller", 2, false);

    let idx = build_index(vec![
        make_file("src/file_a.rs", file_a_content, vec![process_sym]),
        make_file("src/file_b.rs", file_b_content, vec![caller_sym]),
    ]);

    let dead = detect_dead_code(&idx, None);
    let dead_names: Vec<&str> = dead.iter().map(|d| d.symbol.as_str()).collect();
    assert!(
        !dead_names.contains(&"process"),
        "`process` must be alive — it's called via `handler.process()` in file_b. \
         Got dead list: {dead_names:?}"
    );
}

/// A function `foo` in a file where another file contains `foo::bar` (where
/// `bar` is a DIFFERENT symbol) must NOT be marked alive by matching the
/// prefix. `foo::bar` has `foo` qualifying `bar`, not `bar` qualifying `foo`.
/// Without word-boundary checks, a naive substring match could confuse these.
#[test]
fn prefix_match_does_not_keep_symbol_alive() {
    // file_a.rs defines `pub fn run()` — target under test.
    let file_a_content = "pub fn run() {}\n";
    let file_a_symbols = vec![fn_symbol("run", 1, true)];
    // file_b contains `file_a::run_other` — looks like `file_a::run` as
    // substring but is actually a reference to `run_other`, not `run`.
    let file_b_content = "use crate::file_a;\nfn caller() { file_a::run_other(); }\n";
    let file_b_symbols = vec![fn_symbol("caller", 2, false)];

    let idx = build_index(vec![
        make_file("src/file_a.rs", file_a_content, file_a_symbols),
        make_file("src/file_b.rs", file_b_content, file_b_symbols),
    ]);

    let dead = detect_dead_code(&idx, None);
    let dead_names: Vec<&str> = dead.iter().map(|d| d.symbol.as_str()).collect();
    assert!(
        dead_names.contains(&"run"),
        "`run` must be flagged dead — `file_a::run_other` is NOT a reference to `run`. \
         Got dead list: {dead_names:?}"
    );
}

// ── Fix 12: receiver-heuristic short-name rubber-stamp ───────────────────────

/// A 3-char method `run` defined in src/foo.rs with NO qualified reference
/// anywhere and ONLY an unrelated `.run(` in another file must be flagged dead.
/// The receiver-call heuristic must NOT rubber-stamp it alive just because some
/// unrelated `.run(` exists in the codebase.
#[test]
fn short_method_with_no_evidence_is_flagged_dead() {
    use cxpak::parser::language::Visibility;
    let foo_content = "pub struct App;\nimpl App {\n    pub fn run(&self) {}\n}\n";
    let foo_sym = Symbol {
        name: "run".into(),
        kind: SymbolKind::Method,
        visibility: Visibility::Public,
        signature: "pub fn run(&self)".into(),
        body: "{}".into(),
        start_line: 3,
        end_line: 3,
    };
    // bar.rs has `.run(` but for a completely different object — not App.
    let bar_content = "use crate::other;\nfn caller() { other.run(); }\n";

    let mut files = Vec::new();
    let mut prs = HashMap::new();
    let mut contents = HashMap::new();

    let foo_file = ScannedFile {
        relative_path: "src/foo.rs".into(),
        absolute_path: "/tmp/src/foo.rs".into(),
        language: Some("rust".into()),
        size_bytes: foo_content.len() as u64,
    };
    let bar_file = ScannedFile {
        relative_path: "src/bar.rs".into(),
        absolute_path: "/tmp/src/bar.rs".into(),
        language: Some("rust".into()),
        size_bytes: bar_content.len() as u64,
    };
    prs.insert(
        "src/foo.rs".to_string(),
        ParseResult {
            symbols: vec![foo_sym],
            imports: vec![],
            exports: vec![],
        },
    );
    prs.insert(
        "src/bar.rs".to_string(),
        ParseResult {
            symbols: vec![fn_symbol("caller", 2, false)],
            imports: vec![],
            exports: vec![],
        },
    );
    contents.insert("src/foo.rs".to_string(), foo_content.to_string());
    contents.insert("src/bar.rs".to_string(), bar_content.to_string());
    files.push(foo_file);
    files.push(bar_file);

    let idx = CodebaseIndex::build_with_content(files, prs, &TokenCounter::new(), contents);
    let dead = detect_dead_code(&idx, None);
    let names: Vec<&str> = dead.iter().map(|d| d.symbol.as_str()).collect();
    assert!(
        names.contains(&"run"),
        "3-char method `run` with no qualified ref must NOT be alive-stamped by receiver \
         heuristic just because some unrelated `.run(` exists. Got dead: {names:?}"
    );
}
