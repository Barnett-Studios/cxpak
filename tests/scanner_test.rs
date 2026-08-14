use std::path::Path;

use cxpak::scanner::defaults::BUILTIN_IGNORES;
use cxpak::scanner::{ScanError, Scanner};

/// Absolute path to the simple_repo fixture.
fn fixture_root() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR points at the crate root (where Cargo.toml lives).
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    Path::new(&manifest)
        .join("tests")
        .join("fixtures")
        .join("simple_repo")
}

/// Ensure the `.git` directory exists in the fixture so Scanner::new() accepts it.
/// Using `std::fs::create_dir_all` is safe to call even when the directory already
/// exists.
fn ensure_git_dir(root: &Path) {
    std::fs::create_dir_all(root.join(".git")).expect("failed to create fixture .git directory");
}

// ---------------------------------------------------------------------------
// Scanner::new validation
// ---------------------------------------------------------------------------

#[test]
fn scanner_rejects_non_repo() {
    // Use a temporary directory that has no .git subdirectory.
    let tmp = tempfile::tempdir().expect("tempdir");
    let result = Scanner::new(tmp.path());
    assert!(
        matches!(result, Err(ScanError::NotARepository(_))),
        "expected NotARepository error"
    );
}

#[test]
fn scanner_accepts_fixture_with_git_dir() {
    let root = fixture_root();
    ensure_git_dir(&root);
    Scanner::new(&root).expect("Scanner::new should succeed when .git exists");
}

// ---------------------------------------------------------------------------
// File discovery
// ---------------------------------------------------------------------------

#[test]
fn scanner_finds_source_files() {
    let root = fixture_root();
    ensure_git_dir(&root);

    let scanner = Scanner::new(&root).expect("Scanner::new");
    let files = scanner.scan().expect("scan");

    let relative_paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();

    // These five files must be present.
    assert!(
        relative_paths.contains(&"src/main.rs"),
        "expected src/main.rs, got: {relative_paths:?}"
    );
    assert!(
        relative_paths.contains(&"src/lib.rs"),
        "expected src/lib.rs, got: {relative_paths:?}"
    );
    assert!(
        relative_paths.contains(&"tests/test.rs"),
        "expected tests/test.rs, got: {relative_paths:?}"
    );
    assert!(
        relative_paths.contains(&"README.md"),
        "expected README.md, got: {relative_paths:?}"
    );
    assert!(
        relative_paths.contains(&"Cargo.toml"),
        "expected Cargo.toml, got: {relative_paths:?}"
    );
}

#[test]
fn scanner_results_are_sorted_by_relative_path() {
    let root = fixture_root();
    ensure_git_dir(&root);

    let scanner = Scanner::new(&root).expect("Scanner::new");
    let files = scanner.scan().expect("scan");

    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "files should be sorted by relative_path");
}

// ---------------------------------------------------------------------------
// Gitignore / built-in ignore enforcement
// ---------------------------------------------------------------------------

#[test]
fn scanner_respects_gitignore_target_dir() {
    let root = fixture_root();
    ensure_git_dir(&root);

    let scanner = Scanner::new(&root).expect("Scanner::new");
    let files = scanner.scan().expect("scan");

    let relative_paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();

    // `target/` is in .gitignore, so target/debug/binary must not appear.
    let has_target = relative_paths
        .iter()
        .any(|p| p.starts_with("target/") || *p == "target");
    assert!(
        !has_target,
        "target/ should be excluded by .gitignore, got: {relative_paths:?}"
    );
}

#[test]
fn scanner_respects_gitignore_log_files() {
    let root = fixture_root();
    ensure_git_dir(&root);

    let scanner = Scanner::new(&root).expect("Scanner::new");
    let files = scanner.scan().expect("scan");

    let relative_paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();

    // `*.log` is in .gitignore, so app.log must not appear.
    let has_log = relative_paths.iter().any(|p| p.ends_with(".log"));
    assert!(
        !has_log,
        "*.log files should be excluded by .gitignore, got: {relative_paths:?}"
    );
}

// ---------------------------------------------------------------------------
// Built-in defaults
// ---------------------------------------------------------------------------

#[test]
fn builtin_ignores_contains_node_modules() {
    assert!(
        BUILTIN_IGNORES.contains(&"node_modules"),
        "BUILTIN_IGNORES should contain 'node_modules'"
    );
}

