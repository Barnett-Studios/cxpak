//! Cross-process byte-identity of the default (Active / RRF) auto_context
//! selection.
//!
//! The tie-break regression tests (`determinism_ties.rs`) run in-process, where
//! Rust's per-process `HashMap` seed is fixed for the life of the process — so
//! they structurally cannot observe a `HashMap`-iteration-order leak into the
//! ranking. Since 3.0.0 the default relevance mode is `Active` (RRF fusion,
//! ADR-0187); this test runs the selection in two *separate* processes (each
//! with a distinct SipHash seed) and asserts the packed file order is
//! byte-identical. A future leak of `HashMap` order into the default path fails
//! here rather than shipping green. (ADR-0151 is the sibling harness for the SPA
//! render path; this is its analogue for the relevance scorer.)

use std::path::PathBuf;
use std::process::Command;

const CHILD_ENV: &str = "CXPAK_AC_DETERMINISM_CHILD";
const BEGIN: &str = "<<<AC-SELECTION-BEGIN>>>";
const END: &str = "<<<AC-SELECTION-END>>>";
const TASK: &str = "authenticate user session token";
const TEST_NAME: &str = "active_auto_context_selection_is_byte_identical_across_processes";

fn load_fixture_index() -> cxpak::index::CodebaseIndex {
    let fixture_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/determinism_repo");
    // build_index needs a valid git repo; the fixture ships without a `.git`
    // (created at test time, hermetic — same rationale as spa_determinism.rs).
    if !fixture_root.join(".git").exists() {
        git2::Repository::init(&fixture_root).expect("git2 init fixture");
    }
    cxpak::commands::serve::build_index(&fixture_root).expect("fixture index builds")
}

/// Deterministic serialization of the packed selection. Line *order* is the
/// determinism invariant; paths are exact so any reorder is visible, and the
/// score is rounded to keep same-machine f64 formatting stable.
fn active_selection() -> String {
    let index = load_fixture_index();
    let opts = cxpak::auto_context::AutoContextOpts {
        tokens: 8000,
        focus: None,
        include_tests: true,
        include_blast_radius: true,
        mode: "full".to_string(),
        cost_model: None,
    };
    let result = cxpak::auto_context::auto_context_with_mode(
        TASK,
        &index,
        &opts,
        cxpak::relevance::RelevanceMode::Active,
    );
    let s = &result.sections;
    let mut out = String::new();
    for (label, section) in [
        ("target", &s.target_files),
        ("test", &s.test_files),
        ("schema", &s.schema_context),
    ] {
        for f in &section.files {
            out.push_str(&format!(
                "{label}\t{}\t{:.6}\t{}\t{}\n",
                f.path, f.score, f.detail_level, f.tokens
            ));
        }
    }
    out
}

#[test]
fn active_auto_context_selection_is_byte_identical_across_processes() {
    if std::env::var(CHILD_ENV).is_ok() {
        // Child: emit the selection between markers and exit before libtest
        // prints its own summary lines. The markers isolate our payload from
        // libtest's "running 1 test" chatter on the same stdout.
        use std::io::Write;
        print!("{BEGIN}{}{END}", active_selection());
        std::io::stdout().flush().ok();
        std::process::exit(0);
    }

    let exe = std::env::current_exe().expect("current_exe");
    let run_once = || -> String {
        let output = Command::new(&exe)
            .args([TEST_NAME, "--exact", "--nocapture"])
            .env(CHILD_ENV, "1")
            .env("CARGO_MANIFEST_DIR", env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("spawn child test process");
        assert!(
            output.status.success(),
            "child process failed: status={:?}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("child stdout is utf8");
        let start = stdout
            .find(BEGIN)
            .expect("BEGIN marker missing from child output")
            + BEGIN.len();
        let end = stdout
            .find(END)
            .expect("END marker missing from child output");
        stdout[start..end].to_string()
    };

    let first = run_once();
    let second = run_once();
    assert!(
        !first.is_empty(),
        "selection was empty — fixture produced no packed files, test proves nothing"
    );
    assert_eq!(
        first, second,
        "Active auto_context selection differs across processes — a nondeterminism \
         (likely HashMap iteration order) leaked into the default relevance path"
    );
}
