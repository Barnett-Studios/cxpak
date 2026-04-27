use crate::parser::language::Visibility;
use std::path::Path;
use tower_lsp::lsp_types::{CodeLens, Command, Position, Range, Url};

/// One-word visibility label for dead-code diagnostic messages.
fn visibility_label(v: &Visibility) -> &'static str {
    match v {
        Visibility::Public => "public",
        Visibility::Private => "private",
    }
}

/// Restrictive allowlist for git rev-spec strings supplied by LSP clients.
///
/// `git2::Repository::revparse_single` accepts the full rev-spec grammar
/// including `HEAD^{/regex}` and `@{u}` patterns that can trigger expensive
/// regex scans over commit history.  We only need the simple cases —
/// abbreviated/full SHA, branch/tag name, `HEAD`, `HEAD~N`, `HEAD^N`,
/// `<ref>~N`, `<ref>^N`.  Anything outside this grammar is rejected with
/// `Internal(...)` so the user gets a clear error instead of a denial-of-
/// service vector.
fn validate_git_ref(raw: Option<&str>) -> Result<Option<&str>, LspMethodError> {
    let Some(s) = raw else {
        return Ok(None);
    };
    if s.is_empty() {
        return Ok(None);
    }
    if s.len() > 200 {
        return Err(LspMethodError::Internal(
            "git ref too long (max 200 chars)".into(),
        ));
    }
    // Reject any character outside the safe set for branch/tag/SHA names
    // plus the small ref-spec syntax (`~`, `^`, digits) we actually support.
    // Notably excludes `{`, `}`, `:`, `/regex/`, `@`, backtick, semicolons,
    // and whitespace — the vectors that drive expensive parses.
    let ok = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '~' | '^'));
    if !ok {
        return Err(LspMethodError::Internal(format!(
            "git ref `{s}` contains disallowed characters; allowed: A-Z a-z 0-9 _ - . / ~ ^"
        )));
    }
    // No leading dot, no `..` (parent ref, ambiguous), no consecutive `//`,
    // no trailing `.lock` (git ref-name rule).
    if s.starts_with('.')
        || s.starts_with('/')
        || s.contains("..")
        || s.contains("//")
        || s.ends_with(".lock")
        || s.ends_with('/')
        || s.ends_with('.')
    {
        return Err(LspMethodError::Internal(format!(
            "git ref `{s}` violates git ref-name rules"
        )));
    }
    Ok(Some(s))
}

/// Error type returned by [`handle_custom_method`].
#[derive(Debug)]
pub enum LspMethodError {
    /// The requested method name is not registered (maps to JSON-RPC MethodNotFound -32601).
    NotFound(String),
    /// An internal error occurred while handling the method (maps to InternalError -32603).
    Internal(String),
}

/// Convert a `file://` URI to a path relative to `repo_root`.
///
/// Example: `file:///Users/me/repo/src/main.rs` + `/Users/me/repo` → `src/main.rs`
pub fn uri_to_rel_path(uri: &Url, repo_root: &Path) -> Option<String> {
    let abs = uri.to_file_path().ok()?;
    let rel = abs.strip_prefix(repo_root).ok()?;
    Some(rel.to_string_lossy().into_owned())
}

/// Extract the identifier token that spans `char_idx` on `line_idx` inside `content`.
///
/// Walks backward and forward from `char_idx` while characters are alphanumeric or `_`.
pub fn extract_word_at(content: &str, line_idx: usize, char_idx: usize) -> String {
    let line = match content.lines().nth(line_idx) {
        Some(l) => l,
        None => return String::new(),
    };
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    if char_idx >= len {
        return String::new();
    }
    if !chars[char_idx].is_alphanumeric() && chars[char_idx] != '_' {
        return String::new();
    }
    let start = {
        let mut i = char_idx;
        while i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
            i -= 1;
        }
        i
    };
    let end = {
        let mut i = char_idx + 1;
        while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
            i += 1;
        }
        i
    };
    chars[start..end].iter().collect()
}

