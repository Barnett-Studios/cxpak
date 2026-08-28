//! Every published image must be able to open a bind-mounted repository.
//!
//! libgit2 refuses a repository whose owner differs from the running uid. The images run as
//! uid 10001 and a host repo bind-mounted at `/repo` never belongs to 10001, so `cxpak diff`
//! failed with `Owner (-36)` on every documented container invocation (cxpak#60). `overview`
//! does not open the repo through libgit2, which is why the README's headline command worked
//! and this went unnoticed for the life of the image.
//!
//! Measured on the published `ghcr.io/barnett-studios/cxpak:3.1.4`, same fixture, same mount:
//!
//! ```text
//! base + `printf '[safe]\n\tdirectory = *\n' > /etc/gitconfig`  -> exit 0, prints the diff
//! base + an otherwise identical rebuild, no /etc/gitconfig      -> exit 1, Owner (-36)
//! ```
//!
//! libgit2 reads `/etc/gitconfig` itself, so the exemption needs no `git` binary — the probe
//! image had none.
//!
//! This is a static audit, not a container run: cxpak's CI has no Docker job, and adding one
//! to build this crate from source would cost far more than the regression it guards. What it
//! catches is the line being dropped from one image while the others keep it, or a new image
//! shipping without it — which is the realistic regression, since the Dockerfiles are edited
//! separately and only one of them is exercised by any release path at a time.

use std::path::{Path, PathBuf};

/// Directories a Dockerfile under them would not be a published image: build output and git
/// internals. Everything else in the tree is in scope.
const NOT_SOURCE: &[&str] = &["target", ".git"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every Dockerfile in the repo, discovered rather than listed.
///
/// This was `const IMAGES: &[&str] = &["Dockerfile", "Dockerfile.dist",
/// "Dockerfile.standalone"]`. That is a hand-written denominator for a property about *every*
/// image, and it fails exactly where it matters: a fourth root Dockerfile with no `[safe]`
/// exemption was green across the whole suite, because a name not in the list is a name
/// nothing checks. Review caught it (cxpak#60).
///
/// The three current entries all run as uid 10001 and all mount a host repo at `/repo`:
/// `Dockerfile` builds from source, `Dockerfile.dist` wraps a CI-built binary,
/// `Dockerfile.standalone` downloads a release tarball. A future Dockerfile that genuinely
/// does not open a mounted repo will fail here and needs an exemption written down with its
/// reason — the scan must not stop seeing it quietly.
fn dockerfiles() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => panic!("read_dir {}: {e}", dir.display()),
        };
        for entry in entries {
            let path = entry.expect("dir entry").path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if path.is_dir() {
                if !NOT_SOURCE.contains(&name) {
                    walk(&path, out);
                }
            } else if name.starts_with("Dockerfile") {
                out.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(&repo_root(), &mut found);
    found.sort();
    // Without this the two tests below are green on an empty walk — the failure mode a
    // discovered denominator has and a hard-coded one does not.
    assert!(
        !found.is_empty(),
        "no Dockerfile found under {} — the scan is reading nothing, so it proves nothing",
        repo_root().display()
    );
    found
}

/// Path relative to the repo root. The basename alone is ambiguous the moment the scan
/// reaches a subdirectory — two images both reported as `Dockerfile` name neither.
fn name_of(path: &Path) -> String {
    path.strip_prefix(repo_root())
        .unwrap_or(path)
        .display()
        .to_string()
}

#[test]
fn every_image_exempts_the_mounted_repo_from_ownership_validation() {
    for path in dockerfiles() {
        let name = name_of(&path);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert!(
            text.contains("/etc/gitconfig"),
            "{name} does not write /etc/gitconfig — an image built from it cannot open a \
             bind-mounted repo, and `cxpak diff` exits 1 with Owner (-36) (cxpak#60)"
        );
        assert!(
            text.contains("safe]") && text.contains("directory = *"),
            "{name} writes /etc/gitconfig without a `[safe] directory = *` entry"
        );
    }
}

#[test]
fn the_exemption_is_written_before_the_image_drops_to_uid_10001() {
    // The bound. `> /etc/gitconfig` after `USER 10001` is a permission error at build time on
    // some daemons and a silently unwritten file on others, so position is the property — the
    // same shape as commitward's root-refusal guard, where the characters existing somewhere
    // in the file was not the thing that mattered.
    for path in dockerfiles() {
        let name = name_of(&path);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));

        let gitconfig = text
            .find("> /etc/gitconfig")
            .unwrap_or_else(|| panic!("{name}: no /etc/gitconfig write to position"));
        let user = text
            .find("\nUSER ")
            .unwrap_or_else(|| panic!("{name}: no USER instruction — the image runs as root?"));

        assert!(
            gitconfig < user,
            "{name} writes /etc/gitconfig after dropping to a non-root USER, so the file is \
             not written and the ownership refusal returns"
        );
    }
}
