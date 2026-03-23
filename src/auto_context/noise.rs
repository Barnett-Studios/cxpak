use serde::Serialize;
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Task 2: Blocklist noise filter
// ---------------------------------------------------------------------------

const NOISE_PATH_PATTERNS: &[&str] = &[
    "vendor/",
    "node_modules/",
    "third_party/",
    "external/",
    "dist/",
    "build/",
    "target/",
    ".next/",
    "__pycache__/",
    "out/",
    ".min.js",
    ".min.css",
    ".generated.",
    "_generated.",
    ".gen.",
    "_pb.go",
    "_pb2.py",
    ".pb.cc",
    ".pb.h",
    ".map",
];

const NOISE_EXACT_FILENAMES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "Cargo.lock",
    "pnpm-lock.yaml",
    "poetry.lock",
    "Gemfile.lock",
    "composer.lock",
];

const GENERATED_MARKERS: &[&str] = &[
    "// Code generated",
    "# AUTO-GENERATED",
    "/* DO NOT EDIT */",
    "// DO NOT EDIT",
    "@generated",
    "# This file is auto-generated",
];

/// Returns `true` if the given path belongs to a noise category that should
/// be excluded regardless of relevance score.
///
/// Lock files are matched by exact filename only — e.g. `deadlock.rs` will
/// NOT be excluded because it happens to contain "lock".  All other patterns
/// perform a substring match on the full path string.
pub fn is_blocklisted(path: &str) -> bool {
    let filename = path.rsplit('/').next().unwrap_or(path);
    if NOISE_EXACT_FILENAMES.contains(&filename) {
        return true;
    }
    NOISE_PATH_PATTERNS.iter().any(|p| path.contains(p))
}

/// Returns `true` if the first 5 lines of `content` contain one of the
/// known generated-file markers.
pub fn has_generated_marker(content: &str) -> bool {
    let header: String = content.lines().take(5).collect::<Vec<_>>().join("\n");
    GENERATED_MARKERS.iter().any(|m| header.contains(m))
}

// ---------------------------------------------------------------------------
// Task 3: Similarity dedup
// ---------------------------------------------------------------------------