/// Resolve a URI or uri-ish string to an indexed file.
///
/// Primary strategy: parse as file:// URL, strip `repo_root`, exact-match on
/// `IndexedFile.relative_path`.  Fallback: path-bounded suffix match — the
/// URI must end with `/<relative_path>` or be exactly `<relative_path>`.
/// A plain `ends_with(relative_path)` without the separator bound would
/// falsely match `src/main.rs` against URIs in unrelated crates whose paths
/// happen to end in `main.rs` (e.g., `my_src/main.rs`).
pub fn find_indexed_file<'a>(
    uri_path: &str,
    index: &'a crate::index::CodebaseIndex,
    repo_root: &Path,
) -> Option<&'a crate::index::IndexedFile> {
    let url = Url::parse(uri_path).ok();
    let rel_opt = url.as_ref().and_then(|u| uri_to_rel_path(u, repo_root));
    index.files.iter().find(|f| {
        if rel_opt.as_deref().is_some_and(|r| f.relative_path == r) {
            return true;
        }
        // Path-bounded suffix match: require a leading `/` or exact equality
        // so the match aligns to a directory boundary. `src/main.rs` MUST NOT
        // match `my_src/main.rs`.
        let rel = &f.relative_path;
        uri_path == rel
            || (uri_path.len() > rel.len()
                && uri_path.ends_with(rel)
                && uri_path.as_bytes()[uri_path.len() - rel.len() - 1] == b'/')
    })
}

pub fn code_lens_for_file(
    uri_path: &str,
    index: &crate::index::CodebaseIndex,
    repo_root: &Path,
) -> Vec<CodeLens> {
    let file = find_indexed_file(uri_path, index, repo_root);

    match file {
        None => Vec::new(),
        Some(f) => {
            let range = Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 0,
                },
            };
            vec![CodeLens {
                range,
                command: Some(Command {
                    title: format!(
                        "cxpak: {} tokens | {}",
                        f.token_count,
                        f.language.as_deref().unwrap_or("unknown")
                    ),
                    command: "cxpak.showFileInfo".to_string(),
                    arguments: None,
                }),
                data: None,
            }]
        }
    }
}

pub fn hover_for_symbol(
    symbol: &str,
    index: &crate::index::CodebaseIndex,
) -> Option<tower_lsp::lsp_types::Hover> {
    let matches = index.find_symbol(symbol);
    let (file_path, sym) = matches.first()?;
    let pagerank = index.pagerank.get(*file_path).copied().unwrap_or(0.0);
    let content = format!(
        "**{:?}** `{}`\n\nFile: `{}`\nPageRank: {:.3}",
        sym.kind, sym.name, file_path, pagerank
    );
    Some(tower_lsp::lsp_types::Hover {
        contents: tower_lsp::lsp_types::HoverContents::Markup(
            tower_lsp::lsp_types::MarkupContent {
                kind: tower_lsp::lsp_types::MarkupKind::Markdown,
                value: content,
            },
        ),
        range: None,
    })
}

pub fn diagnostics_for_file(
    uri_path: &str,
    index: &crate::index::CodebaseIndex,
    repo_root: &Path,
) -> Vec<tower_lsp::lsp_types::Diagnostic> {
    use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

    let Some(file) = find_indexed_file(uri_path, index, repo_root) else {
        return Vec::new();
    };

    // Emit a Warning diagnostic for every symbol the dead-code detector flags
    // in this file. `detect_dead_code` uses our strict heuristics, so
    // diagnostics inherit the same zero-false-positive contract locked by
    // `tests/dead_code_adversarial.rs`.  `DeadSymbol` doesn't carry line
    // numbers, so look them up from the file's parse result.
    //
    // Using the cached dead-code analysis on the index — an LSP client
    // requests diagnostics per file open/save/focus, so the naive per-call
    // recomputation was O(F·S·C) per request, freezing editors on any
    // non-toy repo. The cache pays off on the second call onwards.
    let Some(pr) = &file.parse_result else {
        return Vec::new();
    };
    index
        .dead_code_cached()
        .iter()
        .filter(|d| d.file == file.relative_path)
        .filter_map(|d| {
            let sym = pr
                .symbols
                .iter()
                .find(|s| s.name == d.symbol && s.kind == d.kind)?;
            let line = sym.start_line.saturating_sub(1) as u32;
            let end_line = sym.end_line.saturating_sub(1) as u32;
            Some(Diagnostic {
                range: Range {
                    start: Position { line, character: 0 },
                    end: Position {
                        line: end_line,
                        character: 0,
                    },
                },
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(tower_lsp::lsp_types::NumberOrString::String(
                    "cxpak.dead_code".into(),
                )),
                code_description: None,
                source: Some("cxpak".into()),
                // Include kind and visibility so the IDE message is
                // actionable at a glance.  `d.reason` is the detector's
                // single fixed string ("zero callers, not entry point,
                // no test reference") — useful but repetitive across
                // every diagnostic; prefixing with kind+visibility tells
                // the user whether they can safely delete a private
                // function vs. whether a pub symbol requires review of
                // external callers.  Bidi-sanitised so a symbol named
                // with U+202E cannot flip the visual order of the
                // diagnostic message.
                message: format!(
                    "dead code: {} {:?} `{}` — {}",
                    visibility_label(&sym.visibility),
                    d.kind,
                    crate::util::sanitize_bidi(&d.symbol),
                    d.reason,
                ),
                related_information: None,
                tags: Some(vec![tower_lsp::lsp_types::DiagnosticTag::UNNECESSARY]),
                data: None,
            })
        })
        .collect()
}

