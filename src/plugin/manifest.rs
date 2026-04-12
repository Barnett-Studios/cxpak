use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginsManifest {
    pub plugins: Vec<PluginEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginEntry {
    pub name: String,
    pub path: String,
    pub checksum: String,
    pub file_patterns: Vec<String>,
    #[serde(default)]
    pub needs_content: bool,
}

pub fn load_manifest(repo_root: &Path) -> Result<PluginsManifest, Box<dyn std::error::Error>> {
    let manifest_path = repo_root.join(".cxpak/plugins.json");
    if !manifest_path.exists() {
        return Ok(PluginsManifest { plugins: vec![] });
    }
    let content = std::fs::read_to_string(&manifest_path)?;
    let manifest: PluginsManifest = serde_json::from_str(&content)?;
    Ok(manifest)
}

/// Build an `IndexSnapshot` from the codebase index, filtered to the files
/// matching `entry.file_patterns`. Content is only populated when
/// `entry.needs_content` is true.
pub fn build_index_snapshot(
    index: &crate::index::CodebaseIndex,
    entry: &PluginEntry,
) -> super::IndexSnapshot {
    let files: Vec<super::FileSnapshot> = index
        .files
        .iter()
        .filter(|f| patterns_match(&entry.file_patterns, &f.relative_path))
        .map(|f| {
            let public_symbols = f
                .parse_result
                .as_ref()
                .map(|pr| {
                    pr.symbols
                        .iter()
                        .filter(|s| s.visibility == crate::parser::language::Visibility::Public)
                        .map(|s| s.name.clone())
                        .collect()
                })
                .unwrap_or_default();

            let imports = f
                .parse_result
                .as_ref()
                .map(|pr| pr.imports.iter().map(|i| i.source.clone()).collect())
                .unwrap_or_default();

            super::FileSnapshot {
                path: f.relative_path.clone(),
                language: f.language.clone(),
                token_count: f.token_count,
                content: if entry.needs_content {
                    Some(f.content.clone())
                } else {
                    None
                },
                public_symbols,
                imports,
            }
        })
        .collect();

    super::IndexSnapshot {
        total_files: files.len(),
        files,
        pagerank: index.pagerank.clone(),
    }
}

/// Returns true if any of the glob patterns matches the given path.
/// Supports `**` as a multi-segment wildcard, `*` as a single-segment wildcard.
fn patterns_match(patterns: &[String], path: &str) -> bool {
    if patterns.is_empty() {
        return true;
    }
    patterns.iter().any(|pat| glob_match(pat, path))
}

/// Minimal glob matcher supporting `**` and `*` wildcards.
fn glob_match(pattern: &str, path: &str) -> bool {
    glob_match_inner(pattern.as_bytes(), path.as_bytes())
}

fn glob_match_inner(pattern: &[u8], path: &[u8]) -> bool {
    match (pattern.first(), path.first()) {
        // Both exhausted — match
        (None, None) => true,
        // Pattern exhausted but path remains — only match if trailing slashes
        (None, _) => false,
        // Double-star: skip any number of path segments
        (Some(b'*'), Some(b'*')) if pattern.get(1) == Some(&b'*') => {
            let rest_pat = if pattern.len() > 2 && pattern[2] == b'/' {
                &pattern[3..]
            } else {
                &pattern[2..]
            };
            // Try matching at each position in path
            for start in 0..=path.len() {
                if glob_match_inner(rest_pat, &path[start..]) {
                    return true;
                }
                // Advance past the next path byte (including separator)
                if start < path.len() {
                    // continue
                } else {
                    break;
                }
            }
            false
        }
        // Single star: matches anything up to the next separator
        (Some(b'*'), _) => {
            let rest_pat = &pattern[1..];
            for i in 0..=path.len() {
                if i > 0 && path[i - 1] == b'/' {
                    break;
                }
                if glob_match_inner(rest_pat, &path[i..]) {
                    return true;
                }
            }
            false
        }
        // Literal: must match exactly
        (Some(&pc), Some(&tc)) => pc == tc && glob_match_inner(&pattern[1..], &path[1..]),
        // Pattern has more characters but path exhausted
        _ => false,
    }
}

pub fn verify_checksum(path: &Path, expected: &str) -> Result<(), Box<dyn std::error::Error>> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    if hash != expected {
        return Err(format!(
            "checksum mismatch for {}: expected {expected}, got {hash}",
            path.display()
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_deserializes_with_missing_needs_content() {
        let json = r#"{"plugins":[{"name":"foo","path":"plugins/foo.wasm","checksum":"abc","file_patterns":["**/*.rs"]}]}"#;
        let manifest: PluginsManifest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(manifest.plugins.len(), 1);
        assert!(!manifest.plugins[0].needs_content);
    }

    #[test]
    fn load_manifest_on_non_existent_path_returns_empty() {
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let result = load_manifest(tmp.path()).expect("load");
        assert!(result.plugins.is_empty());
    }

    #[test]
    fn build_index_snapshot_filters_by_pattern() {
        use crate::index::CodebaseIndex;
        use std::collections::HashMap;

        // Build a minimal CodebaseIndex with two files
        let index = CodebaseIndex {
            files: vec![
                crate::index::IndexedFile {
                    relative_path: "src/main.py".to_string(),
                    language: Some("Python".to_string()),
                    size_bytes: 100,
                    token_count: 10,
                    parse_result: None,
                    content: "print('hello')".to_string(),
                    mtime_secs: None,
                },
                crate::index::IndexedFile {
                    relative_path: "src/lib.rs".to_string(),
                    language: Some("Rust".to_string()),
                    size_bytes: 200,
                    token_count: 20,
                    parse_result: None,
                    content: "fn main() {}".to_string(),
                    mtime_secs: None,
                },
            ],
            language_stats: HashMap::new(),
            total_files: 2,
            total_bytes: 300,
            total_tokens: 30,
            term_frequencies: HashMap::new(),
            domains: std::collections::HashSet::new(),
            schema: None,
            graph: crate::index::graph::DependencyGraph::default(),
            pagerank: HashMap::new(),
            test_map: HashMap::new(),
            conventions: crate::conventions::ConventionProfile::default(),
            call_graph: crate::intelligence::call_graph::CallGraph::default(),
            co_changes: vec![],
            cross_lang_edges: vec![],
            #[cfg(feature = "embeddings")]
            embedding_index: None,
        };

        let entry = PluginEntry {
            name: "py-analyzer".to_string(),
            path: "plugins/py.wasm".to_string(),
            checksum: "abc".to_string(),
            file_patterns: vec!["**/*.py".to_string()],
            needs_content: false,
        };

        let snapshot = build_index_snapshot(&index, &entry);
        assert_eq!(snapshot.files.len(), 1);
        assert_eq!(snapshot.files[0].path, "src/main.py");
        assert!(snapshot.files[0].content.is_none());
    }

    #[test]
    fn verify_checksum_with_correct_hash_succeeds() {
        use sha2::{Digest, Sha256};
        use std::io::Write;

        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let data = b"hello plugin";
        tmp.write_all(data).expect("write");
        tmp.flush().expect("flush");

        let hash = format!("{:x}", Sha256::digest(data));
        verify_checksum(tmp.path(), &hash).expect("checksum should succeed");
    }

    #[test]
    fn verify_checksum_with_wrong_hash_fails() {
        use std::io::Write;

        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        tmp.write_all(b"hello plugin").expect("write");
        tmp.flush().expect("flush");

        let result = verify_checksum(tmp.path(), "deadbeef");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("checksum mismatch"), "got: {msg}");
    }
}
