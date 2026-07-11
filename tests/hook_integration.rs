//! End-to-end integration for `cxpak hook` (Task B3, ADR-0178).
//!
//! Exercises the real binary + the real `git` CLI: `cxpak hook install` wires a
//! union merge driver into a throwaway temp repo, and an actual `git merge` of
//! two branches that each appended to the canonical artifact resolves
//! conflict-free via our driver. libgit2 does not invoke external merge drivers,
//! so this must go through the git CLI to prove the wiring fires.
//!
//! All git state lives in a `tempfile::TempDir`; user identity is passed via
//! `-c` flags so no global/user git config is touched.

use std::path::Path;
use std::process::Command;

/// Run `git` in `dir` with a throwaway identity (never writes user config).
fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    let mut full = vec![
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "-c",
        "commit.gpgsign=false",
    ];
    full.extend_from_slice(args);
    Command::new("git")
        .current_dir(dir)
        .args(&full)
        .output()
        .expect("git invocation failed")
}

fn git_ok(dir: &Path, args: &[&str]) {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn cxpak_path() -> std::path::PathBuf {
    assert_cmd::cargo_bin!("cxpak").to_path_buf()
}

#[test]
fn install_then_real_git_merge_union_resolves_artifact() {
    // Skip cleanly if git CLI is unavailable (the unit tests still cover logic).
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("git CLI unavailable — skipping e2e merge test");
        return;
    }

    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path();
    git_ok(p, &["init", "-q", "-b", "main"]);

    // Install the hook + merge driver via the real binary.
    assert_cmd::Command::new(cxpak_path())
        .args(["hook", "install", p.to_str().unwrap()])
        .assert()
        .success();

    // libgit2's install wrote `cxpak hook merge-driver %O %A %B`, which relies on
    // `cxpak` being on PATH. In-test the binary is at the cargo target dir, so
    // repoint the local driver config at the absolute binary path.
    let driver = format!("{} hook merge-driver %O %A %B", cxpak_path().display());
    git_ok(p, &["config", "merge.cxpak-union.driver", &driver]);

    let artifact = p.join(".cxpak/graph.edges");
    std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();

    // Base commit: artifact with one edge.
    std::fs::write(&artifact, "a.rs\tb.rs\timport\textracted\n").unwrap();
    git_ok(p, &["add", "-A"]);
    git_ok(p, &["commit", "-q", "-m", "base"]);

    // Branch ours: append edge X.
    git_ok(p, &["checkout", "-q", "-b", "ours"]);
    std::fs::write(
        &artifact,
        "a.rs\tb.rs\timport\textracted\na.rs\tx.rs\timport\textracted\n",
    )
    .unwrap();
    git_ok(p, &["add", "-A"]);
    git_ok(p, &["commit", "-q", "-m", "ours adds x"]);

    // Branch theirs (from base): append edge Y.
    git_ok(p, &["checkout", "-q", "main"]);
    git_ok(p, &["checkout", "-q", "-b", "theirs"]);
    std::fs::write(
        &artifact,
        "a.rs\tb.rs\timport\textracted\na.rs\ty.rs\timport\textracted\n",
    )
    .unwrap();
    git_ok(p, &["add", "-A"]);
    git_ok(p, &["commit", "-q", "-m", "theirs adds y"]);

    // Merge theirs into ours — the union driver must resolve cleanly.
    git_ok(p, &["checkout", "-q", "ours"]);
    let merge = git(p, &["merge", "--no-edit", "theirs"]);
    assert!(
        merge.status.success(),
        "merge must succeed via union driver: {}",
        String::from_utf8_lossy(&merge.stderr)
    );

    let merged = std::fs::read_to_string(&artifact).unwrap();
    assert!(
        merged.contains("a.rs\tx.rs\timport\textracted"),
        "edge X kept"
    );
    assert!(
        merged.contains("a.rs\ty.rs\timport\textracted"),
        "edge Y kept"
    );
    assert!(
        merged.contains("a.rs\tb.rs\timport\textracted"),
        "base kept"
    );
    assert!(!merged.contains("<<<<<<<"), "no conflict markers");
    assert!(!merged.contains(">>>>>>>"), "no conflict markers");

    // Deterministic canonical order (sorted).
    assert_eq!(
        merged,
        "a.rs\tb.rs\timport\textracted\n\
         a.rs\tx.rs\timport\textracted\n\
         a.rs\ty.rs\timport\textracted\n"
    );
}

#[test]
fn post_commit_subcommand_writes_artifact_end_to_end() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path();
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    git_ok(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a.rs"), "use crate::b;\npub fn a() {}\n").unwrap();
    std::fs::write(p.join("b.rs"), "pub fn b() {}\n").unwrap();
    git_ok(p, &["add", "-A"]);
    git_ok(p, &["commit", "-q", "-m", "init"]);

    assert_cmd::Command::new(cxpak_path())
        .args(["hook", "post-commit", p.to_str().unwrap()])
        .assert()
        .success();

    assert!(
        p.join(".cxpak/graph.edges").exists(),
        "post-commit must write the canonical artifact"
    );
}
