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
fn test_overview_uses_default_tokens() {
    // Since v0.7.0, --tokens defaults to 50k so overview works without it
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", repo.path().to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn test_trace_requires_target() {
    // trace still requires a target argument even with default tokens
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

#[test]
fn test_overview_with_focus() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "overview",
            "--tokens",
            "50k",
            "--focus",
            "src",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}

#[test]
fn test_overview_with_focus_no_match() {
    // A focus path that matches nothing should still succeed (no boost applied).
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "overview",
            "--tokens",
            "50k",
            "--focus",
            "nonexistent/path",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}

#[test]
fn test_diff_with_focus() {
    let repo = make_test_repo();
    // Modify a file so diff has something to show.
    std::fs::write(
        repo.path().join("src/main.rs"),
        "fn main() { println!(\"focused\"); }\n",
    )
    .unwrap();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "diff",
            "--tokens",
            "50k",
            "--focus",
            "src",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}

#[test]
fn test_trace_with_focus() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "trace",
            "--tokens",
            "50k",
            "--focus",
            "src",
            "main",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}

#[test]
fn test_overview_with_timing() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "overview",
            "--tokens",
            "50k",
            "--timing",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("cxpak [timing]:"));
}

#[test]
fn test_timing_flag_accepted_by_diff() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "diff",
            "--tokens",
            "50k",
            "--timing",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}

#[test]
fn test_timing_flag_accepted_by_trace() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "trace",
            "--tokens",
            "50k",
            "--timing",
            "main",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}

#[test]
fn test_trace_timing_produces_output() {
    let repo = make_test_repo();
    let output = Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "trace",
            "--tokens",
            "50k",
            "--timing",
            "main",
            repo.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cxpak [timing]: scan"),
        "stderr should contain scan timing, got: {stderr}"
    );
    assert!(
        stderr.contains("cxpak [timing]: total"),
        "stderr should contain total timing, got: {stderr}"
    );
}

#[test]
fn test_diff_timing_produces_output() {
    let repo = make_test_repo();
    // Modify a file so diff has actual changes to process.
    std::fs::write(
        repo.path().join("src/main.rs"),
        "fn main() { println!(\"changed\"); }\n",
    )
    .unwrap();

    let output = Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "diff",
            "--tokens",
            "50k",
            "--timing",
            repo.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cxpak [timing]: scan"),
        "stderr should contain scan timing, got: {stderr}"
    );
    assert!(
        stderr.contains("cxpak [timing]: total"),
        "stderr should contain total timing, got: {stderr}"
    );
}

#[test]
fn test_trace_no_timing_by_default() {
    let repo = make_test_repo();
    let output = Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "trace",
            "--tokens",
            "50k",
            "main",
            repo.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("cxpak [timing]"),
        "no timing output expected without --timing flag, got: {stderr}"
    );
}

#[test]
fn test_trace_focus_produces_output() {
    let repo = make_test_repo();
    let output = Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "trace",
            "--tokens",
            "50k",
            "--focus",
            "src/",
            "main",
            repo.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.is_empty(),
        "trace with --focus should produce output"
    );
    assert!(
        stdout.contains("main"),
        "output should contain the traced symbol, got: {stdout}"
    );
}

#[test]
fn test_diff_focus_produces_output() {
    let repo = make_test_repo();
    std::fs::write(
        repo.path().join("src/main.rs"),
        "fn main() { println!(\"focused\"); }\n",
    )
    .unwrap();

    let output = Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "diff",
            "--tokens",
            "50k",
            "--focus",
            "src/",
            repo.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.is_empty(),
        "diff with --focus should produce output"
    );
}

#[test]
fn test_overview_tokens_zero_fails() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", "--tokens", "0", repo.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--tokens must be greater than 0"));
}

#[test]
fn test_diff_tokens_zero_fails() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["diff", "--tokens", "0", repo.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--tokens must be greater than 0"));
}

#[test]
fn test_trace_tokens_zero_fails() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "trace",
            "--tokens",
            "0",
            "main",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--tokens must be greater than 0"));
}

#[test]
fn test_overview_tokens_invalid_fails() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", "--tokens", "abc", repo.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid token count"));
}

#[test]
fn test_diff_tokens_invalid_fails() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["diff", "--tokens", "abc", repo.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid token count"));
}

#[test]
fn test_trace_tokens_invalid_fails() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "trace",
            "--tokens",
            "abc",
            "main",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid token count"));
}

#[test]
fn test_overview_tiny_budget_triggers_pack_mode() {
    let repo = make_test_repo();
    // Add multiple files so the index has enough content
    std::fs::create_dir_all(repo.path().join("src/lib")).unwrap();
    std::fs::write(
        repo.path().join("src/lib/utils.rs"),
        "pub fn helper() -> i32 { 42 }\npub fn another() -> String { String::new() }\n",
    )
    .unwrap();
    std::fs::write(
        repo.path().join("src/lib/models.rs"),
        "pub struct User { pub name: String }\npub struct Item { pub id: u64 }\n",
    )
    .unwrap();

    // Very tiny budget should trigger pack mode (detail files in .cxpak/)
    let output = Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["overview", "--tokens", "1", repo.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // With a 1-token budget, pack mode should activate and mention detail files
    // or at minimum the output should be very short
    assert!(
        stdout.len() < 5000,
        "tiny budget should produce small output"
    );
}

#[test]
fn test_overview_verbose_and_timing_combined() {
    let repo = make_test_repo();
    let output = Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "overview",
            "--tokens",
            "10k",
            "--verbose",
            "--timing",
            repo.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("scanning"),
        "verbose output should contain scanning, got: {stderr}"
    );
    assert!(
        stderr.contains("cxpak [timing]:"),
        "timing output should appear, got: {stderr}"
    );
}

