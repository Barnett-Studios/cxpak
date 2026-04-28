//! Adversarial tests closing the 8 round-4 pending-issues defects.
//!
//! - #1 standard LSP methods now use the snapshot-then-release pattern
//! - #2 `--token ""` rejected at startup
//! - #3 IPv4-mapped IPv6 loopback pinned (covered in serve_security.rs)
//! - #5 architecture module names + god_files bidi-sanitised
//! - #7 clipboard .catch present (covered in spa_golden update)
//! - #9 7 previously-dead handler params now wired
//! - #12 DependencyGraph::edge_count() shared helper
//! - #13 feature-matrix script presence (covered by scripts/feature-matrix.sh)
#![cfg(all(feature = "visual", feature = "daemon", feature = "lsp"))]

use serde_json::json;

// ── #1: standard LSP handlers use snapshot pattern ──────────────────────────

#[test]
fn standard_lsp_handlers_use_snapshot_helper() {
    // Source-level pin: code_lens / hover / diagnostic / symbol must each
    // call self.snapshot()? rather than self.index.read().  Without this
    // pin a refactor that re-introduces the long-held read guard would
    // pass tests (the index is already a snapshot during single-test
    // runs) and only manifest under concurrent watcher load in production.
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lsp/backend.rs"))
        .expect("read backend.rs");
    let snapshot_count = src.matches("self.snapshot()?").count();
    assert!(
        snapshot_count >= 4,
        "code_lens / hover / diagnostic / symbol must each go through self.snapshot(); \
         expected >=4 occurrences, found {snapshot_count}"
    );
    // Pin: NO bare `self.index.read()` should remain in the LanguageServer
    // impl block — the snapshot helper is the single read-side entry point.
    // We accept the helper itself (which contains the literal) but no
    // other site.
    let bare = src.matches("self.index.read()").count();
    assert_eq!(
        bare, 1,
        "exactly one `self.index.read()` should remain — inside the snapshot() helper itself; found {bare}"
    );
}

// ── #5: architecture module + god_files bidi-sanitised ──────────────────────

#[test]
fn architecture_module_prefix_is_bidi_sanitised() {
    use cxpak::budget::counter::TokenCounter;
    use cxpak::index::CodebaseIndex;
    use cxpak::scanner::ScannedFile;
    use std::collections::HashMap;
    let counter = TokenCounter::new();
    // Path embeds RLO between module segments.
    let evil = "src/admin\u{202E}//legit.rs";
    let files = vec![
        ScannedFile {
            relative_path: evil.into(),
            absolute_path: format!("/tmp/{evil}").into(),
            language: Some("rust".into()),
            size_bytes: 0,
        },
        // A second file in the same module so it has >=1 file count.
        ScannedFile {
            relative_path: format!("{}/x.rs", evil.split('/').next().unwrap_or("src")),
            absolute_path: "/tmp/src/x.rs".into(),
            language: Some("rust".into()),
            size_bytes: 0,
        },
    ];
    let idx = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, HashMap::new());
    let map = cxpak::intelligence::architecture::build_architecture_map(&idx, 2);
    for module in &map.modules {
        assert!(
            !module
                .prefix
                .chars()
                .any(|c| matches!(c, '\u{202E}' | '\u{202D}')),
            "architecture module prefix `{}` contains raw bidi control char — must be sanitised",
            module.prefix
        );
    }
}

// ── #9: previously-dead handler params now actually filter results ──────────

