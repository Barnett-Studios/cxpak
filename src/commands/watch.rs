use crate::budget::counter::TokenCounter;
use crate::cli::OutputFormat;
use crate::daemon::watcher::{FileChange, FileWatcher};
use crate::index::CodebaseIndex;
use crate::parser::LanguageRegistry;
use crate::scanner::Scanner;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

pub fn run(
    path: &Path,
    _token_budget: usize,
    _format: &OutputFormat,
    _verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let counter = TokenCounter::new();
    let registry = LanguageRegistry::new();

    // Initial full build
    let scanner = Scanner::new(path)?;
    let files = scanner.scan()?;

    // Parse all files
    let mut parse_results = HashMap::new();
    let mut content_map = HashMap::new();
    for file in &files {
        let source = std::fs::read_to_string(&file.absolute_path).unwrap_or_default();
        if let Some(lang_name) = &file.language {
            if let Some(lang) = registry.get(lang_name) {
                let ts_lang = lang.ts_language();
                let mut parser = tree_sitter::Parser::new();
                parser.set_language(&ts_lang).ok();
                if let Some(tree) = parser.parse(&source, None) {
                    let result = lang.extract(&source, &tree);
                    parse_results.insert(file.relative_path.clone(), result);
                }
            }
        }
        content_map.insert(file.relative_path.clone(), source);
    }

    let mut index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);

    eprintln!(
        "cxpak: watching {} ({} files indexed, {} tokens)",
        path.display(),
        index.total_files,
        index.total_tokens
    );

    // Start watching
    let watcher = FileWatcher::new(path)?;

    loop {
        // Block until a change comes in, then drain all pending
        if let Some(first) = watcher.recv_timeout(Duration::from_secs(1)) {
            let mut changes = vec![first];
            // Small debounce: wait a bit then drain
            std::thread::sleep(Duration::from_millis(50));
            changes.extend(watcher.drain());

            let mut modified_paths = std::collections::HashSet::new();
            let mut removed_paths = std::collections::HashSet::new();

            for change in changes {
                match change {
                    FileChange::Created(p) | FileChange::Modified(p) => {
                        if let Ok(rel) = p.strip_prefix(path) {
                            modified_paths.insert(rel.to_string_lossy().to_string());
                        }
                    }
                    FileChange::Removed(p) => {
                        if let Ok(rel) = p.strip_prefix(path) {
                            removed_paths.insert(rel.to_string_lossy().to_string());
                        }
                    }
                }
            }

            let start = std::time::Instant::now();
            let mut update_count = 0;

            for rel_path in &removed_paths {
                index.remove_file(rel_path);
                update_count += 1;
            }

            for rel_path in &modified_paths {
                if removed_paths.contains(rel_path) {
                    continue;
                }
                let abs_path = path.join(rel_path);
                if let Ok(content) = std::fs::read_to_string(&abs_path) {
                    let lang_name = crate::scanner::detect_language(Path::new(rel_path));
                    let parse_result = lang_name.as_deref().and_then(|ln| {
                        registry.get(ln).and_then(|lang| {
                            let ts_lang = lang.ts_language();
                            let mut parser = tree_sitter::Parser::new();
                            parser.set_language(&ts_lang).ok()?;
                            let tree = parser.parse(&content, None)?;
                            Some(lang.extract(&content, &tree))
                        })
                    });

                    index.upsert_file(
                        rel_path,
                        lang_name.as_deref(),
                        &content,
                        parse_result,
                        &counter,
                    );
                    update_count += 1;
                }
            }

            if update_count > 0 {
                eprintln!(
                    "cxpak: updated {} file(s) ({:.0?}), {} files / {} tokens total",
                    update_count,
                    start.elapsed(),
                    index.total_files,
                    index.total_tokens
                );
            }
        }
    }
}
