use crate::core_graph::IndexedFile;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// `TestFileRef`/`TestConfidence` are data-model types now in `core_graph`
// (cxpak 3.0.0 Phase 0 de-cycle); the mapping logic below stays here.
pub use crate::core_graph::intel::{TestConfidence, TestFileRef};

/// Returns true when `path` looks like a test file (contains a standard test
/// directory component or a test-naming marker in the filename).
fn is_test_path(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    // Check any directory component
    for part in &parts[..parts.len().saturating_sub(1)] {
        if matches!(*part, "tests" | "test" | "spec" | "__tests__") {
            return true;
        }
    }
    // Check filename markers
    if let Some(filename) = parts.last() {
        let name = filename.to_lowercase();
        if name.contains("_test.")
            || name.contains("_spec.")
            || name.starts_with("test_")
            || name.ends_with("test.rs")
            || name.ends_with("test.py")
            || name.ends_with("test.go")
            || name.ends_with("test.ts")
            || name.ends_with("test.js")
            || name.ends_with(".test.ts")
            || name.ends_with(".test.js")
            || name.ends_with(".spec.ts")
            || name.ends_with(".spec.js")
            || name.to_lowercase().ends_with("test.java")
            || name.to_lowercase().ends_with("tests.java")
        {
            return true;
        }
    }
    false
}

/// For a source file path and extension, generate candidate test paths based
/// on language-specific conventions and return those that exist in `all_paths`.
pub fn find_test_files_by_name(source_path: &str, all_paths: &HashSet<String>) -> Vec<TestFileRef> {
    // Do not generate test candidates for files that are already test files.
    if is_test_path(source_path) {
        return Vec::new();
    }

    let ext = source_path.rsplit('.').next().unwrap_or("");
    let stem = source_path
        .rsplit('.')
        .skip(1)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(".");

    // stem is the full path without extension, e.g. "src/foo" or "src/db/store"
    let filename_no_ext = stem.rsplit('/').next().unwrap_or(&stem);
    let dir_prefix = stem
        .rsplit_once('/')
        .map(|(prefix, _)| prefix)
        .unwrap_or("");

    let candidates: Vec<String> = match ext {
        "rs" => {
            // Rust: tests/ directory variants + inline _test sibling
            vec![
                format!("tests/{filename_no_ext}.rs"),
                format!("tests/{filename_no_ext}_test.rs"),
                format!("{stem}_test.rs"),
                format!("tests/test_{filename_no_ext}.rs"),
            ]
        }
        "py" => {
            vec![
                format!("tests/test_{filename_no_ext}.py"),
                format!("tests/{filename_no_ext}_test.py"),
                format!("test_{filename_no_ext}.py"),
            ]
        }
        "java" => {
            vec![
                // Same directory as source (Java src tree)
                if dir_prefix.is_empty() {
                    format!("{filename_no_ext}Test.java")
                } else {
                    format!("{dir_prefix}/{filename_no_ext}Test.java")
                },
                format!("test/{filename_no_ext}Test.java"),
                format!("tests/{filename_no_ext}Test.java"),
            ]
        }
        "ts" => {
            vec![
                // Co-located test files
                format!("{stem}.test.ts"),
                format!("{stem}.spec.ts"),
                format!("tests/{filename_no_ext}.test.ts"),
                format!("__tests__/{filename_no_ext}.test.ts"),
            ]
        }
        "go" => {
            // Go: always same directory, append _test to filename before extension
            vec![format!("{stem}_test.go")]
        }
        "rb" => {
            vec![
                format!("spec/{filename_no_ext}_spec.rb"),
                format!("test/{filename_no_ext}_test.rb"),
                // Mirror the full source directory tree under spec/
                // e.g. lib/foo.rb → spec/lib/foo_spec.rb
                format!("spec/{stem}_spec.rb"),
            ]
        }
        _ => {
            // General catch-all for any other extension
            general_test_candidates(filename_no_ext, dir_prefix, ext)
        }
    };

    candidates
        .into_iter()
        .filter(|c| all_paths.contains(c))
        .map(|path| TestFileRef {
            path,
            confidence: TestConfidence::NameMatch,
        })
        .collect()
}