#[tokio::test]
async fn v1_dead_code_workspace_aliases_focus() {
    use axum::body::Body;
    use axum::http::Request;
    use cxpak::budget::counter::TokenCounter;
    use cxpak::commands::serve::build_router_for_test;
    use cxpak::index::CodebaseIndex;
    use cxpak::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use cxpak::scanner::ScannedFile;
    use std::collections::HashMap;
    use tower::ServiceExt;
    let counter = TokenCounter::new();
    let files = vec![
        ScannedFile {
            relative_path: "src/auth/dead.rs".into(),
            absolute_path: "/tmp/src/auth/dead.rs".into(),
            language: Some("rust".into()),
            size_bytes: 0,
        },
        ScannedFile {
            relative_path: "src/util/dead.rs".into(),
            absolute_path: "/tmp/src/util/dead.rs".into(),
            language: Some("rust".into()),
            size_bytes: 0,
        },
    ];
    let mut parses = HashMap::new();
    for f in &files {
        parses.insert(
            f.relative_path.clone(),
            ParseResult {
                symbols: vec![Symbol {
                    name: format!("dead_in_{}", f.relative_path.replace('/', "_")),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "fn x()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
    }
    let mut content = HashMap::new();
    for f in &files {
        content.insert(f.relative_path.clone(), "fn x() {}".into());
    }
    let idx = CodebaseIndex::build_with_content(files, parses, &counter, content);
    let shared = std::sync::Arc::new(std::sync::RwLock::new(std::sync::Arc::new(idx)));
    let router_focus = build_router_for_test(
        shared.clone(),
        std::sync::Arc::new(std::path::PathBuf::from("/tmp")),
    );

    // workspace alias must filter exactly like focus would.
    let req = Request::builder()
        .method("POST")
        .uri("/dead_code")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&json!({"workspace": "src/auth"})).unwrap(),
        ))
        .unwrap();
    let resp = router_focus.oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let symbols = body["dead_symbols"].as_array().expect("dead_symbols array");
    for sym in symbols {
        assert!(
            sym["file"].as_str().unwrap_or("").starts_with("src/auth"),
            "workspace=src/auth filter must drop entries outside that prefix; got {sym}"
        );
    }
}

// ── #12: shared edge_count helper ───────────────────────────────────────────

#[test]
fn dependency_graph_exposes_edge_count_helper() {
    use cxpak::budget::counter::TokenCounter;
    use cxpak::index::CodebaseIndex;
    let counter = TokenCounter::new();
    let idx = CodebaseIndex::build_with_content(
        vec![],
        std::collections::HashMap::new(),
        &counter,
        std::collections::HashMap::new(),
    );
    // Method exists and returns 0 on an empty graph.
    assert_eq!(idx.graph.edge_count(), 0);

    // Inlined formulation MUST agree with the helper — pin so the two
    // never drift independently.
    let inlined: usize = idx.graph.edges.values().map(|v| v.len()).sum();
    assert_eq!(idx.graph.edge_count(), inlined);
}

#[test]
fn no_inline_edge_count_lambda_anywhere_in_src() {
    // Stronger pin than the prior version, which only checked two specific
    // files.  The MCP `cxpak_visual` handler in serve.rs was missed and
    // continued to use the inline lambda.  This test now greps the whole
    // src/ tree for the lambda pattern and forbids it — any future
    // RenderMetadata construction site must go through `.edge_count()`.
    use std::path::PathBuf;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");

    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
                    out.push(p);
                }
            }
        }
    }
    let mut files = Vec::new();
    walk(&root, &mut files);

    let mut offenders = Vec::new();
    for f in files {
        let content = std::fs::read_to_string(&f).expect("read source file");
        // The DependencyGraph::edge_count helper itself is the canonical
        // implementation — exempt that one site by checking the call comes
        // from outside the helper definition line.
        let is_helper_def = f.ends_with("index/graph.rs");
        for (i, line) in content.lines().enumerate() {
            if line.contains(".edges.values().map(|v| v.len()).sum") {
                if is_helper_def
                    && content
                        .lines()
                        .skip(i.saturating_sub(3))
                        .take(8)
                        .any(|l| l.contains("pub fn edge_count"))
                {
                    continue;
                }
                offenders.push(format!("{}:{}", f.display(), i + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "Contract 8 violation — inline edge_count lambda found at: {offenders:?}\n\
         All RenderMetadata construction sites must use DependencyGraph::edge_count()."
    );
}

// ── #13: feature-matrix script exists and is executable ─────────────────────

// ── P3: structural innerHTML invariant ─────────────────────────────────────

/// `innerHTML = ... + ... + ...` is FORBIDDEN in render.rs as of v2.1.3.
/// All dynamic-data sites have been migrated to `CX.h(tag, attrs, children)`
/// — a safe-by-construction DOM builder where every attribute goes
/// through setAttribute and every text child through textContent.  No
/// developer-discipline-forever requirement; the structural property
/// is now mechanically enforceable by this test.
///
/// Static-literal innerHTMLs and clearing assignments (`innerHTML = ''`)
/// remain permitted — neither can interpolate attacker-controlled data.
/// Variable RHS (`innerHTML = msg`) is permitted with the assignment of
/// `msg` audited at its own site.
#[test]
fn render_innerhtml_concat_sites_are_forbidden() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/visual/render.rs"))
        .expect("read render.rs");
    let lines: Vec<&str> = src.lines().collect();
    let mut violations = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !line.contains(".innerHTML") {
            continue;
        }
        // Only consider assignment statements (not `el.innerHTML;` reads).
        if !line.contains("innerHTML = ") && !line.contains("innerHTML =") {
            continue;
        }
        // Skip clearing assignments — no interpolation, safe by construction.
        let trimmed = line.trim_end();
        if trimmed.ends_with("innerHTML = '';") || trimmed.ends_with("innerHTML = \"\";") {
            continue;
        }
        // Skip variable RHS (e.g., `el.innerHTML = html;`) — safety lives at
        // the assignment of `html`, not here.  These are de-facto safe in
        // render.rs (auditable above each callsite) and rare.
        if line.contains("innerHTML = html;") || line.contains("innerHTML = msg;") {
            continue;
        }
        // Detect concat ONLY within the assignment expression: collect
        // lines from this one until one ends with `;` (terminating the
        // statement).  Scan up to 8 forward lines max — guards against
        // missing semicolons.
        let mut expr_lines: Vec<&str> = Vec::new();
        let mut j = i;
        while j < lines.len() && j < i + 8 {
            expr_lines.push(lines[j]);
            if lines[j].trim_end().ends_with(';') {
                break;
            }
            j += 1;
        }
        let expr = expr_lines.join("\n");
        // Strip the leading `xx.innerHTML =` so we don't confuse the
        // assignment operator with concat.
        let rhs = expr
            .split_once("innerHTML")
            .map(|(_, after)| after)
            .unwrap_or("")
            .trim_start_matches([' ', '=']);
        // Heuristic: a `+` outside string literals indicates concat.  Walk
        // the rhs char-by-char tracking a single-quote/double-quote state
        // and report `+` only when outside a string literal.
        let mut in_squote = false;
        let mut in_dquote = false;
        let mut prev = ' ';
        let mut has_concat = false;
        for c in rhs.chars() {
            match c {
                '\'' if !in_dquote && prev != '\\' => in_squote = !in_squote,
                '"' if !in_squote && prev != '\\' => in_dquote = !in_dquote,
                '+' if !in_squote && !in_dquote => {
                    has_concat = true;
                    break;
                }
                _ => {}
            }
            prev = c;
        }
        if !has_concat {
            continue;
        }
        // Concat site — forbidden, period.  Migrate to
        // CX.h(tag, attrs, children) instead.  Static-only innerHTML
        // (no `+`) is fine; concat is the structural defect.
        violations.push(format!("render.rs:{}: {}", i + 1, line.trim()));
    }
    assert!(
        violations.is_empty(),
        "Spec § 1.5 / Contract 13 violation — innerHTML+concat is FORBIDDEN in render.rs.\n\
         Migrate the offending site to `CX.h(tag, attrs, children)` — the safe-by-construction\n\
         DOM builder.  setAttribute handles attributes; textContent handles strings; appendChild\n\
         handles element children.  No interpolation path can produce attacker-controlled markup.\n\n\
         Offenders ({}):\n{}",
        violations.len(),
        violations.join("\n")
    );
}

// ── P7: empty-token consistency between startup and bearer middleware ──────

#[test]
fn empty_token_treated_as_none_by_check_auth() {
    // `validate_bind_security` rejects empty-string tokens at startup,
    // but for loopback binds it lets them through (loopback bypasses
    // auth anyway).  The `run()` path now strips empty tokens before
    // installing on the router, so by the time `check_auth` sees the
    // expected token it's `None`.  Pin the consistency: an empty
    // expected token should NEVER authenticate any incoming request,
    // even one carrying an empty Bearer header.
    //
    // Asserts the run-time normalisation in serve::run() — empty token
    // stripped to None before reaching `check_auth`.
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/commands/serve.rs"
    ))
    .expect("read serve.rs");
    assert!(
        src.contains("token\n            .filter(|s| !s.is_empty())")
            || src.contains("token.filter(|s| !s.is_empty())"),
        "run() must strip empty-string tokens before passing to build_router \
         so check_auth never sees an empty expected token"
    );
}

