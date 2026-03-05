use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Create a temp git repo with a Rust source file for integration testing.
fn make_temp_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();

    // Create a source file
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();

    // Create Cargo.toml
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    // Create README
    std::fs::write(dir.path().join("README.md"), "# Test Project\n").unwrap();

    // Stage and commit
    let mut index = repo.index().unwrap();
    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None).unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::now("Test", "test@test.com").unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[]).unwrap();

    dir
}

#[test]
fn test_overview_markdown_output() {
    let repo = make_temp_repo();
    Command::cargo_bin("cxpak")
        .unwrap()
        .args(["overview", "--tokens", "50k"])
        .arg(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("## Project Metadata"))
        .stdout(predicate::str::contains("## Directory Tree"));
}

#[test]
fn test_overview_json_output() {
    let repo = make_temp_repo();
    Command::cargo_bin("cxpak")
        .unwrap()
        .args(["overview", "--tokens", "50k", "--format", "json"])
        .arg(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"metadata\""));
}

#[test]
fn test_overview_xml_output() {
    let repo = make_temp_repo();
    Command::cargo_bin("cxpak")
        .unwrap()
        .args(["overview", "--tokens", "50k", "--format", "xml"])
        .arg(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("<cxpak>"));
}

#[test]
fn test_overview_out_flag() {
    let repo = make_temp_repo();
    let out_dir = TempDir::new().unwrap();
    let out_file = out_dir.path().join("output.md");

    Command::cargo_bin("cxpak")
        .unwrap()
        .args([
            "overview",
            "--tokens", "50k",
            "--out", out_file.to_str().unwrap(),
        ])
        .arg(repo.path())
        .assert()
        .success();

    let content = std::fs::read_to_string(&out_file).unwrap();
    assert!(content.contains("## Project Metadata"));
}

#[test]
fn test_overview_small_budget_shows_omission_markers() {
    let repo = make_temp_repo();
    Command::cargo_bin("cxpak")
        .unwrap()
        .args(["overview", "--tokens", "500"])
        .arg(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("omitted"));
}

#[test]
fn test_overview_not_git_repo() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("cxpak")
        .unwrap()
        .args(["overview", "--tokens", "50k"])
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a git repository"));
}

#[test]
fn test_trace_not_yet_implemented() {
    Command::cargo_bin("cxpak")
        .unwrap()
        .args(["trace", "--tokens", "50k", "main"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_overview_verbose_output() {
    let repo = make_temp_repo();
    Command::cargo_bin("cxpak")
        .unwrap()
        .args(["overview", "--tokens", "50k", "--verbose"])
        .arg(repo.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("cxpak: scanning"))
        .stderr(predicate::str::contains("cxpak: found"))
        .stderr(predicate::str::contains("cxpak: parsed"));
}

#[test]
fn test_overview_contains_rust_symbols() {
    let repo = make_temp_repo();
    Command::cargo_bin("cxpak")
        .unwrap()
        .args(["overview", "--tokens", "50k"])
        .arg(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("main"));
}
