use crate::index::CodebaseIndex;
use crate::relevance::SignalResult;
use std::collections::{HashMap, HashSet};

/// Tokenize a string into lowercase parts split on non-alphanumeric chars and underscores.
/// Also includes the lowercased whole word to handle all-caps identifiers (e.g., "API" -> "api").
pub fn tokenize(s: &str) -> HashSet<String> {
    let mut tokens = HashSet::new();
    for word in s.split(|c: char| !c.is_alphanumeric() && c != '_') {
        let parts: Vec<String> = crate::index::split_identifier(word)
            .into_iter()
            .filter(|p| p.len() >= 2)
            .collect();
        if parts.is_empty() {
            // No split parts — keep the whole word (e.g. "API" -> "api")
            let lower = word.to_lowercase();
            if lower.len() >= 2 {
                tokens.insert(lower);
            }
        } else {
            tokens.extend(parts);
        }
    }
    tokens
}

/// PathSimilarity: Tokenize query + file path segments, compute Jaccard similarity.
pub fn path_similarity(query: &str, file_path: &str) -> SignalResult {
    let query_tokens = tokenize(query);
    // For the path, also split on '/' and '.'
    let path_tokens: HashSet<String> = file_path
        .split(['/', '.', '_', '-'])
        .flat_map(crate::index::split_identifier)
        .filter(|t| t.len() >= 2)
        .collect();

    if query_tokens.is_empty() || path_tokens.is_empty() {
        return SignalResult {
            name: "path_similarity",
            score: 0.0,
            detail: "empty tokens".to_string(),
        };
    }

    let intersection = query_tokens.intersection(&path_tokens).count();
    // Blend query coverage (recall) and path coverage (precision) with
    // heavier weight on query coverage so that matching all query terms
    // scores high even when the path has many extra segments.
    let query_coverage = intersection as f64 / query_tokens.len() as f64;
    let path_coverage = intersection as f64 / path_tokens.len() as f64;
    let score = 0.7 * query_coverage + 0.3 * path_coverage;

    SignalResult {
        name: "path_similarity",
        score,
        detail: format!(
            "score={:.2}, qcov={:.2}, pcov={:.2}",
            score, query_coverage, path_coverage
        ),
    }
}

/// SymbolMatch: Fuzzy match query terms against function/struct/class names in file.
pub fn symbol_match(
    query: &str,
    file_path: &str,
    index: &CodebaseIndex,
    expanded_tokens: Option<&HashSet<String>>,
) -> SignalResult {
    let file = match index.files.iter().find(|f| f.relative_path == file_path) {
        Some(f) => f,
        None => {
            return SignalResult {
                name: "symbol_match",
                score: 0.0,
                detail: "file not found".to_string(),
            }
        }
    };

    let symbols = match &file.parse_result {
        Some(pr) => &pr.symbols,
        None => {
            return SignalResult {
                name: "symbol_match",
                score: 0.0,
                detail: "no parse result".to_string(),
            }
        }
    };

    if symbols.is_empty() {
        return SignalResult {
            name: "symbol_match",
            score: 0.0,
            detail: "no symbols".to_string(),
        };
    }

    let owned_tokens;
    let query_tokens = match expanded_tokens {
        Some(tokens) => tokens,
        None => {
            owned_tokens = tokenize(query);
            &owned_tokens
        }
    };
    if query_tokens.is_empty() {
        return SignalResult {
            name: "symbol_match",
            score: 0.0,
            detail: "empty query".to_string(),
        };
    }

    let mut best_score = 0.0_f64;
    let mut best_symbol = String::new();

    for symbol in symbols {
        let symbol_tokens = tokenize(&symbol.name);
        if symbol_tokens.is_empty() {
            continue;
        }
        let intersection = query_tokens.intersection(&symbol_tokens).count();
        let union = query_tokens.union(&symbol_tokens).count();
        let score = if union > 0 {
            intersection as f64 / union as f64
        } else {
            0.0
        };
        if score > best_score {
            best_score = score;
            best_symbol = symbol.name.clone();
        }
    }

    SignalResult {
        name: "symbol_match",
        score: best_score,
        detail: if best_score > 0.0 {
            format!("matched: {} (score={:.2})", best_symbol, best_score)
        } else {
            "no matches".to_string()
        },
    }
}