/// Generate general-purpose test candidates for languages not explicitly handled.
fn general_test_candidates(filename_no_ext: &str, dir_prefix: &str, ext: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let prefix = if dir_prefix.is_empty() {
        String::new()
    } else {
        format!("{dir_prefix}/")
    };

    // Suffix variants
    for suffix in &["_test", "_spec"] {
        candidates.push(format!("{prefix}{filename_no_ext}{suffix}.{ext}"));
    }
    // Dot-separated test/spec (e.g. .test.ts style for non-ts too)
    for marker in &["test", "spec"] {
        candidates.push(format!("{prefix}{filename_no_ext}.{marker}.{ext}"));
    }
    // PascalCase Test/Tests suffix
    for suffix in &["Test", "Tests"] {
        candidates.push(format!("{prefix}{filename_no_ext}{suffix}.{ext}"));
    }
    // Standard test directories
    for dir in &["tests", "test", "spec", "__tests__"] {
        candidates.push(format!("{dir}/{filename_no_ext}.{ext}"));
        candidates.push(format!("{dir}/test_{filename_no_ext}.{ext}"));
        candidates.push(format!("{dir}/{filename_no_ext}_test.{ext}"));
    }
    candidates
}

/// For each test file (identified by path containing `test`, `spec`, or `__tests__`),
/// examine imports and resolve them back to source file paths. Returns a map of
/// source_path -> Vec<TestFileRef>.
///
/// `crate_name` is the analysed Rust project's own crate name (from its
/// `Cargo.toml`, see [`crate::index::graph::detect_crate_name`]). A Rust
/// integration test under `tests/` can only import the crate's own name
/// (`use cxpak::commands::serve::build_index;` — `use crate::...` is a
/// compile error from that directory), so that prefix is stripped before
/// candidate generation, same as `crate::` would be. `None` for non-Rust
/// projects, or when no root `Cargo.toml` was found (cxpak#84 cause 2).
pub fn find_test_files_by_imports(
    files: &[Arc<IndexedFile>],
    crate_name: Option<&str>,
) -> HashMap<String, Vec<TestFileRef>> {
    // Build a lookup set of all file paths for fast resolution
    let all_paths: HashSet<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();

    let mut result: HashMap<String, Vec<TestFileRef>> = HashMap::new();

    for file in files {
        // Only consider files that look like test files
        if !is_test_path(&file.relative_path) {
            continue;
        }

        let Some(pr) = &file.parse_result else {
            continue;
        };

        for import in &pr.imports {
            // Strip the crate's own name as an import prefix — the only form
            // Rust accepts from an integration test — before resolving, same
            // as `resolve_rust_import` treats `<crate_name>::X` as `crate::X`.
            let source = crate_name
                .and_then(|name| import.source.strip_prefix(name))
                .and_then(|rest| rest.strip_prefix("::"))
                .unwrap_or(&import.source);

            let candidate_base = source.replace("::", "/").replace('.', "/");
            let base_parts: Vec<&str> = candidate_base
                .split('/')
                .filter(|s| !s.is_empty())
                .collect();
            if base_parts.is_empty() {
                continue;
            }

            // Progressive right-stripping: try the full path first, then
            // shorter prefixes, so e.g. `cxpak::commands::serve::build_index`
            // falls back to `src/commands/serve.rs` when `build_index` is a
            // function (not a file) in that module — same pattern
            // `resolve_rust_import`'s `crate::` handling already uses for
            // re-exported types.
            let mut resolved: Option<String> = None;
            'shorten: for take in (1..=base_parts.len()).rev() {
                let base = base_parts[..take].join("/");
                let candidates = [
                    format!("{base}.rs"),
                    format!("{base}/mod.rs"),
                    format!("src/{base}.rs"),
                    format!("src/{base}/mod.rs"),
                    format!("{base}.ts"),
                    format!("{base}.js"),
                    format!("{base}.py"),
                    format!("{base}.go"),
                    format!("{base}.java"),
                    format!("{base}.rb"),
                ];
                for candidate in &candidates {
                    if all_paths.contains(candidate.as_str()) {
                        resolved = Some(candidate.clone());
                        break 'shorten;
                    }
                }
            }

            if let Some(candidate) = resolved {
                result.entry(candidate).or_default().push(TestFileRef {
                    path: file.relative_path.clone(),
                    confidence: TestConfidence::ImportMatch,
                });
            }
        }
    }

    result
}

