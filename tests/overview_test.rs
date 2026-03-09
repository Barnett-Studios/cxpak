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

/// Create a temp git repo with enough files to exceed a tiny token budget.
fn make_large_temp_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();

    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    // Create 20 files to exceed a 500-token budget
    for i in 0..20 {
        let content = format!(
            "pub fn function_{i}(x: i32) -> i32 {{\n    x + {i}\n}}\n\npub fn helper_{i}() -> String {{\n    String::from(\"hello_{i}\")\n}}\n"
        );
        std::fs::write(src_dir.join(format!("mod_{i}.rs")), &content).unwrap();
    }

    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"large\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("README.md"),
        "# Large Test Project\nThis is a readme with some content for testing.\n",
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
fn test_pack_mode_creates_cxpak_dir() {
    let repo = make_large_temp_repo();
    let out_dir = TempDir::new().unwrap();
    let out_file = out_dir.path().join("overview.md");

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "overview",
            "--tokens",
            "500",
            "--out",
            out_file.to_str().unwrap(),
        ])
        .arg(repo.path())
        .assert()
        .success();

    let cxpak_dir = repo.path().join(".cxpak");
    assert!(cxpak_dir.exists(), ".cxpak/ directory should be created");

    // At least one detail file should exist
    let has_detail_files = cxpak_dir.join("modules.md").exists()
        || cxpak_dir.join("signatures.md").exists()
        || cxpak_dir.join("tree.md").exists();
    assert!(
        has_detail_files,
        "at least one detail file should exist in .cxpak/"
    );
}

#[test]
fn test_pack_mode_overview_has_pointers() {
    let repo = make_large_temp_repo();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", "--tokens", "500"])
        .arg(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(".cxpak/"));
}

#[test]
fn test_pack_mode_gitignore_updated() {
    let repo = make_large_temp_repo();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", "--tokens", "500"])
        .arg(repo.path())
        .assert()
        .success();

    let gitignore = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    assert!(gitignore.contains(".cxpak/"));
}

#[test]
fn test_single_file_mode_no_cxpak_dir() {
    let repo = make_temp_repo();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", "--tokens", "50k"])
        .arg(repo.path())
        .assert()
        .success();

    let cxpak_dir = repo.path().join(".cxpak");
    assert!(
        !cxpak_dir.exists(),
        ".cxpak/ should NOT exist when repo fits in budget"
    );
}

#[test]
fn test_overview_markdown_output() {
    let repo = make_temp_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
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
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", "--tokens", "50k", "--format", "json"])
        .arg(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"metadata\""));
}

#[test]
fn test_overview_xml_output() {
    let repo = make_temp_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
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

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "overview",
            "--tokens",
            "50k",
            "--out",
            out_file.to_str().unwrap(),
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
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", "--tokens", "500"])
        .arg(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("omitted"));
}

#[test]
fn test_overview_not_git_repo() {
    let dir = TempDir::new().unwrap();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", "--tokens", "50k"])
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a git repository"));
}

#[test]
fn test_overview_verbose_output() {
    let repo = make_temp_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
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
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", "--tokens", "50k"])
        .arg(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("main"));
}

#[test]
fn test_pack_mode_metadata_has_budget_and_detail_info() {
    let repo = make_large_temp_repo();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", "--tokens", "500"])
        .arg(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Token budget"))
        .stdout(predicate::str::contains("Detail files"));
}

#[test]
fn test_single_file_mode_no_detail_info() {
    let repo = make_temp_repo();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", "--tokens", "50k"])
        .arg(repo.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Detail files").not());
}

#[test]
fn test_stale_cxpak_cleaned_on_single_file_mode() {
    let repo = make_temp_repo();

    // Create stale .cxpak/ directory with a leftover file
    let cxpak_dir = repo.path().join(".cxpak");
    std::fs::create_dir_all(&cxpak_dir).unwrap();
    std::fs::write(cxpak_dir.join("stale.md"), "stale content").unwrap();

    // Run in single-file mode (repo fits in budget)
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", "--tokens", "50k"])
        .arg(repo.path())
        .assert()
        .success();

    // .cxpak/ should be cleaned up
    assert!(
        !cxpak_dir.exists(),
        "stale .cxpak/ should be removed in single-file mode"
    );
}

#[test]
fn test_stale_cxpak_cleaned_on_pack_mode() {
    let repo = make_large_temp_repo();

    // Create stale file in .cxpak/
    let cxpak_dir = repo.path().join(".cxpak");
    std::fs::create_dir_all(&cxpak_dir).unwrap();
    std::fs::write(cxpak_dir.join("stale.md"), "stale content").unwrap();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", "--tokens", "500"])
        .arg(repo.path())
        .assert()
        .success();

    // stale.md should not exist (dir was cleaned before new files written)
    assert!(
        !cxpak_dir.join("stale.md").exists(),
        "stale files should be cleaned"
    );
    // But new detail files should exist
    assert!(
        cxpak_dir.exists(),
        ".cxpak/ should be recreated with fresh files"
    );
}