#[test]
fn test_diff_verbose() {
    let repo = make_test_repo();
    std::fs::write(
        repo.path().join("src/main.rs"),
        "fn main() { println!(\"verbose diff\"); }\n",
    )
    .unwrap();

    let output = Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "diff",
            "--tokens",
            "10k",
            "--verbose",
            repo.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("scanning"),
        "diff --verbose should show scanning, got: {stderr}"
    );
}

#[test]
fn test_trace_verbose() {
    let repo = make_test_repo();
    let output = Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "trace",
            "--tokens",
            "10k",
            "--verbose",
            "main",
            repo.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("scanning"),
        "trace --verbose should show scanning, got: {stderr}"
    );
}

#[test]
fn test_overview_json_format_detail_files() {
    let repo = make_test_repo();
    // Add more files so pack mode might be triggered with tiny budget
    std::fs::create_dir_all(repo.path().join("src/extra")).unwrap();
    for i in 0..5 {
        std::fs::write(
            repo.path().join(format!("src/extra/mod{i}.rs")),
            format!("pub fn func{i}() -> i32 {{ {i} }}\n"),
        )
        .unwrap();
    }

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "overview",
            "--tokens",
            "1",
            "--format",
            "json",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}

#[test]
fn test_overview_xml_format_detail_files() {
    let repo = make_test_repo();
    std::fs::create_dir_all(repo.path().join("src/extra")).unwrap();
    for i in 0..5 {
        std::fs::write(
            repo.path().join(format!("src/extra/mod{i}.rs")),
            format!("pub fn func{i}() -> i32 {{ {i} }}\n"),
        )
        .unwrap();
    }

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "overview",
            "--tokens",
            "1",
            "--format",
            "xml",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}

#[test]
fn test_trace_all_flag() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "trace",
            "--tokens",
            "10k",
            "--all",
            "main",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}

#[test]
fn test_diff_json_format() {
    let repo = make_test_repo();
    std::fs::write(
        repo.path().join("src/main.rs"),
        "fn main() { println!(\"json diff\"); }\n",
    )
    .unwrap();

    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "diff",
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
fn test_trace_json_format() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "trace",
            "--tokens",
            "10k",
            "--format",
            "json",
            "main",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("{"));
}

#[test]
fn test_trace_xml_format() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "trace",
            "--tokens",
            "10k",
            "--format",
            "xml",
            "main",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("<cxpak"));
}

#[test]
fn test_diff_invalid_since_fails() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "diff",
            "--since",
            "not_a_valid_time",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .failure();
}

/// A repo with a real cross-file edge (`src/main.rs` `use crate::`-imports
/// `src/helper_mod.rs`), so `graph nodes`/`subgraph` have something to
/// enumerate. `make_test_repo`'s `main.rs` has no imports at all.
fn make_graph_test_repo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let sig = git2::Signature::now("Test", "t@t.com").unwrap();

    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "mod helper_mod;\nuse crate::helper_mod::helper;\nfn main() { helper(); }\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/helper_mod.rs"), "pub fn helper() {}\n").unwrap();
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

/// Issue #20 (ADR-0202): `graph nodes` enumerates every node id with no
/// `--id`/`--seeds`/`--from`/`--to` required — the bootstrap op for a fresh
/// consumer who doesn't yet know a valid id.
#[test]
fn graph_cli_nodes_op() {
    let repo = make_graph_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["graph", "nodes", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("src/main.rs"))
        .stdout(predicate::str::contains("src/helper_mod.rs"));
}

/// Negative-path regression: `node` must still require `--id` after the
/// `nodes` op is added (op stays a free-form positional; only `nodes` skips
/// the id requirement).
#[test]
fn graph_cli_node_still_requires_id() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["graph", "node", repo.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("id"));
}

/// Negative-path regression: same as above for `neighbors`.
#[test]
fn graph_cli_neighbors_still_requires_id() {
    let repo = make_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(["graph", "neighbors", repo.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("id"));
}

/// Issue #20 (ADR-0202): `subgraph` partitions a non-existent seed into
/// `unknown_seeds` instead of echoing it back as a node.
#[test]
fn graph_cli_subgraph_reports_unknown_seed() {
    let repo = make_graph_test_repo();
    Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args([
            "graph",
            "subgraph",
            "--seeds",
            "totally/bogus.xyz",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"unknown_seeds\""))
        .stdout(predicate::str::contains("totally/bogus.xyz"))
        .stdout(
            predicate::str::contains("\"nodes\": []").or(predicate::str::contains("\"nodes\":[]")),
        );
}
