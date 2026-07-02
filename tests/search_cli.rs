//! CLI surface for the C1 retrieval capability: `cxpak search` (search /
//! references / expand) over a real temp repo. Exercises `commands::search::run`
//! end-to-end and proves the CLI returns the same index-derived JSON the core
//! produces (ADR-0180).

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Temp git repo: `main.rs` (with `run_search`, importing lib) and `lib.rs`
/// (with `search` + `helper`), so all three retrieval ops have real data.
fn make_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();

    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("lib.rs"),
        "pub fn search() {}\npub fn helper() {}\n",
    )
    .unwrap();
    std::fs::write(
        src_dir.join("main.rs"),
        "use crate::lib;\nfn run_search() {\n    helper();\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"search_cli_test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("README.md"), "# Search CLI Test\n").unwrap();

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
fn cli_search_finds_symbol() {
    let repo = make_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["search", "search"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"symbol\": \"search\""))
        .stdout(predicate::str::contains("src/lib.rs"));
}

#[test]
fn cli_search_references_op() {
    let repo = make_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["search", "--op", "references", "helper"])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"symbol\": \"helper\""))
        .stdout(predicate::str::contains("src/lib.rs"));
}

#[test]
fn cli_search_expand_op() {
    let repo = make_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "search",
            "--op",
            "expand",
            "--seeds",
            "src/main.rs",
            "--depth",
            "2",
        ])
        .current_dir(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"nodes\""))
        .stdout(predicate::str::contains("src/main.rs"));
}

#[test]
fn cli_search_unknown_op_errors() {
    let repo = make_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["search", "--op", "frobnicate", "x"])
        .current_dir(repo.path())
        .assert()
        .failure();
}
