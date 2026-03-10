use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Create a temp git repo with two Rust files where `main.rs` uses a function
/// defined in `lib.rs`.
fn make_trace_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();

    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    // lib.rs defines a public function `compute`
    std::fs::write(
        src_dir.join("lib.rs"),
        "pub fn compute(x: i32) -> i32 {\n    x * 2\n}\n",
    )
    .unwrap();

    // main.rs imports and calls compute
    std::fs::write(
        src_dir.join("main.rs"),
        "use crate::compute;\n\nfn main() {\n    let result = compute(21);\n    println!(\"{}\", result);\n}\n",
    )
    .unwrap();

    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"trace_test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    std::fs::write(dir.path().join("README.md"), "# Trace Test\n").unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::now("Test", "test@test.com").unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])
        .unwrap();

    dir
}

#[test]
fn test_trace_finds_function() {
    let repo = make_trace_repo();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["trace", "--tokens", "50k", "compute"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("compute"));
}

#[test]
fn test_trace_not_found() {
    let repo = make_trace_repo();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["trace", "--tokens", "50k", "nonexistent_symbol_xyz"])
        .current_dir(repo.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_trace_json_output() {
    let repo = make_trace_repo();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["trace", "--tokens", "50k", "--format", "json", "compute"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"metadata\""));
}

#[test]
fn test_trace_all_flag() {
    let repo = make_trace_repo();

    // --all triggers full BFS; should still succeed and find the symbol
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["trace", "--tokens", "50k", "--all", "compute"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("compute"));
}

#[test]
fn test_trace_out_flag() {
    let repo = make_trace_repo();
    let out_dir = TempDir::new().unwrap();
    let out_file = out_dir.path().join("trace.md");

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "trace",
            "--tokens",
            "50k",
            "--out",
            out_file.to_str().unwrap(),
            "compute",
        ])
        .current_dir(repo.path())
        .assert()
        .success();

    let content = std::fs::read_to_string(&out_file).unwrap();
    assert!(
        content.contains("compute"),
        "output file should mention the target"
    );
}

#[test]
fn test_trace_content_match_fallback() {
    let repo = make_trace_repo();

    // "result" appears in main.rs but is not a symbol name — should match via
    // content search and succeed.
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["trace", "--tokens", "50k", "result"])
        .current_dir(repo.path())
        .assert()
        .success();
}

#[test]
fn test_trace_not_git_repo() {
    let dir = TempDir::new().unwrap();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["trace", "--tokens", "50k", "anything"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a git repository"));
}

#[test]
fn test_trace_xml_output() {
    let repo = make_trace_repo();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["trace", "--tokens", "50k", "--format", "xml", "compute"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("<cxpak>"))
        .stdout(predicate::str::contains("compute"));
}

#[test]
fn test_trace_verbose_output() {
    let repo = make_trace_repo();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["trace", "--tokens", "50k", "--verbose", "compute"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("cxpak: scanning"))
        .stderr(predicate::str::contains("cxpak: found"))
        .stderr(predicate::str::contains("cxpak: parsed"));
}

#[test]
fn test_trace_with_path_argument() {
    let repo = make_trace_repo();

    // Run from a different directory, passing repo path as positional arg
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["trace", "--tokens", "50k", "compute"])
        .arg(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("compute"));
}

#[test]
fn test_trace_small_budget_truncates() {
    let repo = make_trace_repo();

    // Very small budget should still succeed, just with truncated output
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["trace", "--tokens", "200", "compute"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("compute"));
}

#[test]
fn test_trace_case_insensitive_symbol_match() {
    let repo = make_trace_repo();

    // "Compute" (uppercase C) should still find "compute"
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["trace", "--tokens", "50k", "Compute"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("compute"));
}

#[test]
fn test_trace_shows_matched_file() {
    let repo = make_trace_repo();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["trace", "--tokens", "50k", "compute"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("lib.rs"));
}

#[test]
fn test_trace_creates_cache() {
    let repo = make_trace_repo();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["trace", "--tokens", "50k", "compute"])
        .current_dir(repo.path())
        .assert()
        .success();

    let cache_file = repo.path().join(".cxpak").join("cache").join("cache.json");
    assert!(
        cache_file.exists(),
        "cache.json should be created after trace"
    );
}

#[test]
fn test_trace_second_run_uses_cache() {
    let repo = make_trace_repo();

    // First run
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["trace", "--tokens", "50k", "compute"])
        .current_dir(repo.path())
        .assert()
        .success();

    // Second run should produce identical output
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["trace", "--tokens", "50k", "compute"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("compute"));
}

#[test]
fn test_trace_out_json_format() {
    let repo = make_trace_repo();
    let out_dir = TempDir::new().unwrap();
    let out_file = out_dir.path().join("trace.json");

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "trace",
            "--tokens",
            "50k",
            "--format",
            "json",
            "--out",
            out_file.to_str().unwrap(),
            "compute",
        ])
        .current_dir(repo.path())
        .assert()
        .success();

    let content = std::fs::read_to_string(&out_file).unwrap();
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&content);
    assert!(parsed.is_ok(), "output file should be valid JSON");
}
