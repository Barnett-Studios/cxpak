//! Every GitHub Action this repo runs must be pinned to a commit, and say what it was pinned from.
//!
//! cxpak advertises cosign signatures, an SBOM and provenance as a trust story. Those attest the
//! output of the build; they say nothing about the toolchain that produced it. With
//! `uses: owner/repo@v3` the ref is MUTABLE — the owner can retag it, and a retagged or
//! compromised action injects code *before* signing, so the signature attests a poisoned build
//! faithfully. The posture was inverted relative to what it advertised (cxpak#31).
//!
//! THE COMMENT IS PART OF THE PIN. A bare 40-hex string is unauditable: nobody can tell whether
//! `de0fac2e…` is `v6.0.2` or something a pull request quietly substituted, and a reviewer who
//! cannot check a pin does not review it. Each line carries `# <ref>` naming what it was resolved
//! from, so the pin can be re-derived with one API call.
//!
//! ENUMERATED FROM THE DIRECTORY, deliberately. A hand-written list of workflow files is a list
//! that a new workflow is not on — and a new workflow is exactly where an unpinned action would
//! next appear. The refusals below exist because "no workflow files found" and "no actions found"
//! would otherwise both read as a pass.

use std::path::{Path, PathBuf};

fn workflows() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| {
            panic!(
                "{} is unreadable ({e}) — this check cannot answer",
                dir.display()
            )
        })
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            matches!(
                p.extension().and_then(|s| s.to_str()),
                Some("yml") | Some("yaml")
            )
        })
        .collect();
    out.sort();
    assert!(
        !out.is_empty(),
        "no workflow files under {} — a check that finds nothing to check must not report success",
        dir.display()
    );
    out
}

/// `(file, line number, the `uses:` value, the rest of the line after it)`
fn uses_lines() -> Vec<(String, usize, String, String)> {
    let mut out = Vec::new();
    for wf in workflows() {
        let name = wf.file_name().unwrap().to_string_lossy().to_string();
        let body = std::fs::read_to_string(&wf).expect("read workflow");
        for (i, line) in body.lines().enumerate() {
            let Some(rest) = line.split_once("uses:") else {
                continue;
            };
            let mut parts = rest.1.trim().splitn(2, char::is_whitespace);
            let value = parts.next().unwrap_or("").to_string();
            let trailing = parts.next().unwrap_or("").trim().to_string();
            out.push((name.clone(), i + 1, value, trailing));
        }
    }
    assert!(
        !out.is_empty(),
        "no `uses:` found in any workflow — either the workflows stopped using actions, or this \
         parser stopped matching them. Both need a human; neither is a pass."
    );
    out
}

fn is_sha_pinned(value: &str) -> bool {
    match value.split_once('@') {
        Some((_, r)) => {
            r.len() == 40
                && r.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
        }
        None => false,
    }
}

#[test]
fn every_action_is_pinned_to_a_commit() {
    let mut bad = Vec::new();
    for (file, line, value, _) in uses_lines() {
        // A local action (`./.github/actions/x`) is this repository's own code at this commit,
        // and a docker:// reference is pinned by its own digest rules — neither is a mutable
        // third-party ref.
        if value.starts_with("./") || value.starts_with("docker://") {
            continue;
        }
        if !is_sha_pinned(&value) {
            bad.push(format!("  {file}:{line}: {value}"));
        }
    }
    assert!(
        bad.is_empty(),
        "these actions are on a MUTABLE ref. The owner can retag it, and the retagged code runs \
         before cosign signs anything — so the signature attests whatever it did.\n{}\n\nPin with \
         `uses: owner/repo@<40-char-sha>  # <ref>`; resolve with \
         `gh api repos/<owner>/<repo>/commits/<ref> --jq .sha`.",
        bad.join("\n")
    );
}

#[test]
fn every_pin_says_what_it_was_pinned_from() {
    let mut bare = Vec::new();
    for (file, line, value, trailing) in uses_lines() {
        if !is_sha_pinned(&value) {
            continue; // the test above owns that failure
        }
        if !trailing.starts_with('#') || trailing.trim_start_matches('#').trim().is_empty() {
            bare.push(format!("  {file}:{line}: {value}"));
        }
    }
    assert!(
        bare.is_empty(),
        "these pins are bare SHAs with no comment saying what they were pinned from. A reviewer \
         cannot tell a legitimate `v6.0.2` from a substituted commit, so an unauditable pin buys \
         immutability and gives up review:\n{}",
        bare.join("\n")
    );
}

#[test]
fn cargo_publish_runs_from_a_clean_tree() {
    // `--allow-dirty` tells cargo to package whatever is on disk, uncommitted changes included,
    // so what reaches crates.io need not correspond to any commit — under a workflow whose whole
    // claim is provenance. Measured on a clean checkout: `cargo package` succeeds without it.
    for wf in workflows() {
        let body = std::fs::read_to_string(&wf).expect("read workflow");
        assert!(
            !body.contains("--allow-dirty"),
            "{}: `--allow-dirty` publishes files that are in no commit, which is the opposite of \
             the provenance this workflow signs for",
            wf.display()
        );
    }
}