/// Build a complete test map: source_path → Vec<TestFileRef>.
///
/// Combines naming-convention matching and import-based matching. When the same
/// test file is found by both methods, its confidence is upgraded to `Both`.
///
/// `crate_name` is forwarded to [`find_test_files_by_imports`] — see its doc
/// comment. Callers typically pass
/// `crate::index::graph::detect_crate_name(files)`.
pub fn build_test_map(
    files: &[Arc<IndexedFile>],
    all_paths: &HashSet<String>,
    crate_name: Option<&str>,
) -> HashMap<String, Vec<TestFileRef>> {
    let mut map: HashMap<String, Vec<TestFileRef>> = HashMap::new();

    // Step 1: naming convention matching for every non-test source file
    for file in files {
        if !is_test_path(&file.relative_path) {
            let refs = find_test_files_by_name(&file.relative_path, all_paths);
            if !refs.is_empty() {
                map.entry(file.relative_path.clone())
                    .or_default()
                    .extend(refs);
            }
        }
    }

    // Step 2: import-based matching
    let import_map = find_test_files_by_imports(files, crate_name);

    // Step 3: merge — upgrade confidence to Both where both methods agree
    for (source_path, import_refs) in import_map {
        let name_refs = map.entry(source_path).or_default();
        for import_ref in import_refs {
            // Check if this test path was already found by name
            if let Some(existing) = name_refs.iter_mut().find(|r| r.path == import_ref.path) {
                if existing.confidence == TestConfidence::NameMatch {
                    existing.confidence = TestConfidence::Both;
                }
                // If already ImportMatch or Both, leave as-is
            } else {
                name_refs.push(import_ref);
            }
        }
    }

    // Step 4: deduplicate within each source's list (same path may appear twice
    // from name matching if multiple candidate patterns hit the same file)
    for refs in map.values_mut() {
        let mut seen: HashSet<String> = HashSet::new();
        refs.retain(|r| seen.insert(r.path.clone()));
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::language::{Import, ParseResult};

    fn make_paths(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|s| s.to_string()).collect()
    }

    fn make_file(path: &str, imports: Vec<Import>) -> Arc<IndexedFile> {
        Arc::new(IndexedFile {
            relative_path: path.to_string(),
            language: None,
            size_bytes: 0,
            token_count: 0,
            parse_result: Some(ParseResult {
                symbols: vec![],
                imports,
                exports: vec![],
            }),
            content: String::new(),
            mtime_ns: None,
        })
    }

    fn make_import(source: &str) -> Import {
        Import {
            source: source.to_string(),
            names: vec![],
        }
    }

    // -------------------------------------------------------------------------
    // Task 7: naming convention tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_naming_rust() {
        let paths = make_paths(&[
            "tests/foo.rs",
            "tests/foo_test.rs",
            "src/foo_test.rs",
            "tests/test_foo.rs",
        ]);
        let refs = find_test_files_by_name("src/foo.rs", &paths);
        let found: HashSet<String> = refs.iter().map(|r| r.path.clone()).collect();
        assert!(found.contains("tests/foo.rs"), "expected tests/foo.rs");
        assert!(
            found.contains("tests/foo_test.rs"),
            "expected tests/foo_test.rs"
        );
        assert!(
            found.contains("src/foo_test.rs"),
            "expected src/foo_test.rs"
        );
        assert!(
            found.contains("tests/test_foo.rs"),
            "expected tests/test_foo.rs"
        );
        for r in &refs {
            assert_eq!(r.confidence, TestConfidence::NameMatch);
        }
    }

    #[test]
    fn test_naming_python() {
        let paths = make_paths(&["tests/test_foo.py", "tests/foo_test.py", "test_foo.py"]);
        let refs = find_test_files_by_name("src/foo.py", &paths);
        let found: HashSet<String> = refs.iter().map(|r| r.path.clone()).collect();
        assert!(
            found.contains("tests/test_foo.py"),
            "expected tests/test_foo.py"
        );
        assert!(
            found.contains("tests/foo_test.py"),
            "expected tests/foo_test.py"
        );
        assert!(found.contains("test_foo.py"), "expected test_foo.py");
    }

    #[test]
    fn test_naming_java() {
        let paths = make_paths(&[
            "src/FooTest.java",
            "test/FooTest.java",
            "tests/FooTest.java",
        ]);
        let refs = find_test_files_by_name("src/Foo.java", &paths);
        let found: HashSet<String> = refs.iter().map(|r| r.path.clone()).collect();
        assert!(
            found.contains("src/FooTest.java"),
            "expected src/FooTest.java"
        );
        assert!(
            found.contains("test/FooTest.java"),
            "expected test/FooTest.java"
        );
        assert!(
            found.contains("tests/FooTest.java"),
            "expected tests/FooTest.java"
        );
    }

    #[test]
    fn test_naming_typescript() {
        let paths = make_paths(&[
            "src/foo.test.ts",
            "src/foo.spec.ts",
            "tests/foo.test.ts",
            "__tests__/foo.test.ts",
        ]);
        let refs = find_test_files_by_name("src/foo.ts", &paths);
        let found: HashSet<String> = refs.iter().map(|r| r.path.clone()).collect();
        assert!(
            found.contains("src/foo.test.ts"),
            "expected src/foo.test.ts"
        );
        assert!(
            found.contains("src/foo.spec.ts"),
            "expected src/foo.spec.ts"
        );
        assert!(
            found.contains("tests/foo.test.ts"),
            "expected tests/foo.test.ts"
        );
        assert!(
            found.contains("__tests__/foo.test.ts"),
            "expected __tests__/foo.test.ts"
        );
    }

    #[test]
    fn test_naming_go() {
        // CRITICAL: full directory path must be preserved — src/db/store_test.go
        // NOT src/store_test.go or tests/store_test.go
        let paths = make_paths(&["src/db/store_test.go"]);
        let refs = find_test_files_by_name("src/db/store.go", &paths);
        assert_eq!(refs.len(), 1, "expected exactly 1 match");
        assert_eq!(refs[0].path, "src/db/store_test.go");
        // Ensure wrong paths are NOT matched
        let wrong_paths = make_paths(&["src/store_test.go", "tests/store_test.go"]);
        let refs2 = find_test_files_by_name("src/db/store.go", &wrong_paths);
        assert!(
            refs2.is_empty(),
            "should not match wrong-directory go test file"
        );
    }

    #[test]
    fn test_naming_ruby() {
        let paths = make_paths(&[
            "spec/foo_spec.rb",
            "test/foo_test.rb",
            "spec/lib/foo_spec.rb",
        ]);
        let refs = find_test_files_by_name("lib/foo.rb", &paths);
        let found: HashSet<String> = refs.iter().map(|r| r.path.clone()).collect();
        assert!(
            found.contains("spec/foo_spec.rb"),
            "expected spec/foo_spec.rb"
        );
        assert!(
            found.contains("test/foo_test.rb"),
            "expected test/foo_test.rb"
        );
        assert!(
            found.contains("spec/lib/foo_spec.rb"),
            "expected spec/lib/foo_spec.rb"
        );
    }

    #[test]
    fn test_naming_no_match() {
        // A standalone source file with no test counterpart should return empty vec
        let paths = make_paths(&["src/other.rs"]);
        let refs = find_test_files_by_name("standalone.rs", &paths);
        assert!(refs.is_empty(), "expected no matches for standalone.rs");
    }

    // -------------------------------------------------------------------------
    // Task 8: import analysis + build_test_map tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_import_analysis() {
        // A test file that imports a source module → mapped to that source file
        let files = vec![
            make_file("src/db.rs", vec![]),
            make_file(
                "tests/db_test.rs",
                vec![make_import("src::db"), make_import("src/db")],
            ),
        ];
        let import_map = find_test_files_by_imports(&files, None);
        // "src/db.rs" should map to "tests/db_test.rs"
        let refs = import_map
            .get("src/db.rs")
            .expect("src/db.rs should be in map");
        assert!(
            refs.iter().any(|r| r.path == "tests/db_test.rs"),
            "tests/db_test.rs should be mapped to src/db.rs"
        );
        assert_eq!(refs[0].confidence, TestConfidence::ImportMatch);
    }

    #[test]
    fn test_both_name_and_import() {
        // A test that matches both by name and by import → confidence upgraded to Both
        let all_paths = make_paths(&["src/util.rs", "tests/util_test.rs"]);
        let files = vec![
            make_file("src/util.rs", vec![]),
            make_file("tests/util_test.rs", vec![make_import("src::util")]),
        ];
        let map = build_test_map(&files, &all_paths, None);
        let refs = map
            .get("src/util.rs")
            .expect("src/util.rs should be in map");
        let test_ref = refs
            .iter()
            .find(|r| r.path == "tests/util_test.rs")
            .expect("tests/util_test.rs should appear");
        assert_eq!(
            test_ref.confidence,
            TestConfidence::Both,
            "confidence should be Both when matched by both name and import"
        );
    }

    #[test]
    fn test_multiple_sources_per_test() {
        // Integration test imports 3 modules → maps to all 3
        let files = vec![
            make_file("src/auth.rs", vec![]),
            make_file("src/db.rs", vec![]),
            make_file("src/handler.rs", vec![]),
            make_file(
                "tests/integration_test.rs",
                vec![
                    make_import("src::auth"),
                    make_import("src::db"),
                    make_import("src::handler"),
                ],
            ),
        ];
        let import_map = find_test_files_by_imports(&files, None);

        for src in &["src/auth.rs", "src/db.rs", "src/handler.rs"] {
            let refs = import_map
                .get(*src)
                .unwrap_or_else(|| panic!("{src} should be in import map"));
            assert!(
                refs.iter().any(|r| r.path == "tests/integration_test.rs"),
                "{src} should reference tests/integration_test.rs"
            );
        }
    }

    #[test]
    fn test_multiple_tests_per_source() {
        // Source has both unit and integration test → both listed
        let all_paths = make_paths(&[
            "src/router.rs",
            "tests/router_test.rs",
            "tests/integration_test.rs",
        ]);
        let files = vec![
            make_file("src/router.rs", vec![]),
            make_file("tests/router_test.rs", vec![make_import("src::router")]),
            make_file(
                "tests/integration_test.rs",
                vec![make_import("src::router")],
            ),
        ];
        let map = build_test_map(&files, &all_paths, None);
        let refs = map
            .get("src/router.rs")
            .expect("src/router.rs should be in map");
        let paths: HashSet<&str> = refs.iter().map(|r| r.path.as_str()).collect();
        assert!(
            paths.contains("tests/router_test.rs"),
            "expected tests/router_test.rs"
        );
        assert!(
            paths.contains("tests/integration_test.rs"),
            "expected tests/integration_test.rs"
        );
    }

    #[test]
    fn test_build_test_map_empty() {
        // No test files → empty map
        let all_paths = make_paths(&["src/main.rs", "src/lib.rs"]);
        let files = vec![
            make_file("src/main.rs", vec![]),
            make_file("src/lib.rs", vec![]),
        ];
        let map = build_test_map(&files, &all_paths, None);
        assert!(
            map.is_empty(),
            "should produce empty map when no test files exist"
        );
    }

    // ── own-crate-name + progressive right-stripping (cxpak#84 cause 2) ────
    //
    // From `tests/`, `use crate::...` is a compile error — the only form
    // Rust accepts is the crate's own name. And `build_index` here is a
    // function, not a file, so the full dotted path never matches; the
    // resolver must fall back to the containing module file, the same
    // progressive-shortening pattern `resolve_rust_import`'s `crate::`
    // handling already uses for re-exported types.

    #[test]
    fn test_find_test_files_by_imports_own_crate_name_with_progressive_stripping() {
        let files = vec![
            make_file("src/commands/serve.rs", vec![]),
            make_file(
                "tests/serve_test.rs",
                vec![make_import("cxpak::commands::serve::build_index")],
            ),
        ];
        let import_map = find_test_files_by_imports(&files, Some("cxpak"));
        let refs = import_map
            .get("src/commands/serve.rs")
            .expect("src/commands/serve.rs should be in the import map");
        assert!(
            refs.iter().any(|r| r.path == "tests/serve_test.rs"),
            "tests/serve_test.rs should resolve to src/commands/serve.rs via \
             the own-crate-name prefix with progressive right-stripping"
        );
    }

    #[test]
    fn test_find_test_files_by_imports_without_crate_name_does_not_resolve() {
        // Regression guard: without a detected crate name, the own-crate
        // prefix is left on the candidate path (`cxpak/commands/serve/...`)
        // and never matches — this is the pre-fix behaviour.
        let files = vec![
            make_file("src/commands/serve.rs", vec![]),
            make_file(
                "tests/serve_test.rs",
                vec![make_import("cxpak::commands::serve::build_index")],
            ),
        ];
        let import_map = find_test_files_by_imports(&files, None);
        assert!(
            !import_map.contains_key("src/commands/serve.rs"),
            "without crate_name the own-crate-prefixed import must not resolve"
        );
    }
}
