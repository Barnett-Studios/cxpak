use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn test_help_output() {
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("overview"))
        .stdout(predicate::str::contains("trace"));
}

#[test]
fn test_version_output() {
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn test_overview_requires_tokens() {
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--tokens"));
}

#[test]
fn test_trace_requires_tokens_and_target() {
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["trace"])
        .assert()
        .failure();
}

#[test]
fn test_tokens_parses_k_suffix() {
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", "--tokens", "50k"])
        .assert()
        .success();
}

fn make_test_repo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let sig = git2::Signature::now("Test", "t@t.com").unwrap();

    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() { println!(\"hello\"); }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
        .unwrap();

    dir
}

#[test]
fn test_overview_markdown() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", "--tokens", "10k", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Files:"));
}

#[test]
fn test_overview_json() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "overview",
            "--tokens",
            "10k",
            "--format",
            "json",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("{"));
}

#[test]
fn test_overview_xml() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "overview",
            "--tokens",
            "10k",
            "--format",
            "xml",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("<cxpak"));
}

#[test]
fn test_trace_not_found() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "trace",
            "--tokens",
            "10k",
            "nonexistent_symbol_xyz",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .failure();
}

#[test]
fn test_trace_found() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "trace",
            "--tokens",
            "10k",
            "main",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("main"));
}

#[test]
fn test_diff_no_changes() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["diff", "--tokens", "10k", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes detected"));
}

#[test]
fn test_diff_with_changes() {
    let repo = make_test_repo();
    std::fs::write(
        repo.path().join("src/main.rs"),
        "fn main() { println!(\"changed\"); }\n",
    )
    .unwrap();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["diff", "--tokens", "10k", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("src/main.rs"));
}

#[test]
fn test_clean_command() {
    let repo = make_test_repo();
    std::fs::create_dir_all(repo.path().join(".cxpak/cache")).unwrap();
    std::fs::write(repo.path().join(".cxpak/test.md"), "test").unwrap();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["clean", repo.path().to_str().unwrap()])
        .assert()
        .success();

    assert!(!repo.path().join(".cxpak").exists());
}

#[test]
fn test_overview_verbose() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "overview",
            "--tokens",
            "10k",
            "--verbose",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("scanning"));
}

#[test]
fn test_bad_subcommand() {
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["nonsense"])
        .assert()
        .failure();
}

#[test]
fn test_no_subcommand() {
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .assert()
        .failure();
}

#[test]
fn test_overview_with_output_file() {
    let repo = make_test_repo();
    let out = repo.path().join("output.md");
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "overview",
            "--tokens",
            "10k",
            "--out",
            out.to_str().unwrap(),
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    assert!(out.exists());
    let content = std::fs::read_to_string(&out).unwrap();
    assert!(content.contains("Files:"));
}