/// ImportProximity: Boost if file imports or is imported by other files.
/// Returns 0.0 when no connections exist, scales to 1.0 at 10+ connections.
pub fn import_proximity(file_path: &str, index: &CodebaseIndex) -> SignalResult {
    let file = match index.files.iter().find(|f| f.relative_path == file_path) {
        Some(f) => f,
        None => {
            return SignalResult {
                name: "import_proximity",
                score: 0.5,
                detail: "file not found".to_string(),
            }
        }
    };

    // Count outgoing imports from this file
    let outgoing = file
        .parse_result
        .as_ref()
        .map(|pr| pr.imports.len())
        .unwrap_or(0);

    // Count incoming imports (other files importing things from this file)
    let incoming = index
        .files
        .iter()
        .filter(|f| f.relative_path != file_path)
        .filter(|f| {
            f.parse_result.as_ref().is_some_and(|pr| {
                pr.imports.iter().any(|imp| {
                    // Check if import source references this file's path.
                    // Split source into segments and match against the file stem
                    // to avoid false positives (e.g. "config" matching "reconfigure").
                    let path_stem = file_path
                        .rsplit('/')
                        .next()
                        .unwrap_or(file_path)
                        .split('.')
                        .next()
                        .unwrap_or("")
                        .to_lowercase();
                    if path_stem.len() < 2 {
                        return false;
                    }
                    let source_lower = imp.source.to_lowercase();
                    source_lower
                        .split([':', '/', '.', '\\'])
                        .any(|segment| segment == path_stem)
                })
            })
        })
        .count();

    let connections = outgoing + incoming;

    // Scale: 0.0 (no connections) to 1.0 (10+ connections), capped at 10.
    // Floor is 0.0 so disconnected files score lower than weakly-connected ones
    // — preserving full signal discrimination across the range.
    let score = connections.min(10) as f64 / 10.0;

    SignalResult {
        name: "import_proximity",
        score,
        detail: format!(
            "connections={} (out={}, in={})",
            connections, outgoing, incoming
        ),
    }
}

/// TermFrequency: Lightweight TF of query terms in file content.
pub fn term_frequency(
    query: &str,
    file_path: &str,
    index: &CodebaseIndex,
    expanded_tokens: Option<&HashSet<String>>,
) -> SignalResult {
    let tf_map = match index.term_frequencies.get(file_path) {
        Some(m) => m,
        None => {
            return SignalResult {
                name: "term_frequency",
                score: 0.0,
                detail: "file not found".to_string(),
            }
        }
    };

    if tf_map.is_empty() {
        return SignalResult {
            name: "term_frequency",
            score: 0.0,
            detail: "no terms".to_string(),
        };
    }

    let owned_tokens;
    let query_tokens = match expanded_tokens {
        Some(tokens) => tokens,
        None => {
            owned_tokens = tokenize(query);
            &owned_tokens
        }
    };
    if query_tokens.is_empty() {
        return SignalResult {
            name: "term_frequency",
            score: 0.0,
            detail: "empty query".to_string(),
        };
    }

    let total_terms: u32 = tf_map.values().sum();
    if total_terms == 0 {
        return SignalResult {
            name: "term_frequency",
            score: 0.0,
            detail: "no terms".to_string(),
        };
    }

    let mut matched_count: u32 = 0;
    let mut matched_terms = Vec::new();
    for token in query_tokens {
        if let Some(&count) = tf_map.get(token.as_str()) {
            matched_count += count;
            matched_terms.push(format!("{}={}", token, count));
        }
    }

    if matched_count == 0 {
        return SignalResult {
            name: "term_frequency",
            score: 0.0,
            detail: "no matching terms".to_string(),
        };
    }

    // Normalize: ratio of matched term occurrences to total terms, clamped to 1.0
    let score = (matched_count as f64 / total_terms as f64).min(1.0);

    SignalResult {
        name: "term_frequency",
        score,
        detail: format!("tf={:.3}, terms: {}", score, matched_terms.join(", ")),
    }
}

