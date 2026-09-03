//! The plugin's version pin must agree with the crate it ships beside.
//!
//! ADR-0037 chose in-repo plugin distribution over a separate plugin repo *because* it
//! "eliminates an entire class of version-skew bugs", accepting in exchange that "the plugin
//! cannot version independently of the CLI". At `v3.1.4` the plugin versioned independently
//! anyway — `Cargo.toml` moved and three other declarations did not — so the cost of Option A
//! was paid and its benefit was not received (#91).
//!
//! Nothing enforced it. The release checklist named the step in prose
//! (`plans/2026-07-11-cxpak-ui-overhaul-PLAN.md:317`), no test asserted it, and the result was a
//! published release whose `ensure-cxpak` rejects its own binary — taking the MCP server, four
//! commands, two skills and both git hook wrappers with it.
//!
//! The three paths below are enumerated from ADR-0037 and that checklist, NOT from a scan of the
//! tree. A completeness check that discovers its own denominator only ever confirms that the
//! files it found agree with each other; it cannot notice a fourth declaration nobody listed.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The authority. Resolved by cargo from `Cargo.toml`, so this cannot drift from the crate.
const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// (path, the text immediately preceding the version literal, which is then read to the next `"`)
const SITES: [(&str, &str); 3] = [
    // ADR-0037, Decision: "held in lockstep with the crate across plugin/.claude-plugin/plugin.json
    // and .claude-plugin/marketplace.json".
    ("plugin/.claude-plugin/plugin.json", "\"version\": \""),
    (".claude-plugin/marketplace.json", "\"version\": \""),
    // The rollout checklist's fourth file: the resolver's own pin.
    ("plugin/lib/ensure-cxpak", "REQUIRED_VERSION=\""),
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn declared(path: &Path, prefix: &str) -> String {
    let body = std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "{} is unreadable ({e}) — this check cannot answer, and a \
                                    version site that moved is exactly what it exists to catch",
            path.display()
        )
    });
    let hits = body.matches(prefix).count();
    assert_eq!(
        hits,
        1,
        "{} contains {hits} occurrences of {prefix:?}, expected exactly 1. With none, the \
         declaration moved or was renamed and this check would silently pass over it; with \
         several, it is ambiguous which one ships.",
        path.display()
    );
    let rest = &body[body.find(prefix).unwrap() + prefix.len()..];
    rest[..rest
        .find('"')
        .unwrap_or_else(|| panic!("{}: unterminated version literal", path.display()))]
        .to_string()
}

#[test]
fn every_declaration_agrees_with_the_crate_version() {
    let root = repo_root();
    let mut wrong = Vec::new();
    for (rel, prefix) in SITES {
        let got = declared(&root.join(rel), prefix);
        if got != CRATE_VERSION {
            wrong.push(format!("  {rel}: {got} (Cargo.toml says {CRATE_VERSION})"));
        }
    }
    assert!(
        wrong.is_empty(),
        "the plugin is versioned independently of the CLI, which is the one thing ADR-0037 \
         chose in-repo distribution to prevent:\n{}\n\nA release cut here ships a resolver that \
         rejects its own binary.",
        wrong.join("\n")
    );
}

/// The consumer-side proof. `ensure-cxpak` is the single binary resolver (ADR-0033); every plugin
/// surface goes through it. Handed a cxpak that reports THIS crate's version — which is what a
/// user gets from `brew install cxpak` or `cargo install cxpak` the moment a release is cut — it
/// must resolve. At `v3.1.4` it did not, and the string check above says which file to fix while
/// this one says what the user experiences.
#[test]
fn ensure_cxpak_resolves_a_binary_reporting_the_crate_version() {
    let root = repo_root();
    let shim_dir = std::env::temp_dir().join(format!("cxpak-lockstep-{}", std::process::id()));
    std::fs::create_dir_all(&shim_dir).expect("create shim dir");
    let shim = shim_dir.join("cxpak");
    std::fs::write(
        &shim,
        format!("#!/bin/sh\necho \"cxpak {CRATE_VERSION}\"\n"),
    )
    .expect("write shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // `/usr/bin:/bin` only, so `brew` is absent and the auto-install step cannot fire and change
    // the answer — the resolution being tested is the version comparison, nothing else.
    let out = Command::new("bash")
        .arg(root.join("plugin/lib/ensure-cxpak"))
        .env("PATH", format!("{}:/usr/bin:/bin", shim_dir.display()))
        .output()
        .expect("run ensure-cxpak");
    let _ = std::fs::remove_dir_all(&shim_dir);

    assert!(
        out.status.success(),
        "ensure-cxpak rejected a cxpak reporting {CRATE_VERSION}, the version this crate \
         publishes. Both remedies it prints install exactly that version, so a user following \
         them cannot recover.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        shim.display().to_string(),
        "resolved a different binary than the one on PATH"
    );
}
