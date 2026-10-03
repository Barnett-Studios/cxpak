use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Create a temp git repo with two Rust files where `main.rs` uses a function
/// defined in `lib.rs`, committed to HEAD so we can diff against it.
fn make_diff_repo() -> TempDir {
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
        "[package]\nname = \"diff_test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

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
fn test_diff_no_changes() {
    let repo = make_diff_repo();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["diff", "--tokens", "50k"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes detected."));
}

#[test]
fn test_diff_shows_changes() {
    let repo = make_diff_repo();

    // Modify lib.rs so there's a working-tree change
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn compute(x: i32) -> i32 {\n    x * 3\n}\n",
    )
    .unwrap();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["diff", "--tokens", "50k"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("src/lib.rs"));
}

#[test]
fn test_diff_includes_context() {
    let repo = make_diff_repo();

    // Modify lib.rs; main.rs uses it, so it should appear as context
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn compute(x: i32) -> i32 {\n    x * 4\n}\n",
    )
    .unwrap();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["diff", "--tokens", "50k"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("lib.rs"));
}

#[test]
fn test_diff_json_format() {
    let repo = make_diff_repo();

    // Modify a file to get some output
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn compute(x: i32) -> i32 {\n    x * 5\n}\n",
    )
    .unwrap();

    let output = Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["diff", "--tokens", "50k", "--format", "json"])
        .current_dir(repo.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let content = String::from_utf8(output).unwrap();
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&content);
    assert!(parsed.is_ok(), "output should be valid JSON");
    assert!(
        content.contains("metadata"),
        "JSON should contain metadata key"
    );
}

#[test]
fn test_diff_xml_format() {
    let repo = make_diff_repo();

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn compute(x: i32) -> i32 {\n    x * 6\n}\n",
    )
    .unwrap();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["diff", "--tokens", "50k", "--format", "xml"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("<cxpak>"));
}

#[test]
fn test_diff_with_ref() {
    let repo = make_diff_repo();

    // Make a second commit changing lib.rs
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn compute(x: i32) -> i32 {\n    x * 10\n}\n",
    )
    .unwrap();

    let git_repo = git2::Repository::open(repo.path()).unwrap();
    let sig = git2::Signature::now("Test", "test@test.com").unwrap();
    let mut index = git_repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = git_repo.find_tree(tree_id).unwrap();
    let head = git_repo.head().unwrap().peel_to_commit().unwrap();
    git_repo
        .commit(Some("HEAD"), &sig, &sig, "second commit", &tree, &[&head])
        .unwrap();

    // Diff HEAD~1 against HEAD — should show lib.rs changed
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["diff", "--tokens", "50k", "--git-ref", "HEAD~1"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("src/lib.rs"));
}

#[test]
fn test_diff_out_flag() {
    let repo = make_diff_repo();
    let out_dir = TempDir::new().unwrap();
    let out_file = out_dir.path().join("diff_output.md");

    // Modify a file
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn compute(x: i32) -> i32 {\n    x * 7\n}\n",
    )
    .unwrap();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "diff",
            "--tokens",
            "50k",
            "--out",
            out_file.to_str().unwrap(),
        ])
        .current_dir(repo.path())
        .assert()
        .success();

    assert!(out_file.exists(), "--out file should be created");
    let content = std::fs::read_to_string(&out_file).unwrap();
    assert!(
        content.contains("lib.rs"),
        "output file should mention changed file"
    );
}

#[test]
fn test_diff_since_without_git_binary_on_path() {
    // Built by hand rather than via `make_diff_repo`: the oldest commit *inside* the
    // `--since` window needs a parent to diff against, so the first commit here is
    // backdated well outside the window and the second (the one the window actually
    // captures) sits on top of it.
    let dir = TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();

    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("lib.rs"),
        "pub fn compute(x: i32) -> i32 {\n    x * 2\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"diff_test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let old_time = git2::Time::new(now_secs - 30 * 24 * 3600, 0);
    let old_sig = git2::Signature::new("Test", "test@test.com", &old_time).unwrap();
    {
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(
            Some("HEAD"),
            &old_sig,
            &old_sig,
            "initial commit",
            &tree,
            &[],
        )
        .unwrap();
    }

    // Second commit, now — inside the `--since 1d` window — carrying the change to report.
    std::fs::write(
        src_dir.join("lib.rs"),
        "pub fn compute(x: i32) -> i32 {\n    x * 11\n}\n",
    )
    .unwrap();
    let sig = git2::Signature::now("Test", "test@test.com").unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "second commit", &tree, &[&parent])
        .unwrap();

    // Mirrors the published image (cxpak#108): no `git` binary anywhere on PATH.
    // `--since` must resolve the time window through git2 alone, with no subprocess spawn.
    let empty_bin = TempDir::new().unwrap();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["diff", "--tokens", "50k", "--since", "1d"])
        .current_dir(dir.path())
        .env("PATH", empty_bin.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("src/lib.rs"));
}

#[test]
fn test_diff_not_git_repo() {
    let dir = TempDir::new().unwrap();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["diff", "--tokens", "50k"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("could not find repository"));
}

#[test]
fn test_diff_verbose() {
    let repo = make_diff_repo();

    // Modify a file so we get real output
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn compute(x: i32) -> i32 {\n    x * 8\n}\n",
    )
    .unwrap();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["diff", "--tokens", "50k", "--verbose"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("cxpak:"));
}

#[test]
fn test_diff_with_path_argument() {
    let repo = make_diff_repo();

    // Modify a file
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn compute(x: i32) -> i32 {\n    x * 9\n}\n",
    )
    .unwrap();

    // Run from a different directory, passing repo path as positional arg
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["diff", "--tokens", "50k"])
        .arg(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("src/lib.rs"));
}