/// RecencyBoost: returns a score based on git churn data for the file.
///
/// - 0.667 if the file appears in the 30-day churn bucket (recently changed)
/// - 0.0   if only in the 180-day bucket (changed but not recently)
/// - 0.5   (neutral) when no git history is available
pub fn recency_boost_signal(file_path: &str, index: &CodebaseIndex) -> SignalResult {
    let score = crate::intelligence::recent_changes::recency_score_for_file(file_path, index);
    let detail = if score > 0.6 {
        "in 30d churn bucket".to_string()
    } else if score > 0.0 {
        "no git history".to_string()
    } else {
        "in 180d bucket only".to_string()
    };
    SignalResult {
        name: "recency_boost",
        score,
        detail,
    }
}

/// PageRank: file-level importance score from the dependency graph.
///
/// Returns the pre-computed PageRank score for `file_path` from the index,
/// or 0.0 if the file is not present in the PageRank map.
pub fn pagerank_signal(file_path: &str, pagerank: &HashMap<String, f64>) -> SignalResult {
    let score = pagerank.get(file_path).copied().unwrap_or(0.0);
    SignalResult {
        name: "pagerank",
        score,
        detail: format!("pagerank={:.4}", score),
    }
}

/// EmbeddingSimilarity: cosine similarity of a pre-computed query embedding to the file's
/// stored embedding.
///
/// Requires the `embeddings` feature and an `embedding_index` in the index.
/// `query_embedding` must be computed once by the caller before the per-file loop to avoid
/// embedding the same query string on every invocation.  Pass `None` to get a neutral 0.5.
#[cfg(feature = "embeddings")]
pub fn embedding_similarity_signal(
    query_embedding: Option<&[f32]>,
    file_path: &str,
    index: &CodebaseIndex,
) -> crate::relevance::SignalResult {
    let emb_index = match &index.embedding_index {
        Some(ei) => ei,
        None => {
            return crate::relevance::SignalResult {
                name: "embedding_similarity",
                score: 0.5,
                detail: "no embedding index".to_string(),
            }
        }
    };

    let query_vec = match query_embedding {
        Some(v) => v,
        None => {
            return crate::relevance::SignalResult {
                name: "embedding_similarity",
                score: 0.5,
                detail: "no query embedding".to_string(),
            }
        }
    };

    match emb_index.cosine_similarity(file_path, query_vec) {
        Some(sim) => {
            // Cosine similarity is in [-1, 1]; map to [0, 1].
            let score = ((sim + 1.0) / 2.0).clamp(0.0, 1.0);
            crate::relevance::SignalResult {
                name: "embedding_similarity",
                score,
                detail: format!("cosine={sim:.4}"),
            }
        }
        None => crate::relevance::SignalResult {
            name: "embedding_similarity",
            score: 0.5,
            detail: "file not in embedding index".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::parser::language::{Import, ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    // --- PathSimilarity tests ---

    #[test]
    fn test_path_similarity_exact_match() {
        let result = path_similarity("api mod", "src/api/mod.rs");
        assert!(
            result.score > 0.8,
            "exact path segments should score high: {}",
            result.score
        );
    }

    #[test]
    fn test_path_similarity_partial_match() {
        let result = path_similarity("api", "src/api/middleware.rs");
        assert!(result.score > 0.0 && result.score < 1.0);
    }

    #[test]
    fn test_path_similarity_no_overlap() {
        let result = path_similarity("database", "src/api/mod.rs");
        assert!(
            result.score < 0.2,
            "no overlap should score near zero: {}",
            result.score
        );
    }

    #[test]
    fn test_path_similarity_case_insensitive() {
        let r1 = path_similarity("API", "src/api/mod.rs");
        let r2 = path_similarity("api", "src/api/mod.rs");
        assert!((r1.score - r2.score).abs() < 0.01);
    }

    #[test]
    fn test_path_similarity_nested_paths() {
        let result = path_similarity("middleware", "src/api/middleware/rate_limiter.rs");
        assert!(result.score > 0.3);
    }

    // --- SymbolMatch tests ---

    fn make_symbol_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("handler.rs");
        std::fs::write(&fp, "pub fn handle_api_request() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "handler.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 30,
        }];
        let mut pr = HashMap::new();
        pr.insert(
            "handler.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "handle_api_request".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "pub fn handle_api_request()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        CodebaseIndex::build(files, pr, &counter)
    }

    #[test]
    fn test_symbol_match_exact_hit() {
        let index = make_symbol_index();
        let result = symbol_match("handle_api_request", "handler.rs", &index, None);
        assert!(
            result.score > 0.8,
            "exact symbol match should be high: {}",
            result.score
        );
    }

    #[test]
    fn test_symbol_match_fuzzy() {
        let index = make_symbol_index();
        let result = symbol_match("api request", "handler.rs", &index, None);
        assert!(
            result.score > 0.3,
            "fuzzy match should score mid-range: {}",
            result.score
        );
    }

    #[test]
    fn test_symbol_match_no_match() {
        let index = make_symbol_index();
        let result = symbol_match("database migration", "handler.rs", &index, None);
        assert!(
            result.score < 0.2,
            "no match should be low: {}",
            result.score
        );
    }

    #[test]
    fn test_symbol_match_case_insensitive() {
        let index = make_symbol_index();
        let result = symbol_match("Handle_API_Request", "handler.rs", &index, None);
        assert!(result.score > 0.5);
    }

    #[test]
    fn test_symbol_match_no_symbols() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("empty.rs");
        std::fs::write(&fp, "// no symbols").unwrap();
        let files = vec![ScannedFile {
            relative_path: "empty.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 13,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let result = symbol_match("anything", "empty.rs", &index, None);
        assert_eq!(result.score, 0.0);
    }

    // --- ImportProximity tests ---

    #[test]
    fn test_import_proximity_with_imports() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp1 = dir.path().join("a.rs");
        let fp2 = dir.path().join("b.rs");
        std::fs::write(&fp1, "use b;").unwrap();
        std::fs::write(&fp2, "pub fn b() {}").unwrap();
        let files = vec![
            ScannedFile {
                relative_path: "a.rs".into(),
                absolute_path: fp1,
                language: Some("rust".into()),
                size_bytes: 6,
            },
            ScannedFile {
                relative_path: "b.rs".into(),
                absolute_path: fp2,
                language: Some("rust".into()),
                size_bytes: 14,
            },
        ];
        let mut pr = HashMap::new();
        pr.insert(
            "a.rs".to_string(),
            ParseResult {
                symbols: vec![],
                imports: vec![Import {
                    source: "b".into(),
                    names: vec!["b".into()],
                }],
                exports: vec![],
            },
        );
        let index = CodebaseIndex::build(files, pr, &counter);
        let result = import_proximity("a.rs", &index);
        // a.rs has 1 outgoing import → score = 1/10 = 0.1, which is > 0.0 (no-connection floor).
        assert!(
            result.score > 0.0,
            "file with imports must score above 0.0: {}",
            result.score
        );
    }

    #[test]
    fn test_import_proximity_no_imports() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("standalone.rs");
        std::fs::write(&fp, "fn standalone() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "standalone.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 18,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let result = import_proximity("standalone.rs", &index);
        assert!(
            result.score < 0.01,
            "no imports should score 0.0 (floor): {}",
            result.score
        );
    }

    // --- TermFrequency tests ---

    #[test]
    fn test_term_frequency_high_frequency() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("rate.rs");
        std::fs::write(
            &fp,
            "fn rate_limit() { check_rate(); apply_rate(); rate_exceeded(); }",
        )
        .unwrap();
        let files = vec![ScannedFile {
            relative_path: "rate.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 62,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let result = term_frequency("rate limit", "rate.rs", &index, None);
        assert!(
            result.score > 0.5,
            "high term frequency should score high: {}",
            result.score
        );
    }

    #[test]
    fn test_term_frequency_missing_terms() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("unrelated.rs");
        std::fs::write(&fp, "fn hello_world() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "unrelated.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 20,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let result = term_frequency("database migration", "unrelated.rs", &index, None);
        assert_eq!(result.score, 0.0);
    }

    #[test]
    fn test_term_frequency_nonexistent_file() {
        let counter = TokenCounter::new();
        let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
        let result = term_frequency("test", "nonexistent.rs", &index, None);
        assert_eq!(result.score, 0.0);
    }

    // --- tokenize() edge cases ---

    #[test]
    fn test_tokenize_empty_string() {
        let tokens = tokenize("");
        assert!(tokens.is_empty(), "empty input should produce no tokens");
    }

    #[test]
    fn test_tokenize_single_char_dropped() {
        let tokens = tokenize("a b c");
        assert!(
            tokens.is_empty(),
            "single-char words should be filtered out: {:?}",
            tokens
        );
    }

    #[test]
    fn test_tokenize_snake_case() {
        let tokens = tokenize("rate_limit");
        assert!(
            tokens.contains("rate"),
            "should split snake_case: {:?}",
            tokens
        );
        assert!(
            tokens.contains("limit"),
            "should split snake_case: {:?}",
            tokens
        );
    }

    #[test]
    fn test_tokenize_camel_case() {
        let tokens = tokenize("handleRequest");
        assert!(
            tokens.contains("handle"),
            "should split camelCase: {:?}",
            tokens
        );
        assert!(
            tokens.contains("request"),
            "should split camelCase: {:?}",
            tokens
        );
    }

    #[test]
    fn test_tokenize_all_caps_kept_whole() {
        // "API" has no camelCase split → parts is empty → kept as "api"
        let tokens = tokenize("API");
        assert!(
            tokens.contains("api"),
            "all-caps word should be lowered and kept: {:?}",
            tokens
        );
    }

    #[test]
    fn test_tokenize_mixed_separators() {
        let tokens = tokenize("fix the auth/login bug");
        assert!(tokens.contains("fix"));
        assert!(tokens.contains("the"));
        assert!(tokens.contains("auth"));
        assert!(tokens.contains("login"));
        assert!(tokens.contains("bug"));
    }

    #[test]
    fn test_tokenize_special_chars_only() {
        let tokens = tokenize("!@#$%");
        assert!(
            tokens.is_empty(),
            "punctuation-only should produce no tokens: {:?}",
            tokens
        );
    }

    // --- import_proximity segment matching ---

    #[test]
    fn test_import_proximity_segment_match() {
        // Test that import source "crate::middleware" matches file "middleware.rs"
        // by segment, not substring.
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp1 = dir.path().join("api.rs");
        let fp2 = dir.path().join("middleware.rs");
        let fp3 = dir.path().join("ware.rs"); // should NOT match "middleware"
        std::fs::write(&fp1, "use middleware;").unwrap();
        std::fs::write(&fp2, "pub fn mw() {}").unwrap();
        std::fs::write(&fp3, "pub fn ware() {}").unwrap();
        let files = vec![
            ScannedFile {
                relative_path: "api.rs".into(),
                absolute_path: fp1,
                language: Some("rust".into()),
                size_bytes: 15,
            },
            ScannedFile {
                relative_path: "middleware.rs".into(),
                absolute_path: fp2,
                language: Some("rust".into()),
                size_bytes: 15,
            },
            ScannedFile {
                relative_path: "ware.rs".into(),
                absolute_path: fp3,
                language: Some("rust".into()),
                size_bytes: 15,
            },
        ];
        let mut pr = HashMap::new();
        pr.insert(
            "api.rs".to_string(),
            ParseResult {
                symbols: vec![],
                imports: vec![Import {
                    source: "crate::middleware".into(),
                    names: vec!["middleware".into()],
                }],
                exports: vec![],
            },
        );
        let index = CodebaseIndex::build(files, pr, &counter);

        // middleware.rs should match (segment "middleware" == path stem "middleware") → score > 0.0
        let result_mw = import_proximity("middleware.rs", &index);
        assert!(
            result_mw.score > 0.0,
            "middleware.rs should match via segment: {}",
            result_mw.score
        );

        // ware.rs should NOT match (no segment equals "ware") → score == 0.0
        let result_ware = import_proximity("ware.rs", &index);
        assert!(
            result_ware.score < 0.01,
            "ware.rs should not match 'middleware' by substring: {}",
            result_ware.score
        );
    }

    // --- pagerank_signal tests ---

    #[test]
    fn test_pagerank_signal_found() {
        let mut pr = HashMap::new();
        pr.insert("src/lib.rs".to_string(), 0.7531);
        pr.insert("src/main.rs".to_string(), 1.0);

        let result = pagerank_signal("src/lib.rs", &pr);
        assert_eq!(result.name, "pagerank");
        assert!(
            (result.score - 0.7531).abs() < 1e-9,
            "expected 0.7531, got {}",
            result.score
        );
        assert!(
            result.detail.contains("0.7531"),
            "detail: {}",
            result.detail
        );
    }

    #[test]
    fn test_pagerank_signal_not_found() {
        let pr: HashMap<String, f64> = HashMap::new();
        let result = pagerank_signal("nonexistent.rs", &pr);
        assert_eq!(result.name, "pagerank");
        assert_eq!(result.score, 0.0, "missing file should return 0.0");
        assert!(
            result.detail.contains("0.0000"),
            "detail: {}",
            result.detail
        );
    }

    // --- additional path_similarity edge cases ---

    #[test]
    fn test_path_similarity_empty_query_returns_zero() {
        // Empty query → empty query_tokens → returns score 0.0 with "empty tokens"
        let result = path_similarity("", "src/api/mod.rs");
        assert_eq!(result.score, 0.0);
        assert_eq!(result.detail, "empty tokens");
    }

    #[test]
    fn test_path_similarity_empty_path_returns_zero() {
        let result = path_similarity("api", "");
        assert_eq!(result.score, 0.0);
        assert_eq!(result.detail, "empty tokens");
    }

    #[test]
    fn test_path_similarity_only_punctuation_query() {
        // Only single-char/punctuation tokens get filtered → empty query tokens
        let result = path_similarity("!@#", "src/api/mod.rs");
        assert_eq!(result.score, 0.0);
    }

    // --- additional symbol_match edge cases ---

    #[test]
    fn test_symbol_match_file_not_found() {
        let counter = TokenCounter::new();
        let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
        let result = symbol_match("anything", "missing.rs", &index, None);
        assert_eq!(result.score, 0.0);
        assert_eq!(result.detail, "file not found");
    }

    #[test]
    fn test_symbol_match_no_parse_result() {
        // A file in the index but with no parse result
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("noparse.rs");
        std::fs::write(&fp, "fn x() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "noparse.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 9,
        }];
        // Empty parse_results map → file ends up with no parse result
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        // CodebaseIndex::build assigns parse results from the map; if missing, file.parse_result = None
        let result = symbol_match("anything", "noparse.rs", &index, None);
        // Either "no parse result" or "no symbols" or "empty query" depending on Rust parser behavior;
        // we just assert it gracefully returns 0.0 for the empty/missing case.
        assert_eq!(result.score, 0.0);
    }

    #[test]
    fn test_symbol_match_empty_query_with_expanded_tokens() {
        let index = make_symbol_index();
        // Pass an explicitly empty expanded token set
        let empty: HashSet<String> = HashSet::new();
        let result = symbol_match("anything", "handler.rs", &index, Some(&empty));
        assert_eq!(result.score, 0.0);
        assert_eq!(result.detail, "empty query");
    }

    #[test]
    fn test_symbol_match_uses_expanded_tokens_when_provided() {
        let index = make_symbol_index();
        let mut tokens = HashSet::new();
        tokens.insert("handle".to_string());
        tokens.insert("api".to_string());
        let result = symbol_match("totally unrelated", "handler.rs", &index, Some(&tokens));
        // The expanded tokens overrule the raw query → should get a real score
        assert!(result.score > 0.0);
    }

    // --- additional import_proximity edge cases ---

    #[test]
    fn test_import_proximity_file_not_found() {
        let counter = TokenCounter::new();
        let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
        let result = import_proximity("missing.rs", &index);
        assert_eq!(result.score, 0.5);
        assert_eq!(result.detail, "file not found");
    }

    #[test]
    fn test_import_proximity_short_stem_skipped() {
        // A file with a very short stem (e.g. "a.rs") whose stem length is <2.
        // The import-stem guard should skip it as a possible incoming reference.
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp1 = dir.path().join("a.rs");
        let fp2 = dir.path().join("user.rs");
        std::fs::write(&fp1, "// short stem file").unwrap();
        std::fs::write(&fp2, "use a;").unwrap();
        let files = vec![
            ScannedFile {
                relative_path: "a.rs".into(),
                absolute_path: fp1,
                language: Some("rust".into()),
                size_bytes: 18,
            },
            ScannedFile {
                relative_path: "user.rs".into(),
                absolute_path: fp2,
                language: Some("rust".into()),
                size_bytes: 6,
            },
        ];
        let mut pr = HashMap::new();
        pr.insert(
            "user.rs".to_string(),
            ParseResult {
                symbols: vec![],
                imports: vec![Import {
                    source: "a".into(),
                    names: vec!["a".into()],
                }],
                exports: vec![],
            },
        );
        let index = CodebaseIndex::build(files, pr, &counter);
        // a.rs has no outgoing imports and stem "a" is too short → no incoming match → 0.0
        let result = import_proximity("a.rs", &index);
        assert!(
            result.score < 0.01,
            "short stem with no connections should score 0.0: {}",
            result.score
        );
    }

    // --- additional term_frequency edge cases ---

    #[test]
    fn test_term_frequency_file_not_found() {
        let counter = TokenCounter::new();
        let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
        let result = term_frequency("query", "missing.rs", &index, None);
        assert_eq!(result.score, 0.0);
        assert_eq!(result.detail, "file not found");
    }

    #[test]
    fn test_term_frequency_empty_query_with_expanded_tokens() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("data.rs");
        std::fs::write(&fp, "fn process_data() { handle_data(); }").unwrap();
        let files = vec![ScannedFile {
            relative_path: "data.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 36,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let empty: HashSet<String> = HashSet::new();
        let result = term_frequency("anything", "data.rs", &index, Some(&empty));
        assert_eq!(result.score, 0.0);
        assert_eq!(result.detail, "empty query");
    }

    #[test]
    fn test_term_frequency_uses_expanded_tokens() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("data.rs");
        std::fs::write(
            &fp,
            "fn process_data() { handle_data(); apply_data(); save_data(); }",
        )
        .unwrap();
        let files = vec![ScannedFile {
            relative_path: "data.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 64,
        }];
        let index = CodebaseIndex::build(files, HashMap::new(), &counter);
        let mut expanded = HashSet::new();
        expanded.insert("data".to_string());
        let result = term_frequency("totally unrelated", "data.rs", &index, Some(&expanded));
        // expanded tokens should override the raw query → produces score>0
        assert!(result.score > 0.0);
    }

    // --- recency_boost_signal tests ---

    #[test]
    fn test_recency_boost_signal_no_git_history() {
        // Empty index → no churn data → neutral 0.5
        let counter = TokenCounter::new();
        let index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
        let result = recency_boost_signal("any.rs", &index);
        assert_eq!(result.name, "recency_boost");
        // 0.5 → falls in else branch → "no git history"
        assert_eq!(result.detail, "no git history");
        assert!((result.score - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_recency_boost_signal_in_30d_bucket() {
        use crate::conventions::git_health::ChurnEntry;
        let counter = TokenCounter::new();
        let mut index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
        index.conventions.git_health.churn_30d.push(ChurnEntry {
            path: "src/hot.rs".to_string(),
            modifications: 5,
        });
        let result = recency_boost_signal("src/hot.rs", &index);
        assert_eq!(result.name, "recency_boost");
        assert_eq!(result.detail, "in 30d churn bucket");
        assert!(result.score > 0.6);
    }

    #[test]
    fn test_recency_boost_signal_in_180d_only() {
        use crate::conventions::git_health::ChurnEntry;
        let counter = TokenCounter::new();
        let mut index = CodebaseIndex::build(vec![], HashMap::new(), &counter);
        index.conventions.git_health.churn_180d.push(ChurnEntry {
            path: "src/cold.rs".to_string(),
            modifications: 2,
        });
        let result = recency_boost_signal("src/cold.rs", &index);
        assert_eq!(result.name, "recency_boost");
        assert_eq!(result.detail, "in 180d bucket only");
        assert_eq!(result.score, 0.0);
    }
}
