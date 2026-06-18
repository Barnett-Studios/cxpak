//! Indexing benchmarks (W1, ADR-0165/0166/0167).
//!
//! Lead metric: **per-edit incremental_rebuild latency (p50/p99)** — the number
//! that decides whether `serve`/`watch`/LSP feel instant under edits. We print
//! the explicit p50/p99 distribution (criterion reports mean/median but not p99
//! in its text output), then register criterion benches for regression tracking:
//! cold build, warm derived-cache build, and a single incremental edit.

use criterion::{criterion_group, criterion_main, Criterion};
use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use cxpak::parser::language::{Import, ParseResult};
use cxpak::scanner::ScannedFile;
use std::collections::HashMap;
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

/// Generate a synthetic repo of `n` Rust files under `dir`, each importing the
/// previous one (a dependency chain → non-trivial graph + PageRank). Returns the
/// scanned files and their parse results.
fn synth_repo(dir: &Path, n: usize) -> (Vec<ScannedFile>, HashMap<String, ParseResult>) {
    let mut files = Vec::with_capacity(n);
    let mut parses = HashMap::with_capacity(n);
    for i in 0..n {
        let rel = format!("src/f{i}.rs");
        let abs = dir.join(format!("f{i}.rs"));
        let content = format!("// file {i}\npub fn f{i}() {{}}\n");
        std::fs::write(&abs, &content).unwrap();
        let imports = if i > 0 {
            vec![Import {
                source: format!("crate::f{}", i - 1),
                names: vec![],
            }]
        } else {
            vec![]
        };
        files.push(ScannedFile {
            relative_path: rel.clone(),
            absolute_path: abs,
            language: Some("rust".to_string()),
            size_bytes: content.len() as u64,
        });
        parses.insert(
            rel,
            ParseResult {
                symbols: vec![],
                imports,
                exports: vec![],
            },
        );
    }
    (files, parses)
}

/// Print the explicit per-edit p50/p99 for `incremental_rebuild` at scale `n`.
/// Each edit changes one file's size so `needs_update` fires, then re-indexes.
fn report_per_edit_quantiles(n: usize, edits: usize) {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let (files, parses) = synth_repo(dir.path(), n);
    let mut index = CodebaseIndex::build(files.clone(), parses.clone(), &counter);

    let target = "src/f0.rs";
    let target_abs = dir.path().join("f0.rs");
    let mut durations: Vec<u128> = Vec::with_capacity(edits);
    for e in 0..edits {
        // Vary the file size every iteration so the change is always detected.
        let body = "/".repeat(e % 200 + 1);
        std::fs::write(&target_abs, format!("pub fn f0() {{}}\n{body}")).unwrap();
        let mut current = files.clone();
        if let Some(f) = current.iter_mut().find(|f| f.relative_path == target) {
            f.size_bytes = std::fs::metadata(&target_abs).unwrap().len();
        }
        let start = Instant::now();
        index.incremental_rebuild(&current, &parses, &counter);
        durations.push(start.elapsed().as_micros());
    }
    durations.sort_unstable();
    let pct = |p: f64| durations[((durations.len() as f64 * p) as usize).min(durations.len() - 1)];
    eprintln!(
        "[per-edit incremental_rebuild @ {n} files, {edits} edits] p50={}us p99={}us max={}us",
        pct(0.50),
        pct(0.99),
        durations[durations.len() - 1],
    );
}

fn bench_indexing(c: &mut Criterion) {
    // Explicit headline distribution (printed once, not part of criterion stats).
    report_per_edit_quantiles(1000, 300);

    let counter = TokenCounter::new();

    // Cold full build at 1k files.
    {
        let dir = tempfile::TempDir::new().unwrap();
        let (files, parses) = synth_repo(dir.path(), 1000);
        c.bench_function("cold_build_1k", |b| {
            b.iter(|| {
                let idx = CodebaseIndex::build(files.clone(), parses.clone(), &counter);
                black_box(idx.total_files)
            })
        });
    }

    // Single incremental edit at 1k files (the live-edit primitive).
    {
        let dir = tempfile::TempDir::new().unwrap();
        let (files, parses) = synth_repo(dir.path(), 1000);
        let base = CodebaseIndex::build(files.clone(), parses.clone(), &counter);
        let target_abs = dir.path().join("f0.rs");
        let edit = std::cell::Cell::new(0usize);
        c.bench_function("incremental_edit_1k", |b| {
            b.iter(|| {
                let e = edit.get();
                edit.set(e + 1);
                let body = "/".repeat(e % 200 + 1);
                std::fs::write(&target_abs, format!("pub fn f0() {{}}\n{body}")).unwrap();
                let mut files = files.clone();
                if let Some(f) = files.iter_mut().find(|f| f.relative_path == "src/f0.rs") {
                    f.size_bytes = std::fs::metadata(&target_abs).unwrap().len();
                }
                let mut idx = base.clone();
                idx.incremental_rebuild(&files, &parses, &counter);
                black_box(idx.graph.edge_count())
            })
        });
    }
}

criterion_group!(benches, bench_indexing);
criterion_main!(benches);