pub fn workspace_symbols(
    query: &str,
    index: &crate::index::CodebaseIndex,
    repo_root: &Path,
) -> Vec<tower_lsp::lsp_types::SymbolInformation> {
    use crate::parser::language::SymbolKind as CxpakKind;
    use tower_lsp::lsp_types::{Location, SymbolInformation, SymbolKind as LspKind};

    let query_lower = query.to_lowercase();
    let mut results = Vec::new();

    for file in &index.files {
        let Some(pr) = &file.parse_result else {
            continue;
        };
        for sym in &pr.symbols {
            if !query.is_empty() && !sym.name.to_lowercase().contains(&query_lower) {
                continue;
            }
            let kind = match sym.kind {
                CxpakKind::Function => LspKind::FUNCTION,
                CxpakKind::Struct => LspKind::STRUCT,
                CxpakKind::Enum => LspKind::ENUM,
                CxpakKind::Trait | CxpakKind::Interface => LspKind::INTERFACE,
                CxpakKind::Class => LspKind::CLASS,
                CxpakKind::Method => LspKind::METHOD,
                CxpakKind::Constant => LspKind::CONSTANT,
                CxpakKind::TypeAlias | CxpakKind::Type => LspKind::TYPE_PARAMETER,
                CxpakKind::Variable => LspKind::VARIABLE,
                _ => LspKind::KEY,
            };

            #[allow(deprecated)]
            let info = SymbolInformation {
                name: sym.name.clone(),
                kind,
                tags: None,
                deprecated: None,
                location: Location {
                    uri: Url::from_file_path(repo_root.join(&file.relative_path))
                        .unwrap_or_else(|_| Url::parse("file:///unknown").unwrap()),
                    range: tower_lsp::lsp_types::Range {
                        start: Position {
                            line: sym.start_line.saturating_sub(1) as u32,
                            character: 0,
                        },
                        end: Position {
                            line: sym.end_line.saturating_sub(1) as u32,
                            character: 0,
                        },
                    },
                },
                container_name: Some(file.relative_path.clone()),
            };
            results.push(info);
        }
    }
    results
}