/// Computes the Jaccard similarity between two symbol sets represented as
/// `HashSet<&str>`.  Returns `0.0` when both sets are empty.
pub fn jaccard_symbol_similarity(symbols_a: &HashSet<&str>, symbols_b: &HashSet<&str>) -> f64 {
    if symbols_a.is_empty() && symbols_b.is_empty() {
        return 0.0;
    }
    let intersection = symbols_a.intersection(symbols_b).count();
    let union = symbols_a.union(symbols_b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Deduplicates a list of scored file entries by symbol similarity.
///
/// For each pair where Jaccard similarity exceeds 0.80 the file with the
/// lower PageRank score is removed.  Iteration order of `entries` is
/// preserved for the survivors.
///
/// `symbols_by_path` maps a file's path to the set of symbol names it
/// exports/defines.  `pagerank` maps a file's path to its PageRank value.
pub fn dedup_similar_files(
    entries: Vec<ScoredFileEntry>,
    symbols_by_path: &HashMap<String, HashSet<String>>,
    pagerank: &HashMap<String, f64>,
) -> (Vec<ScoredFileEntry>, Vec<(String, String)>) {
    // Track which indices have been removed and which path each survivor
    // maps to for the filtered-out reason.
    let n = entries.len();
    let mut removed: Vec<bool> = vec![false; n];
    // (filtered_path, kept_path)
    let mut filter_reasons: Vec<(String, String)> = Vec::new();

    // Build borrowed symbol sets once to avoid repeated HashMap look-ups.
    let empty: HashSet<String> = HashSet::new();
    let sym_sets: Vec<HashSet<&str>> = entries
        .iter()
        .map(|e| {
            symbols_by_path
                .get(&e.path)
                .unwrap_or(&empty)
                .iter()
                .map(|s| s.as_str())
                .collect()
        })
        .collect();

    for i in 0..n {
        if removed[i] {
            continue;
        }
        for j in (i + 1)..n {
            if removed[j] {
                continue;
            }
            let sim = jaccard_symbol_similarity(&sym_sets[i], &sym_sets[j]);
            if sim > 0.80 {
                let pr_i = *pagerank.get(&entries[i].path).unwrap_or(&0.0);
                let pr_j = *pagerank.get(&entries[j].path).unwrap_or(&0.0);
                if pr_i >= pr_j {
                    filter_reasons.push((entries[j].path.clone(), entries[i].path.clone()));
                    removed[j] = true;
                } else {
                    filter_reasons.push((entries[i].path.clone(), entries[j].path.clone()));
                    removed[i] = true;
                    break; // entry i is gone; move to the next i
                }
            }
        }
    }

    let kept = entries
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| !removed[*idx])
        .map(|(_, e)| e)
        .collect();

    (kept, filter_reasons)
}

// ---------------------------------------------------------------------------
// Task 4: Relevance floor + orchestrator
// ---------------------------------------------------------------------------

/// Files scoring below this threshold are excluded from the context window.
pub const DEFAULT_RELEVANCE_FLOOR: f64 = 0.15;

/// A file together with its relevance score and token budget cost.
#[derive(Debug, Clone)]
pub struct ScoredFileEntry {
    pub path: String,
    pub score: f64,
    pub token_count: usize,
}

/// A file that was excluded by the noise filter, along with the reason.
#[derive(Debug, Clone, Serialize)]
pub struct FilteredFile {
    pub path: String,
    pub reason: String,
}

/// The result of running `filter_noise()`.
pub struct NoiseFilterResult {
    pub kept: Vec<ScoredFileEntry>,
    pub filtered_out: Vec<FilteredFile>,
}

/// Orchestrates all three noise-filtering layers in order:
///
/// 1. **Blocklist / generated markers** — removes vendor directories, lock
///    files, minified assets, and auto-generated files.
/// 2. **Similarity dedup** — removes the lower-PageRank duplicate when two
///    files share >80 % of their symbol vocabulary.
/// 3. **Relevance floor** — removes files whose score is below
///    [`DEFAULT_RELEVANCE_FLOOR`].
pub fn filter_noise(
    candidates: Vec<ScoredFileEntry>,
    index: &crate::index::CodebaseIndex,
    pagerank: &HashMap<String, f64>,
) -> NoiseFilterResult {
    let mut kept: Vec<ScoredFileEntry> = Vec::new();
    let mut filtered_out: Vec<FilteredFile> = Vec::new();

    // --- Layer 1: blocklist + generated markers ---
    let mut after_blocklist: Vec<ScoredFileEntry> = Vec::new();
    for entry in candidates {
        // Check path-based blocklist first (cheap).
        if is_blocklisted(&entry.path) {
            // Find which pattern triggered the match for a descriptive reason.
            let filename = entry.path.rsplit('/').next().unwrap_or(&entry.path);
            let reason = if NOISE_EXACT_FILENAMES.contains(&filename) {
                format!("blocklist: {filename}")
            } else {
                let pattern = NOISE_PATH_PATTERNS
                    .iter()
                    .find(|p| entry.path.contains(**p))
                    .copied()
                    .unwrap_or("unknown");
                format!("blocklist: {pattern}")
            };
            filtered_out.push(FilteredFile {
                path: entry.path,
                reason,
            });
            continue;
        }

        // Check generated-marker in file content.
        if let Some(file) = index.files.iter().find(|f| f.relative_path == entry.path) {
            if has_generated_marker(&file.content) {
                filtered_out.push(FilteredFile {
                    path: entry.path,
                    reason: "generated_file".to_string(),
                });
                continue;
            }
        }

        after_blocklist.push(entry);
    }

    // --- Layer 2: similarity dedup ---
    // Build a symbol map from the index for the surviving files.
    let symbols_by_path: HashMap<String, HashSet<String>> = after_blocklist
        .iter()
        .map(|e| {
            let syms: HashSet<String> = index
                .files
                .iter()
                .find(|f| f.relative_path == e.path)
                .and_then(|f| f.parse_result.as_ref())
                .map(|pr| pr.symbols.iter().map(|s| s.name.clone()).collect())
                .unwrap_or_default();
            (e.path.clone(), syms)
        })
        .collect();

    let (after_dedup, dedup_reasons) =
        dedup_similar_files(after_blocklist, &symbols_by_path, pagerank);

    for (filtered_path, kept_path) in dedup_reasons {
        filtered_out.push(FilteredFile {
            path: filtered_path,
            reason: format!("similar_to: {kept_path}"),
        });
    }

    // --- Layer 3: relevance floor ---
    for entry in after_dedup {
        if entry.score < DEFAULT_RELEVANCE_FLOOR {
            filtered_out.push(FilteredFile {
                path: entry.path,
                reason: format!("below_relevance_floor: {:.4}", entry.score),
            });
        } else {
            kept.push(entry);
        }
    }

    NoiseFilterResult { kept, filtered_out }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    // -----------------------------------------------------------------------
    // Task 2 tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_blocklist_vendor() {
        assert!(is_blocklisted("vendor/some/lib.js"));
        assert!(is_blocklisted("node_modules/react/index.js"));
        assert!(is_blocklisted("third_party/openssl/ssl.h"));
    }

    #[test]
    fn test_blocklist_build_output() {
        assert!(is_blocklisted("dist/bundle.js"));
        assert!(is_blocklisted("build/output.js"));
        assert!(is_blocklisted("target/debug/binary"));
        assert!(is_blocklisted("src/__pycache__/module.pyc"));
        assert!(is_blocklisted("out/main.class"));
    }

    #[test]
    fn test_blocklist_minified() {
        assert!(is_blocklisted("static/app.min.js"));
        assert!(is_blocklisted("public/styles.min.css"));
    }

    #[test]
    fn test_blocklist_generated() {
        assert!(is_blocklisted("src/schema.generated.ts"));
        assert!(is_blocklisted("src/_generated.types.ts"));
        assert!(is_blocklisted("proto/service.gen.go"));
        assert!(is_blocklisted("proto/service_pb.go"));
        assert!(is_blocklisted("proto/service_pb2.py"));
        assert!(is_blocklisted("proto/service.pb.cc"));
        assert!(is_blocklisted("proto/service.pb.h"));
    }

    #[test]
    fn test_blocklist_lock_files() {
        // Exact filename matches
        assert!(is_blocklisted("package-lock.json"));
        assert!(is_blocklisted("yarn.lock"));
        assert!(is_blocklisted("Cargo.lock"));
        assert!(is_blocklisted("pnpm-lock.yaml"));
        assert!(is_blocklisted("poetry.lock"));
        assert!(is_blocklisted("Gemfile.lock"));
        assert!(is_blocklisted("composer.lock"));

        // Lock files in subdirectories (still exact on filename component)
        assert!(is_blocklisted("frontend/package-lock.json"));
        assert!(is_blocklisted("packages/server/Cargo.lock"));
    }

    #[test]
    fn test_blocklist_source_maps() {
        assert!(is_blocklisted("dist/app.js.map"));
        assert!(is_blocklisted("public/bundle.css.map"));
    }

    #[test]
    fn test_not_blocklisted() {
        // Normal source files must never be filtered.
        assert!(!is_blocklisted("src/api/handler.rs"));
        // "deadlock" contains "lock" but the filename is not exactly a lock file.
        assert!(!is_blocklisted("src/deadlock.rs"));
        // "file_lock" similarly.
        assert!(!is_blocklisted("src/file_lock.py"));
        // "build" as part of an identifier in a path segment, not a directory.
        assert!(!is_blocklisted("src/builder.rs"));
    }

    #[test]
    fn test_generated_marker_detection() {
        // Markers in first 5 lines → detected.
        assert!(has_generated_marker(
            "// Code generated by protoc\npackage foo"
        ));
        assert!(has_generated_marker("# AUTO-GENERATED\n# do not touch"));
        assert!(has_generated_marker("/* DO NOT EDIT */\n.class {}"));
        assert!(has_generated_marker("// DO NOT EDIT\nmod generated;"));
        assert!(has_generated_marker("@generated\npublic class Foo {}"));
        assert!(has_generated_marker(
            "# This file is auto-generated\n# by codegen"
        ));

        // Marker appearing AFTER line 5 must NOT be detected.
        assert!(!has_generated_marker(
            "line1\nline2\nline3\nline4\nline5\nline6\n// Code generated"
        ));

        // Normal file with no markers.
        assert!(!has_generated_marker(
            "pub fn main() {\n    println!(\"hello\");\n}"
        ));
    }

    // -----------------------------------------------------------------------
    // Task 3 tests
    // -----------------------------------------------------------------------

    fn set<'a>(items: &'a [&'a str]) -> HashSet<&'a str> {
        items.iter().copied().collect()
    }

    #[test]
    fn test_jaccard_identical() {
        let a = set(&["foo", "bar", "baz"]);
        let b = set(&["foo", "bar", "baz"]);
        let sim = jaccard_symbol_similarity(&a, &b);
        assert!(
            (sim - 1.0).abs() < f64::EPSILON,
            "identical sets should have similarity 1.0, got {sim}"
        );
    }

    #[test]
    fn test_jaccard_90_percent() {
        // 9 shared out of 10 unique → 0.9 > 0.80 → should be filtered.
        let a = set(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "x"]);
        let b = set(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "y"]);
        let sim = jaccard_symbol_similarity(&a, &b);
        assert!(
            sim > 0.80,
            "90 % overlap should be above threshold, got {sim}"
        );
    }

    #[test]
    fn test_jaccard_50_percent() {
        // 2 shared out of 4 unique → 0.5 < 0.80 → should be kept.
        let a = set(&["a", "b"]);
        let b = set(&["a", "c"]);
        let sim = jaccard_symbol_similarity(&a, &b);
        assert!(
            sim < 0.80,
            "50 % overlap should be below threshold, got {sim}"
        );
    }

    #[test]
    fn test_jaccard_no_overlap() {
        let a = set(&["a", "b"]);
        let b = set(&["c", "d"]);
        let sim = jaccard_symbol_similarity(&a, &b);
        assert!(
            sim.abs() < f64::EPSILON,
            "no overlap should give 0.0, got {sim}"
        );
    }

    #[test]
    fn test_dedup_keeps_higher_pagerank() {
        let entries = vec![
            ScoredFileEntry {
                path: "src/low_rank.rs".to_string(),
                score: 0.5,
                token_count: 100,
            },
            ScoredFileEntry {
                path: "src/high_rank.rs".to_string(),
                score: 0.5,
                token_count: 100,
            },
        ];

        // Both files share the same symbols → Jaccard = 1.0 > 0.80.
        let mut symbols_by_path: HashMap<String, HashSet<String>> = HashMap::new();
        let shared: HashSet<String> = ["foo", "bar", "baz"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        symbols_by_path.insert("src/low_rank.rs".to_string(), shared.clone());
        symbols_by_path.insert("src/high_rank.rs".to_string(), shared);

        let mut pagerank: HashMap<String, f64> = HashMap::new();
        pagerank.insert("src/low_rank.rs".to_string(), 0.1);
        pagerank.insert("src/high_rank.rs".to_string(), 0.9);

        let (kept, reasons) = dedup_similar_files(entries, &symbols_by_path, &pagerank);

        assert_eq!(kept.len(), 1, "one file should survive");
        assert_eq!(
            kept[0].path, "src/high_rank.rs",
            "higher PageRank file should be kept"
        );
        assert_eq!(reasons.len(), 1);
        assert_eq!(reasons[0].0, "src/low_rank.rs");
        assert_eq!(reasons[0].1, "src/high_rank.rs");
    }

    // -----------------------------------------------------------------------
    // Task 4 tests
    // -----------------------------------------------------------------------

    fn make_minimal_index(paths: &[(&str, &str)]) -> crate::index::CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let files: Vec<ScannedFile> = paths
            .iter()
            .map(|(rel, content)| {
                let abs = dir.path().join(rel.replace('/', "_"));
                std::fs::write(&abs, content).unwrap();
                ScannedFile {
                    relative_path: rel.to_string(),
                    absolute_path: abs,
                    language: Some("rust".into()),
                    size_bytes: content.len() as u64,
                }
            })
            .collect();

        crate::index::CodebaseIndex::build(files, HashMap::new(), &counter)
    }

    #[test]
    fn test_relevance_floor_excludes_low() {
        let index = make_minimal_index(&[("src/low.rs", "fn low() {}")]);
        let pagerank = HashMap::new();
        let candidates = vec![ScoredFileEntry {
            path: "src/low.rs".to_string(),
            score: 0.05,
            token_count: 10,
        }];
        let result = filter_noise(candidates, &index, &pagerank);
        assert!(result.kept.is_empty(), "score 0.05 should be excluded");
        assert_eq!(result.filtered_out.len(), 1);
        assert!(result.filtered_out[0]
            .reason
            .starts_with("below_relevance_floor"));
    }

    #[test]
    fn test_relevance_floor_boundary() {
        let index = make_minimal_index(&[("src/boundary.rs", "fn boundary() {}")]);
        let pagerank = HashMap::new();
        let candidates = vec![ScoredFileEntry {
            path: "src/boundary.rs".to_string(),
            score: 0.15,
            token_count: 10,
        }];
        let result = filter_noise(candidates, &index, &pagerank);
        assert_eq!(result.kept.len(), 1, "score 0.15 (= floor) should be kept");
        assert!(result.filtered_out.is_empty());
    }

    #[test]
    fn test_relevance_floor_high() {
        let index = make_minimal_index(&[("src/high.rs", "fn high() {}")]);
        let pagerank = HashMap::new();
        let candidates = vec![ScoredFileEntry {
            path: "src/high.rs".to_string(),
            score: 0.50,
            token_count: 10,
        }];
        let result = filter_noise(candidates, &index, &pagerank);
        assert_eq!(result.kept.len(), 1, "score 0.50 should be kept");
        assert!(result.filtered_out.is_empty());
    }

    #[test]
    fn test_filter_orchestrator() {
        // File 1: blocklisted (vendor directory)
        // File 2: generated marker in content
        // File 3: good score, survives all layers
        // File 4: below floor, excluded by layer 3
        let index = make_minimal_index(&[
            ("vendor/lib.rs", "fn vendored() {}"),
            ("src/generated.rs", "// Code generated by tool\nfn gen() {}"),
            ("src/real.rs", "fn real() {}"),
            ("src/irrelevant.rs", "fn irrelevant() {}"),
        ]);
        let pagerank: HashMap<String, f64> = HashMap::new();

        let candidates = vec![
            ScoredFileEntry {
                path: "vendor/lib.rs".to_string(),
                score: 0.9,
                token_count: 10,
            },
            ScoredFileEntry {
                path: "src/generated.rs".to_string(),
                score: 0.8,
                token_count: 10,
            },
            ScoredFileEntry {
                path: "src/real.rs".to_string(),
                score: 0.7,
                token_count: 10,
            },
            ScoredFileEntry {
                path: "src/irrelevant.rs".to_string(),
                score: 0.05,
                token_count: 10,
            },
        ];

        let result = filter_noise(candidates, &index, &pagerank);

        assert_eq!(result.kept.len(), 1);
        assert_eq!(result.kept[0].path, "src/real.rs");

        assert_eq!(result.filtered_out.len(), 3);

        let reasons: HashMap<&str, &str> = result
            .filtered_out
            .iter()
            .map(|f| (f.path.as_str(), f.reason.as_str()))
            .collect();

        assert!(
            reasons["vendor/lib.rs"].starts_with("blocklist:"),
            "expected blocklist reason, got: {}",
            reasons["vendor/lib.rs"]
        );
        assert_eq!(
            reasons["src/generated.rs"], "generated_file",
            "expected generated_file reason"
        );
        assert!(
            reasons["src/irrelevant.rs"].starts_with("below_relevance_floor"),
            "expected floor reason, got: {}",
            reasons["src/irrelevant.rs"]
        );
    }

    #[test]
    fn test_filter_preserves_order() {
        // The order of `kept` entries must follow the original input order.
        let index = make_minimal_index(&[
            ("src/a.rs", "fn a() {}"),
            ("src/b.rs", "fn b() {}"),
            ("src/c.rs", "fn c() {}"),
        ]);
        let pagerank: HashMap<String, f64> = HashMap::new();

        let candidates = vec![
            ScoredFileEntry {
                path: "src/c.rs".to_string(),
                score: 0.5,
                token_count: 10,
            },
            ScoredFileEntry {
                path: "src/a.rs".to_string(),
                score: 0.6,
                token_count: 10,
            },
            ScoredFileEntry {
                path: "src/b.rs".to_string(),
                score: 0.4,
                token_count: 10,
            },
        ];

        let result = filter_noise(candidates, &index, &pagerank);

        assert_eq!(result.kept.len(), 3);
        assert_eq!(result.kept[0].path, "src/c.rs");
        assert_eq!(result.kept[1].path, "src/a.rs");
        assert_eq!(result.kept[2].path, "src/b.rs");
    }
}