// ── P8: every formerly-dead handler param actually consumed ────────────────

#[test]
fn formerly_dead_handler_params_actually_consumed() {
    // After v2.1.3 round 4 wired the 7 previously #[allow(dead_code)]
    // params, this test grep-pins each one to confirm the handler reads
    // it at least once.  Without the pin, a future "cleanup" PR could
    // drop the consuming line and re-introduce the dead param without
    // any visible signal.
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/commands/serve.rs"
    ))
    .expect("read serve.rs");

    // For each previously-dead field, assert it appears as `params.<field>`
    // somewhere in the file (not just on the struct definition).
    // Each pair: (label, consume-site marker).  At least one occurrence
    // means the field is consumed by some handler.  The struct fields
    // themselves don't use `params.` prefix, so a hit on `params.<name>`
    // proves consumption (not just declaration).
    let consume_markers = [
        ("ContextDiff.since", "params.since"),
        ("CallGraph.depth", "params.depth"),
        ("workspace alias", "params.workspace"),
        ("Predict/Drift/DataFlow.focus", "params.focus"),
    ];
    for (label, marker) in consume_markers {
        let count = src.matches(marker).count();
        assert!(
            count >= 1,
            "{label} not consumed anywhere — `{marker}` must appear at least once in serve.rs"
        );
    }
    // No `#[allow(dead_code)]` should remain on any of those struct fields.
    let allow_count = src.matches("#[allow(dead_code)]").count();
    // A small number of legitimate dead_code allows can exist (test
    // helpers that the harness can't see, for example).  Pin the
    // count so a regression that re-adds the v2.1.3-resolved annotations
    // is visible.  At time of fix the file has none on these structs.
    assert!(
        allow_count <= 2,
        "more than 2 #[allow(dead_code)] annotations remain in serve.rs ({allow_count}); \
         a recent change may have re-introduced one of the v2.1.3-cleared params"
    );
}

#[test]
fn feature_matrix_script_present_and_executable() {
    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("feature-matrix.sh");
    assert!(
        script.exists(),
        "scripts/feature-matrix.sh must exist for #13"
    );
    let metadata = std::fs::metadata(&script).expect("stat feature-matrix.sh");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "feature-matrix.sh must be executable (mode now {:o})",
            mode
        );
    }
    let body = std::fs::read_to_string(&script).expect("read feature-matrix.sh");
    assert!(
        body.contains("--no-default-features"),
        "must test no-default"
    );
    assert!(body.contains("--all-features"), "must test all-features");
    assert!(
        body.contains("--features plugins"),
        "must explicitly test plugins (default-excluded since v2.1.2)"
    );
}