#[test]
fn builtin_ignores_contains_pycache() {
    assert!(
        BUILTIN_IGNORES.contains(&"__pycache__"),
        "BUILTIN_IGNORES should contain '__pycache__'"
    );
}

#[test]
fn builtin_ignores_contains_ds_store() {
    assert!(
        BUILTIN_IGNORES.contains(&".DS_Store"),
        "BUILTIN_IGNORES should contain '.DS_Store'"
    );
}

// ---------------------------------------------------------------------------
// Language detection
// ---------------------------------------------------------------------------

#[test]
fn language_detection_rust() {
    let root = fixture_root();
    ensure_git_dir(&root);

    let scanner = Scanner::new(&root).expect("Scanner::new");
    let files = scanner.scan().expect("scan");

    let main_rs = files
        .iter()
        .find(|f| f.relative_path == "src/main.rs")
        .expect("src/main.rs should be in scan results");

    assert_eq!(
        main_rs.language.as_deref(),
        Some("rust"),
        "src/main.rs should be detected as 'rust'"
    );
}

#[test]
fn language_detection_markdown() {
    let root = fixture_root();
    ensure_git_dir(&root);

    let scanner = Scanner::new(&root).expect("Scanner::new");
    let files = scanner.scan().expect("scan");

    let readme = files
        .iter()
        .find(|f| f.relative_path == "README.md")
        .expect("README.md should be in scan results");

    assert_eq!(
        readme.language.as_deref(),
        Some("markdown"),
        "README.md should be detected as 'markdown'"
    );
}

// ---------------------------------------------------------------------------
// Credential material never reaches the index (cxpak#39)
// ---------------------------------------------------------------------------

/// Build a throwaway repo containing the named files, each with trivial content.
fn repo_with(files: &[&str]) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join(".git")).expect("create .git");
    for f in files {
        let p = tmp.path().join(f);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&p, "placeholder\n").expect("write fixture file");
    }
    tmp
}

fn scanned_paths(root: &Path) -> Vec<String> {
    Scanner::new(root)
        .expect("scanner")
        .scan()
        .expect("scan")
        .into_iter()
        .map(|f| f.relative_path)
        .collect()
}

#[test]
fn credential_files_are_never_scanned() {
    // Measured on the published 3.1.4 image before this list existed: a repo holding
    // `id_rsa`, `server.key` and `credentials.json` indexed all three, and
    // `credentials.json` was packed verbatim into the `overview` bundle. The only
    // protection was whether the user's gitignore happened to cover them.
    let secrets = [
        "id_rsa",
        "id_dsa",
        "id_ecdsa",
        "id_ed25519",
        "server.key",
        "server.pem",
        "bundle.p12",
        "cert.pfx",
        "app.jks",
        "release.keystore",
        "credentials.json",
        "credentials.yml",
        "credentials.yaml",
        "secrets.json",
        "secrets.yml",
        "secrets.yaml",
        ".env",
        ".env.production",
        ".netrc",
        ".npmrc",
        ".pypirc",
        // Nested, because a denylist that only matches at the root is a denylist
        // anyone defeats by putting the key in `config/`.
        "config/deploy.key",
        "deploy/credentials.json",
    ];
    let tmp = repo_with(&secrets);
    let scanned = scanned_paths(tmp.path());
    let leaked: Vec<&String> = scanned.iter().collect();
    assert!(
        leaked.is_empty(),
        "credential material reached the index: {leaked:?}"
    );
}

#[test]
fn ordinary_source_that_merely_mentions_secrets_is_still_scanned() {
    // The bound, and the reason this list is exact names rather than the `*secret*` /
    // `*credentials*` globs the ticket proposed. Those globs drop real source out of the
    // index with no diagnostic — the same silent-exclusion defect, pointed the other way.
    //
    // Without it, `credential_files_are_never_scanned` is satisfied by a scanner that
    // returns nothing at all.
    let sources = [
        "src/secrets_manager.rs",
        "src/credentials_test.go",
        "src/SecretScanner.java",
        "src/keyring.py",
        "src/env_loader.ts",
        "docs/secrets.md",
    ];
    let tmp = repo_with(&sources);
    let scanned = scanned_paths(tmp.path());
    let mut missing: Vec<&str> = sources
        .iter()
        .copied()
        .filter(|s| !scanned.iter().any(|p| p == s))
        .collect();
    missing.sort_unstable();
    assert!(
        missing.is_empty(),
        "real source was excluded by the credential denylist: {missing:?}"
    );
}
