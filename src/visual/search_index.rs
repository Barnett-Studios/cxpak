//! Pre-computes a fuzzy search index from a CodebaseIndex.

use crate::index::CodebaseIndex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEntry {
    pub label: String,
    pub kind: String,
    pub context: String,
    pub detail: String,
    pub target: String,
}

pub fn build_search_index(index: &CodebaseIndex) -> Vec<SearchEntry> {
    use crate::parser::language::Visibility;
    const CAP: usize = 20_000;
    let mut entries: Vec<SearchEntry> = Vec::new();

    // 1. Views (6 fixed navigation entries). Lexicographically "view" > "symbol" > "module" > "file",
    // so views will sort to the END of the final list after step 5. The cap branch at step 6 filters
    // views explicitly to ensure they are always retained regardless of sort position.
    for (label, hash) in [
        ("Dashboard", "#dashboard"),
        ("Architecture Explorer", "#architecture"),
        ("Risk Heatmap", "#risk"),
        ("Flow Diagram", "#flow"),
        ("Time Machine", "#timeline"),
        ("Diff View", "#diff"),
    ] {
        entries.push(SearchEntry {
            label: label.to_string(),
            kind: "view".to_string(),
            context: String::new(),
            detail: "View".to_string(),
            target: hash.to_string(),
        });
    }

    // 2. Files (all — including parse-failed). Skip paths with NUL bytes; they
    // are invalid on all target platforms and would corrupt JSON output.
    for file in &index.files {
        if file.relative_path.contains('\0') {
            continue;
        }
        let language = file.language.clone().unwrap_or_else(|| "unknown".into());
        let detail = if file.parse_result.is_none() {
            format!("{language} · parse error")
        } else {
            format!("{language} · {} tokens", file.token_count)
        };
        entries.push(SearchEntry {
            label: file.relative_path.clone(),
            kind: "file".to_string(),
            context: language.clone(),
            detail,
            target: format!("#architecture?file={}", file.relative_path),
        });
    }

    // 3. Public symbols only.
    for file in &index.files {
        let Some(pr) = &file.parse_result else {
            continue;
        };
        for sym in &pr.symbols {
            if !matches!(sym.visibility, Visibility::Public) {
                continue;
            }
            entries.push(SearchEntry {
                label: sym.name.clone(),
                kind: "symbol".to_string(),
                context: file.relative_path.clone(),
                detail: format!("{:?} in {}", sym.kind, file.relative_path),
                target: format!("#architecture?file={}", file.relative_path),
            });
        }
    }

    // 4. Module prefixes (first two path segments) deduped.
    let mut modules: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for f in &index.files {
        let segs: Vec<&str> = f.relative_path.split('/').collect();
        if segs.len() >= 3 {
            modules.insert(format!("{}/{}", segs[0], segs[1]));
        }
        // Also pick up single first-segment modules (e.g. "src" from "src/foo.rs")
        if let Some((first, _)) = f.relative_path.split_once('/') {
            modules.insert(first.to_string());
        }
    }
    for m in &modules {
        let count = index
            .files
            .iter()
            .filter(|f| f.relative_path == *m || f.relative_path.starts_with(&format!("{m}/")))
            .count();
        entries.push(SearchEntry {
            label: m.clone(),
            kind: "module".to_string(),
            context: String::new(),
            detail: format!("{count} files"),
            target: format!("#architecture?module={m}"),
        });
    }

    // 5. Sort by (kind, label, context) for determinism.
    entries.sort_by(|a, b| (&a.kind, &a.label, &a.context).cmp(&(&b.kind, &b.label, &b.context)));

    // 6. Cap at 20,000 entries: views always kept, then modules, then files/symbols by PageRank desc.
    if entries.len() > CAP {
        eprintln!(
            "warn: search index has {} entries; capping at {CAP}",
            entries.len()
        );
        let mut keep: Vec<SearchEntry> = entries
            .iter()
            .filter(|e| e.kind == "view")
            .cloned()
            .collect();
        // Plan deviation: the plan specified "keep all modules" unconditionally, but a codebase with many
        // unique 2-segment prefixes can produce >CAP module entries on its own (e.g. 21k flat files →
        // 21k unique module entries). Reserve 75% of remaining capacity for ranked files/symbols and
        // truncate modules to the first 25% (in sorted order, for determinism).
        let room_for_modules = CAP.saturating_sub(keep.len()) / 4;
        let mut mods: Vec<SearchEntry> = entries
            .iter()
            .filter(|e| e.kind == "module")
            .cloned()
            .collect();
        mods.truncate(room_for_modules);
        keep.extend(mods);
        let mut ranked: Vec<(f64, SearchEntry)> = entries
            .into_iter()
            .filter(|e| e.kind == "file" || e.kind == "symbol")
            .map(|e| {
                let file_path = if e.kind == "symbol" {
                    &e.context
                } else {
                    &e.label
                };
                let pr = index
                    .pagerank
                    .get(file_path.as_str())
                    .copied()
                    .unwrap_or(0.0);
                (pr, e)
            })
            .collect();
        ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let room = CAP.saturating_sub(keep.len());
        keep.extend(ranked.into_iter().take(room).map(|(_, e)| e));
        keep.sort_by(|a, b| (&a.kind, &a.label, &a.context).cmp(&(&b.kind, &b.label, &b.context)));
        return keep;
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    fn make_test_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let files = vec![
            ScannedFile {
                relative_path: "src/main.rs".into(),
                absolute_path: "/tmp/src/main.rs".into(),
                language: Some("rust".into()),
                size_bytes: 100,
            },
            ScannedFile {
                relative_path: "src/lib.rs".into(),
                absolute_path: "/tmp/src/lib.rs".into(),
                language: Some("rust".into()),
                size_bytes: 200,
            },
        ];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/main.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "main".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn main()".into(),
                    body: "fn main() {}".into(),
                    start_line: 1,
                    end_line: 3,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        parse_results.insert(
            "src/lib.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "private_helper".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "fn private_helper()".into(),
                    body: "".into(),
                    start_line: 1,
                    end_line: 2,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        let mut content = HashMap::new();
        content.insert("src/main.rs".into(), "fn main() {}".into());
        content.insert("src/lib.rs".into(), "fn private_helper() {}".into());
        CodebaseIndex::build_with_content(files, parse_results, &counter, content)
    }

    #[test]
    fn includes_all_six_views() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        assert_eq!(entries.iter().filter(|e| e.kind == "view").count(), 6);
    }

    #[test]
    fn includes_every_file() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        let file_labels: Vec<&str> = entries
            .iter()
            .filter(|e| e.kind == "file")
            .map(|e| e.label.as_str())
            .collect();
        assert!(file_labels.contains(&"src/main.rs"));
        assert!(file_labels.contains(&"src/lib.rs"));
    }

    #[test]
    fn includes_only_public_symbols() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        let sym_labels: Vec<&str> = entries
            .iter()
            .filter(|e| e.kind == "symbol")
            .map(|e| e.label.as_str())
            .collect();
        assert!(sym_labels.contains(&"main"));
        assert!(
            !sym_labels.contains(&"private_helper"),
            "private symbols must be excluded"
        );
    }

    #[test]
    fn filenames_are_not_module_entries() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        let mods: Vec<&str> = entries
            .iter()
            .filter(|e| e.kind == "module")
            .map(|e| e.label.as_str())
            .collect();
        assert!(
            !mods.contains(&"src/main.rs"),
            "filename src/main.rs must not appear as a module; modules were {mods:?}"
        );
        assert!(
            !mods.contains(&"src/lib.rs"),
            "filename src/lib.rs must not appear as a module; modules were {mods:?}"
        );
    }

    #[test]
    fn includes_module_prefixes() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        let mods: Vec<&str> = entries
            .iter()
            .filter(|e| e.kind == "module")
            .map(|e| e.label.as_str())
            .collect();
        assert!(
            mods.contains(&"src"),
            "first-segment module should appear: got {mods:?}"
        );
    }

    #[test]
    fn sorted_by_kind_label_context() {
        let index = make_test_index();
        let entries = build_search_index(&index);
        for w in entries.windows(2) {
            let a = (&w[0].kind, &w[0].label, &w[0].context);
            let b = (&w[1].kind, &w[1].label, &w[1].context);
            assert!(
                a <= b,
                "entries must be sorted by (kind, label, context): {a:?} !<= {b:?}"
            );
        }
    }

    #[test]
    fn is_deterministic() {
        let index = make_test_index();
        let a = build_search_index(&index);
        let b = build_search_index(&index);
        let a_json = serde_json::to_string(&a).unwrap();
        let b_json = serde_json::to_string(&b).unwrap();
        assert_eq!(a_json, b_json);
    }

    #[test]
    fn parse_failed_file_marker() {
        let counter = TokenCounter::new();
        let files = vec![ScannedFile {
            relative_path: "src/broken.rs".into(),
            absolute_path: "/tmp/src/broken.rs".into(),
            language: Some("rust".into()),
            size_bytes: 10,
        }];
        let parse_results = HashMap::new(); // no entry = parse_result remains None
        let mut content = HashMap::new();
        content.insert("src/broken.rs".into(), "invalid rust code".into());
        let index = CodebaseIndex::build_with_content(files, parse_results, &counter, content);
        let entries = build_search_index(&index);
        let entry = entries
            .iter()
            .find(|e| e.label == "src/broken.rs")
            .expect("parse-failed file must still appear");
        assert_eq!(entry.kind, "file");
        assert!(
            entry.detail.contains("parse error"),
            "detail must mark parse error: {}",
            entry.detail
        );
    }

    #[test]
    fn caps_at_20000_entries() {
        // Synthesize an index with >20000 files by replicating.
        let counter = TokenCounter::new();
        let files: Vec<ScannedFile> = (0..21000)
            .map(|i| ScannedFile {
                relative_path: format!("src/mod_{i:05}.rs"),
                absolute_path: std::path::PathBuf::from(format!("/tmp/src/mod_{i:05}.rs")),
                language: Some("rust".into()),
                size_bytes: 10,
            })
            .collect();
        let mut content = HashMap::new();
        for f in &files {
            content.insert(f.relative_path.clone(), "// empty".into());
        }
        let index = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content);
        let entries = build_search_index(&index);
        assert!(
            entries.len() <= 20_000,
            "search index must cap at 20,000 entries, got {}",
            entries.len()
        );
    }
}