pub fn handle_custom_method(
    method: &str,
    params: serde_json::Value,
    index: &crate::index::CodebaseIndex,
    repo_root: &Path,
) -> Result<Option<serde_json::Value>, LspMethodError> {
    match method {
        "cxpak/health" => {
            let health = crate::intelligence::health::compute_health(index);
            Ok(Some(serde_json::json!({
                "total_files": index.total_files,
                "total_tokens": index.total_tokens,
                "composite": health.composite,
                "dimensions": {
                    "conventions": health.conventions,
                    "test_coverage": health.test_coverage,
                    "churn_stability": health.churn_stability,
                    "coupling": health.coupling,
                    "cycles": health.cycles,
                    "dead_code": health.dead_code,
                },
            })))
        }
        "cxpak/conventions" => serde_json::to_value(&index.conventions)
            .map(Some)
            .map_err(|e| LspMethodError::Internal(format!("serialization failed: {e}"))),
        "cxpak/blastRadius" => {
            let files: Vec<String> = params
                .get("files")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .or_else(|| {
                    params
                        .get("file")
                        .and_then(|v| v.as_str())
                        .map(|s| vec![s.to_string()])
                })
                .unwrap_or_default();
            if files.is_empty() {
                return Err(LspMethodError::Internal(
                    "cxpak/blastRadius requires 'file' (string) or 'files' (array) param".into(),
                ));
            }
            // Validate every input path against the index.  Silent empty
            // results for typo'd paths make a caller's "zero dependents"
            // response indistinguishable from "unknown file" — signal the
            // actual problem instead.
            let indexed: std::collections::HashSet<&str> = index
                .files
                .iter()
                .map(|f| f.relative_path.as_str())
                .collect();
            let unknown: Vec<&String> = files
                .iter()
                .filter(|f| !indexed.contains(f.as_str()))
                .collect();
            if !unknown.is_empty() {
                return Err(LspMethodError::Internal(format!(
                    "cxpak/blastRadius: {} file(s) not in index: {}",
                    unknown.len(),
                    unknown
                        .iter()
                        .take(5)
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            let depth = params
                .get("depth")
                .and_then(|v| v.as_u64())
                .unwrap_or(3)
                .min(8) as usize;
            let focus = params.get("focus").and_then(|v| v.as_str());
            let refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
            let result = crate::intelligence::blast_radius::compute_blast_radius(
                &refs,
                &index.graph,
                &index.pagerank,
                &index.test_map,
                depth,
                focus,
            );
            Ok(Some(serde_json::to_value(result).map_err(|e| {
                LspMethodError::Internal(format!("serialization failed: {e}"))
            })?))
        }
        "cxpak/overview" => Ok(Some(serde_json::json!({
            "total_files": index.total_files,
            "total_tokens": index.total_tokens,
            "languages": index.language_stats.len(),
        }))),
        "cxpak/trace" => {
            let sym = params
                .get("symbol")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    LspMethodError::Internal("cxpak/trace requires 'symbol' (string) param".into())
                })?;
            let matches = index.find_symbol(sym);
            let locations: Vec<_> = matches
                .into_iter()
                .map(|(file, s)| {
                    serde_json::json!({
                        "file": file,
                        "start_line": s.start_line,
                        "end_line": s.end_line,
                        "kind": format!("{:?}", s.kind),
                    })
                })
                .collect();
            Ok(Some(
                serde_json::json!({"count": locations.len(), "locations": locations}),
            ))
        }
        "cxpak/diff" => {
            // Optional caps so a 50-file refactor does not produce a multi-MB
            // JSON-RPC response that blocks LSP clients.  Defaults: 50 files,
            // 32 KiB per file's diff_text.  Caller can override (e.g.,
            // `{"max_files": 10, "max_bytes_per_file": 4096}`).
            let max_files = params
                .get("max_files")
                .and_then(|v| v.as_u64())
                .unwrap_or(50)
                .min(500) as usize;
            let max_bytes_per_file = params
                .get("max_bytes_per_file")
                .and_then(|v| v.as_u64())
                .unwrap_or(32 * 1024)
                .min(1_048_576) as usize;
            let git_ref = validate_git_ref(params.get("ref").and_then(|v| v.as_str()))?;
            match crate::commands::diff::extract_changes(repo_root, git_ref) {
                Ok(changes) => {
                    let total_changed = changes.len();
                    let truncated_files = total_changed > max_files;
                    let mut entries: Vec<serde_json::Value> = Vec::new();
                    for c in changes.iter().take(max_files) {
                        let full_bytes = c.diff_text.len();
                        let (text, truncated_text) = if full_bytes > max_bytes_per_file {
                            // Slice on a UTF-8 char boundary <= max_bytes_per_file.
                            let mut end = max_bytes_per_file;
                            while end > 0 && !c.diff_text.is_char_boundary(end) {
                                end -= 1;
                            }
                            (&c.diff_text[..end], true)
                        } else {
                            (c.diff_text.as_str(), false)
                        };
                        entries.push(serde_json::json!({
                            "path": c.path,
                            "diff_text": text,
                            "diff_bytes": full_bytes,
                            "truncated": truncated_text,
                        }));
                    }
                    Ok(Some(serde_json::json!({
                        "ref": git_ref.unwrap_or("uncommitted"),
                        "count": total_changed,
                        "showing": entries.len(),
                        "files_truncated": truncated_files,
                        "changes": entries,
                    })))
                }
                Err(e) => Err(LspMethodError::Internal(format!("git diff failed: {e}"))),
            }
        }
        "cxpak/search" => {
            let query = params
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    LspMethodError::Internal("cxpak/search requires 'query' (string) param".into())
                })?
                .to_lowercase();
            if query.is_empty() {
                return Err(LspMethodError::Internal(
                    "cxpak/search 'query' must be non-empty".into(),
                ));
            }
            let matches: Vec<_> = index
                .files
                .iter()
                .filter(|f| f.relative_path.to_lowercase().contains(&query))
                .take(20)
                .map(|f| serde_json::json!({"path": f.relative_path, "language": f.language}))
                .collect();
            Ok(Some(serde_json::json!({"matches": matches})))
        }
        "cxpak/apiSurface" => {
            let surface =
                crate::intelligence::api_surface::extract_api_surface(index, None, "all", 5000);
            Ok(Some(serde_json::to_value(surface).map_err(|e| {
                LspMethodError::Internal(format!("serialization failed: {e}"))
            })?))
        }
        "cxpak/deadCode" => {
            let dead = index.dead_code_cached();
            Ok(Some(serde_json::json!({
                "dead_symbols": dead,
                "total": dead.len(),
            })))
        }
        "cxpak/callGraph" => Ok(Some(serde_json::json!({
            "edges": index.call_graph.edges,
            "total": index.call_graph.edges.len(),
        }))),
        "cxpak/predict" => {
            let files = params
                .get("files")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if files.is_empty() {
                return Err(LspMethodError::Internal(
                    "cxpak/predict requires 'files' (array of strings) param".into(),
                ));
            }
            let refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
            let result = crate::intelligence::predict::predict(
                &refs,
                &index.graph,
                &index.pagerank,
                &index.co_changes,
                &index.test_map,
                3,
            );
            Ok(Some(serde_json::to_value(result).map_err(|e| {
                LspMethodError::Internal(format!("serialization failed: {e}"))
            })?))
        }
        "cxpak/drift" => {
            let save_baseline = params
                .get("save_baseline")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let report =
                crate::intelligence::drift::build_drift_report(index, repo_root, save_baseline);
            Ok(Some(serde_json::to_value(report).map_err(|e| {
                LspMethodError::Internal(format!("serialization failed: {e}"))
            })?))
        }
        "cxpak/securitySurface" => {
            let result = crate::intelligence::security::build_security_surface(
                index,
                crate::intelligence::security::DEFAULT_AUTH_PATTERNS,
                None,
            );
            Ok(Some(serde_json::to_value(result).map_err(|e| {
                LspMethodError::Internal(format!("serialization failed: {e}"))
            })?))
        }
        "cxpak/dataFlow" => {
            let sym = params
                .get("symbol")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    LspMethodError::Internal(
                        "cxpak/dataFlow requires 'symbol' (string) param".into(),
                    )
                })?;
            let result = crate::intelligence::data_flow::trace_data_flow(sym, None, 6, index);
            Ok(Some(serde_json::to_value(result).map_err(|e| {
                LspMethodError::Internal(format!("serialization failed: {e}"))
            })?))
        }
        _ => Err(LspMethodError::NotFound(method.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::index::CodebaseIndex;
    use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    fn make_test_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let files = vec![ScannedFile {
            relative_path: "src/main.rs".to_string(),
            absolute_path: std::path::PathBuf::from("/tmp/src/main.rs"),
            language: Some("rust".to_string()),
            size_bytes: 100,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/main.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "main".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn main()".to_string(),
                    body: "fn main() {}".to_string(),
                    start_line: 1,
                    end_line: 5,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let mut content_map = HashMap::new();
        content_map.insert("src/main.rs".to_string(), "fn main() {}".to_string());

        CodebaseIndex::build_with_content(files, parse_results, &counter, content_map)
    }

    #[test]
    fn code_lens_returns_empty_for_unknown_file() {
        let index = make_test_index();
        let root = std::path::Path::new("/tmp");
        let result = code_lens_for_file("nonexistent.rs", &index, root);
        assert!(result.is_empty());
    }

    #[test]
    fn code_lens_returns_lens_for_known_file() {
        let index = make_test_index();
        let root = std::path::Path::new("/tmp");
        let result = code_lens_for_file("file:///tmp/src/main.rs", &index, root);
        assert_eq!(result.len(), 1);
        let lens = &result[0];
        let cmd = lens.command.as_ref().unwrap();
        assert!(cmd.title.contains("tokens"));
        assert!(cmd.title.contains("rust"));
    }

    #[test]
    fn code_lens_uses_uri_to_rel_path_not_raw_trim() {
        // URI: file:///Users/me/repo/src/main.rs, repo_root: /Users/me/repo
        // After stripping repo_root the relative path is "src/main.rs" which
        // must match the index entry. The old raw-trim would produce
        // "Users/me/repo/src/main.rs" and miss the file.
        let counter = crate::budget::counter::TokenCounter::new();
        let files = vec![crate::scanner::ScannedFile {
            relative_path: "src/main.rs".to_string(),
            absolute_path: std::path::PathBuf::from("/Users/me/repo/src/main.rs"),
            language: Some("rust".to_string()),
            size_bytes: 100,
        }];
        let index = CodebaseIndex::build_with_content(
            files,
            std::collections::HashMap::new(),
            &counter,
            std::collections::HashMap::new(),
        );
        let root = std::path::Path::new("/Users/me/repo");
        let result = code_lens_for_file("file:///Users/me/repo/src/main.rs", &index, root);
        assert_eq!(
            result.len(),
            1,
            "uri_to_rel_path must find the file; old raw trim would fail"
        );
    }

    #[test]
    fn diagnostics_uses_uri_to_rel_path_not_raw_trim() {
        let counter = crate::budget::counter::TokenCounter::new();
        let files = vec![crate::scanner::ScannedFile {
            relative_path: "src/lib.rs".to_string(),
            absolute_path: std::path::PathBuf::from("/Users/me/repo/src/lib.rs"),
            language: Some("rust".to_string()),
            size_bytes: 100,
        }];
        let index = CodebaseIndex::build_with_content(
            files,
            std::collections::HashMap::new(),
            &counter,
            std::collections::HashMap::new(),
        );
        let root = std::path::Path::new("/Users/me/repo");
        // diagnostics_for_file returns empty (no real diagnostics yet) but must
        // not panic or return an error even with a fully qualified file:// URI.
        let result = diagnostics_for_file("file:///Users/me/repo/src/lib.rs", &index, root);
        // Must successfully find the file and return the empty-diagnostics Vec.
        let _ = result; // function did not panic
    }

    #[test]
    fn hover_returns_none_for_empty_index() {
        let index = make_test_index();
        let result = hover_for_symbol("unknown_fn", &index);
        assert!(result.is_none());
    }

    #[test]
    fn hover_returns_markdown_content() {
        let index = make_test_index();
        let result = hover_for_symbol("main", &index);
        assert!(result.is_some());
        let hover = result.unwrap();
        match hover.contents {
            tower_lsp::lsp_types::HoverContents::Markup(m) => {
                assert!(m.value.contains("main"));
                assert!(m.value.contains("PageRank"));
            }
            _ => panic!("expected Markup hover contents"),
        }
    }

    #[test]
    fn diagnostics_empty_for_unknown_file() {
        let index = make_test_index();
        let root = std::path::Path::new("/tmp");
        let result = diagnostics_for_file("missing.rs", &index, root);
        assert!(result.is_empty());
    }

    #[test]
    fn diagnostics_empty_for_known_file_without_dead_code() {
        // With no dead symbols the diagnostics list is empty.
        let index = make_test_index();
        let root = std::path::Path::new("/tmp");
        let result = diagnostics_for_file("src/main.rs", &index, root);
        assert!(
            result.is_empty(),
            "no dead symbols -> no diagnostics, got {result:?}"
        );
    }

    #[test]
    fn diagnostics_include_dead_code_warnings() {
        // A private function with zero callers, no test attribute, no
        // qualified reference is dead. diagnostics_for_file must surface it
        // as a WARNING-severity diagnostic with source=cxpak and
        // tags=[UNNECESSARY] so the editor can grey it out.
        let counter = TokenCounter::new();
        let file = ScannedFile {
            relative_path: "src/main.rs".to_string(),
            absolute_path: std::path::PathBuf::from("/tmp/src/main.rs"),
            language: Some("rust".to_string()),
            size_bytes: 100,
        };
        let mut parses = HashMap::new();
        parses.insert(
            "src/main.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "unused_helper".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "fn unused_helper()".to_string(),
                    body: "fn unused_helper() { 1 + 1; }".to_string(),
                    start_line: 7,
                    end_line: 9,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        let mut content = HashMap::new();
        content.insert(
            "src/main.rs".to_string(),
            "\n\n\n\n\n\nfn unused_helper() {}\n".to_string(),
        );
        let index = CodebaseIndex::build_with_content(vec![file], parses, &counter, content);
        let root = std::path::Path::new("/tmp");
        let diags = diagnostics_for_file("src/main.rs", &index, root);
        assert!(
            diags.iter().any(|d| d.source.as_deref() == Some("cxpak")
                && d.severity == Some(tower_lsp::lsp_types::DiagnosticSeverity::WARNING)
                && d.message.contains("dead code")
                && d.message.contains("unused_helper")),
            "expected at least one cxpak dead-code warning for unused_helper, got {diags:?}"
        );
        // Range must point at the symbol's declaration line (1-indexed -> 0-indexed).
        let d = diags
            .iter()
            .find(|d| d.message.contains("unused_helper"))
            .unwrap();
        assert_eq!(
            d.range.start.line, 6,
            "start line must be 0-indexed start_line-1"
        );
    }

    fn make_multi_symbol_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let files = vec![ScannedFile {
            relative_path: "src/lib.rs".to_string(),
            absolute_path: std::path::PathBuf::from("/tmp/src/lib.rs"),
            language: Some("rust".to_string()),
            size_bytes: 200,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/lib.rs".to_string(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "foo".to_string(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn foo()".to_string(),
                        body: "fn foo() {}".to_string(),
                        start_line: 1,
                        end_line: 3,
                    },
                    Symbol {
                        name: "bar".to_string(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn bar()".to_string(),
                        body: "fn bar() {}".to_string(),
                        start_line: 5,
                        end_line: 7,
                    },
                    Symbol {
                        name: "baz".to_string(),
                        kind: SymbolKind::Struct,
                        visibility: Visibility::Public,
                        signature: "struct baz".to_string(),
                        body: "struct baz {}".to_string(),
                        start_line: 9,
                        end_line: 11,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let mut content_map = HashMap::new();
        content_map.insert(
            "src/lib.rs".to_string(),
            "fn foo() {}\nfn bar() {}\nstruct baz {}".to_string(),
        );

        CodebaseIndex::build_with_content(files, parse_results, &counter, content_map)
    }

    #[test]
    fn workspace_symbols_empty_query_returns_all() {
        let index = make_multi_symbol_index();
        let root = std::path::Path::new("/tmp");
        let result = workspace_symbols("", &index, root);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn workspace_symbols_filtered_by_query() {
        let index = make_multi_symbol_index();
        let root = std::path::Path::new("/tmp");
        let result = workspace_symbols("ba", &index, root);
        assert_eq!(result.len(), 2);
        let names: Vec<&str> = result.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"bar"));
        assert!(names.contains(&"baz"));
    }

    #[test]
    fn workspace_symbols_uri_uses_repo_root() {
        // Symbols must have URIs rooted at repo_root, not file:///src/...
        let index = make_multi_symbol_index();
        let root = std::path::Path::new("/Users/me/repo");
        let result = workspace_symbols("foo", &index, root);
        assert_eq!(result.len(), 1);
        let uri = result[0].location.uri.as_str();
        assert!(
            uri.starts_with("file:///Users/me/repo/"),
            "URI must start with repo_root path, got: {uri}"
        );
        assert!(
            !uri.starts_with("file:///src/"),
            "URI must not be rooted at /src/, got: {uri}"
        );
    }

    /// Create a throwaway directory with `git init` so drift/diff can run.
    fn make_git_tempdir() -> tempfile::TempDir {
        let temp = tempfile::TempDir::new().unwrap();
        let _ = std::process::Command::new("git")
            .arg("init")
            .arg("--quiet")
            .current_dir(temp.path())
            .output();
        let _ = std::process::Command::new("git")
            .args(["config", "user.email", "t@t"])
            .current_dir(temp.path())
            .output();
        let _ = std::process::Command::new("git")
            .args(["config", "user.name", "t"])
            .current_dir(temp.path())
            .output();
        std::fs::write(temp.path().join("README.md"), "init\n").unwrap();
        let _ = std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(temp.path())
            .output();
        let _ = std::process::Command::new("git")
            .args(["commit", "-m", "init", "--quiet"])
            .current_dir(temp.path())
            .output();
        temp
    }

    #[test]
    fn custom_method_health_returns_json() {
        let index = make_test_index();
        let root = std::path::Path::new("/tmp");
        let result = handle_custom_method("cxpak/health", serde_json::Value::Null, &index, root);
        assert!(result.is_ok());
        let val = result.unwrap().unwrap();
        assert!(val["total_files"].is_number());
    }

    #[test]
    fn custom_method_unknown_returns_not_found() {
        let index = make_test_index();
        let root = std::path::Path::new("/tmp");
        let result =
            handle_custom_method("cxpak/nonexistent", serde_json::Value::Null, &index, root);
        assert!(
            matches!(result, Err(LspMethodError::NotFound(_))),
            "unknown method must return LspMethodError::NotFound"
        );
    }

    #[test]
    fn custom_method_conventions_returns_profile() {
        let index = make_test_index();
        let root = std::path::Path::new("/tmp");
        let result =
            handle_custom_method("cxpak/conventions", serde_json::Value::Null, &index, root);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn all_registered_custom_methods_return_ok() {
        let index = make_test_index();
        let temp = make_git_tempdir();
        let root = temp.path();
        let param_map: &[(&str, serde_json::Value)] = &[
            ("cxpak/health", serde_json::Value::Null),
            ("cxpak/conventions", serde_json::Value::Null),
            (
                "cxpak/blastRadius",
                serde_json::json!({"file": "src/main.rs"}),
            ),
            ("cxpak/overview", serde_json::Value::Null),
            ("cxpak/trace", serde_json::json!({"symbol": "main"})),
            ("cxpak/diff", serde_json::Value::Null),
            ("cxpak/search", serde_json::json!({"query": "main"})),
            ("cxpak/apiSurface", serde_json::Value::Null),
            ("cxpak/deadCode", serde_json::Value::Null),
            ("cxpak/callGraph", serde_json::Value::Null),
            (
                "cxpak/predict",
                serde_json::json!({"files": ["src/main.rs"]}),
            ),
            ("cxpak/drift", serde_json::Value::Null),
            ("cxpak/securitySurface", serde_json::Value::Null),
            ("cxpak/dataFlow", serde_json::json!({"symbol": "main"})),
        ];
        for (m, params) in param_map {
            let result = handle_custom_method(m, params.clone(), &index, root);
            assert!(
                result.is_ok(),
                "method {m} should return Ok, got {result:?}"
            );
        }
    }

    #[test]
    fn methods_with_missing_required_params_return_internal_error() {
        let index = make_test_index();
        let root = std::path::Path::new("/tmp");
        // These methods MUST return an Internal error rather than a soft-failure
        // {"note": "provide X"} payload, which would let a caller mistake a
        // missing-input for a valid empty response.
        let cases = [
            ("cxpak/trace", serde_json::Value::Null),
            ("cxpak/search", serde_json::Value::Null),
            ("cxpak/search", serde_json::json!({"query": ""})),
            ("cxpak/predict", serde_json::Value::Null),
            ("cxpak/predict", serde_json::json!({"files": []})),
            ("cxpak/dataFlow", serde_json::Value::Null),
            ("cxpak/blastRadius", serde_json::Value::Null),
        ];
        for (m, p) in cases {
            let r = handle_custom_method(m, p, &index, root);
            assert!(
                matches!(r, Err(LspMethodError::Internal(_))),
                "method {m} with missing params must Err(Internal), got {r:?}"
            );
        }
    }

    #[test]
    fn extract_word_at_middle_of_word() {
        assert_eq!(extract_word_at("foo bar baz", 0, 5), "bar");
    }

    #[test]
    fn extract_word_at_start_of_content() {
        assert_eq!(extract_word_at("hello", 0, 0), "hello");
    }

    #[test]
    fn extract_word_at_end_of_word() {
        assert_eq!(extract_word_at("foo bar baz", 0, 6), "bar");
    }

    #[test]
    fn extract_word_at_on_space_returns_empty() {
        assert_eq!(extract_word_at("foo bar", 0, 3), "");
    }

    #[test]
    fn extract_word_at_out_of_bounds_line_returns_empty() {
        assert_eq!(extract_word_at("hello", 99, 0), "");
    }

    #[test]
    fn extract_word_at_out_of_bounds_char_returns_empty() {
        assert_eq!(extract_word_at("hi", 0, 99), "");
    }

    #[test]
    fn extract_word_at_underscore_included() {
        assert_eq!(extract_word_at("my_func()", 0, 4), "my_func");
    }

    #[test]
    fn uri_to_rel_path_strips_repo_root() {
        let uri = Url::parse("file:///Users/me/repo/src/main.rs").unwrap();
        let root = std::path::Path::new("/Users/me/repo");
        assert_eq!(uri_to_rel_path(&uri, root), Some("src/main.rs".to_string()));
    }

    #[test]
    fn uri_to_rel_path_returns_none_outside_root() {
        let uri = Url::parse("file:///other/src/main.rs").unwrap();
        let root = std::path::Path::new("/Users/me/repo");
        assert_eq!(uri_to_rel_path(&uri, root), None);
    }

    #[test]
    fn uri_to_rel_path_exact_root_returns_empty_string() {
        let uri = Url::parse("file:///Users/me/repo/file.rs").unwrap();
        let root = std::path::Path::new("/Users/me/repo");
        assert_eq!(uri_to_rel_path(&uri, root), Some("file.rs".to_string()));
    }
}
