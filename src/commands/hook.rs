//! `cxpak hook` — git integration: post-commit auto-rebuild + union-merge driver
//! (cxpak 3.0.0 Task B3, ADR-0178).
//!
//! Two product features that install into a USER repository (never globally,
//! never this repo's dev hooks):
//!
//! 1. **post-commit auto-rebuild** (`cxpak hook post-commit`): after a commit,
//!    regenerate cxpak's canonical, committable graph artifact
//!    (`.cxpak/graph.edges`) so it stays fresh. Cross-process incrementality
//!    comes from the persisted parse cache (`parse_with_cache`) — only the
//!    files the commit touched are re-parsed. **Best-effort: this never fails
//!    the user's git workflow** — every internal error is logged to stderr and
//!    the process still exits 0.
//!
//! 2. **union-merge driver** (`cxpak hook merge-driver`): a deterministic git
//!    merge driver that union-resolves a conflict in the canonical artifact, so
//!    two branches that each regenerated it merge cleanly. The result is the
//!    sorted, deduped union of both sides' edge lines — conflict-free and
//!    commutative.
//!
//! `cxpak hook install` wires both into the target repo idempotently.
//!
//! The canonical artifact is line-oriented (`<from>\t<to>\t<edge_type>\t<confidence>`,
//! one edge per line, sorted + deduped, `\n`-terminated). Line orientation is
//! what makes union-merge well-defined. It is derived from the same
//! [`DependencyGraph`] (`BTreeMap`/`BTreeSet`-backed) the whole pipeline uses,
//! so it is byte-deterministic regardless of whether it was produced by a full
//! build or the edge-delta hot path.

use crate::budget::counter::TokenCounter;
use crate::index::graph::DependencyGraph;
use crate::index::CodebaseIndex;
use crate::scanner::Scanner;
use std::collections::{BTreeSet, HashSet};
use std::error::Error;
use std::path::Path;

/// Repo-relative path of the canonical, committable graph artifact.
pub const ARTIFACT_REL: &str = ".cxpak/graph.edges";

/// Name of the union merge driver as registered in git config.
const MERGE_DRIVER_NAME: &str = "cxpak-union";

/// Marker fencing the cxpak block inside an existing `post-commit` hook so
/// install is idempotent and never clobbers a user's own hook body.
const HOOK_BEGIN: &str = "# >>> cxpak post-commit (managed) >>>";
const HOOK_END: &str = "# <<< cxpak post-commit (managed) <<<";

/// Env var that, when set to a non-empty value, disables the post-commit hook.
const NO_HOOK_ENV: &str = "CXPAK_NO_HOOK";

// ---------------------------------------------------------------------------
// Canonical artifact serialization
// ---------------------------------------------------------------------------

/// Serialize a [`DependencyGraph`] to the canonical line-oriented artifact.
///
/// One edge per line: `<from>\t<to>\t<edge_type>\t<confidence>`. Lines are
/// sorted and deduped; the output ends with a trailing newline (empty graph →
/// empty string). Tab-separated because `EdgeType::label()` for cross-language
/// edges contains a `:` but never a tab, and source paths never contain tabs.
///
/// Deterministic: the graph is `BTreeMap`/`BTreeSet`-backed and the lines are
/// additionally sorted here, so the bytes never depend on hash-map order.
pub fn serialize_graph_canonical(graph: &DependencyGraph) -> String {
    let mut lines: BTreeSet<String> = BTreeSet::new();
    for (from, targets) in &graph.edges {
        for edge in targets {
            let confidence = if edge.confidence.is_inferred() {
                "inferred"
            } else {
                "extracted"
            };
            lines.insert(format!(
                "{from}\t{}\t{}\t{confidence}",
                edge.target,
                edge.edge_type.label()
            ));
        }
    }
    if lines.is_empty() {
        return String::new();
    }
    let mut out = lines.into_iter().collect::<Vec<_>>().join("\n");
    out.push('\n');
    out
}

// ---------------------------------------------------------------------------
// Union merge
// ---------------------------------------------------------------------------

/// Deterministically union-merge two versions of the canonical artifact.
///
/// Returns the sorted, deduped union of every edge line present in either side.
/// Conflict markers and blank lines are dropped defensively. The common
/// ancestor is intentionally not consulted: for a regenerated derived artifact,
/// keeping every edge either branch produced is the safe, self-correcting
/// semantic (the next post-commit rebuild on the merge commit regenerates it
/// exactly). The result is byte-identical regardless of argument order
/// (commutative) and across runs (deterministic).
pub fn union_merge(ours: &str, theirs: &str) -> String {
    let mut lines: BTreeSet<String> = BTreeSet::new();
    for side in [ours, theirs] {
        for raw in side.split('\n') {
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            if line.is_empty() {
                continue;
            }
            // Defensive: never let an upstream conflict marker survive.
            if line.starts_with("<<<<<<<")
                || line.starts_with("=======")
                || line.starts_with(">>>>>>>")
                || line.starts_with("|||||||")
            {
                continue;
            }
            lines.insert(line.to_string());
        }
    }
    if lines.is_empty() {
        return String::new();
    }
    let mut out = lines.into_iter().collect::<Vec<_>>().join("\n");
    out.push('\n');
    out
}

// ---------------------------------------------------------------------------
// git diff: files the HEAD commit changed
// ---------------------------------------------------------------------------

/// Compute the (changed, removed) repo-relative path sets the HEAD commit
/// introduced, by diffing its tree against its first parent (or the empty tree
/// for the initial commit). LOCAL git only — no network.
pub fn changed_paths_in_head(
    repo: &git2::Repository,
) -> Result<(HashSet<String>, HashSet<String>), git2::Error> {
    let head_commit = repo.head()?.peel_to_commit()?;
    let head_tree = head_commit.tree()?;
    let parent_tree = match head_commit.parent(0) {
        Ok(parent) => Some(parent.tree()?),
        Err(_) => None, // initial commit: diff against the empty tree
    };
    let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&head_tree), None)?;

    let mut changed = HashSet::new();
    let mut removed = HashSet::new();
    for delta in diff.deltas() {
        match delta.status() {
            git2::Delta::Deleted => {
                if let Some(p) = delta.old_file().path() {
                    removed.insert(p.to_string_lossy().to_string());
                }
            }
            _ => {
                if let Some(p) = delta.new_file().path() {
                    changed.insert(p.to_string_lossy().to_string());
                }
            }
        }
    }
    Ok((changed, removed))
}

// ---------------------------------------------------------------------------
// Artifact build + write
// ---------------------------------------------------------------------------

/// Which rebuild path produced the artifact — surfaced so tests can assert the
/// SAFE edge-delta actually ran (or correctly fell back) rather than silently
/// doing a full rebuild.
///
/// `Delta` means the persisted derived cache's `base_commit` matched
/// `parent(HEAD)`, so the commit's changed/removed set was applied onto the
/// cached graph via `rebuild_graph_delta` + warm-started PageRank. `Full` means
/// no such validated base existed and the freshly built full-tree graph stood.
/// In BOTH cases the serialized artifact is byte-identical to a full rebuild —
/// delta is purely a speed optimization (ADR-0179).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RebuildKind {
    Delta,
    Full,
}

/// The full oid of `parent(HEAD)` as a hex string, or `None` for the initial
/// commit / unborn HEAD / any error. The edge-delta base validation compares
/// the cache's `base_commit` against this.
fn head_parent_oid(repo: &git2::Repository) -> Option<String> {
    let head_commit = repo.head().ok()?.peel_to_commit().ok()?;
    let parent = head_commit.parent(0).ok()?;
    Some(parent.id().to_string())
}

/// Build the canonical artifact string for the repo's current working tree,
/// using a SAFE base-SHA-validated edge-delta when possible (ADR-0179).
///
/// The working tree is parsed once (parse-cache accelerated) into a full HEAD
/// index. If the persisted derived cache was built at exactly `parent(HEAD)`
/// (`base_commit == parent(HEAD)`), the commit's changed/removed set is applied
/// onto the cached prior graph via `rebuild_graph_delta` + warm-started
/// PageRank — the SAME machinery `serve.rs`'s watcher uses — which is
/// bit-identical to a full rebuild (ADR-0166) while doing work proportional to
/// the change. In every other case (no prior, `base_commit` absent/`None`,
/// mismatched base — i.e. the cache is >1 commit behind / on another branch /
/// post-rebase — or a grammar/version bump) the freshly built full-tree graph
/// stands. Either way the artifact is byte-identical to a full rebuild.
///
/// A fully-valid derived cache stamped `base_commit = HEAD` is then persisted so
/// (a) a subsequent `cxpak overview` gets a warm cache hit and (b) the NEXT
/// commit's post-commit has a validated delta base.
fn build_artifact(repo_path: &Path) -> Result<(String, RebuildKind), Box<dyn Error>> {
    let counter = TokenCounter::new();
    let scanner = Scanner::new(repo_path)?;
    let files = scanner.scan()?;
    if files.is_empty() {
        return Ok((String::new(), RebuildKind::Full));
    }
    let (parse_results, content_map) =
        crate::cache::parse::parse_with_cache(&files, repo_path, &counter, false);
    let mut index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);

    // Resolve the commit's changed/removed set and parent oid. A repo that
    // won't open, or an unborn/parentless HEAD, yields no validated base → the
    // full-tree graph already in `index` stands.
    let repo = git2::Repository::open(repo_path).ok();
    let (changed, removed, parent_oid) = match repo.as_ref() {
        Some(r) => {
            let (changed, removed) = changed_paths_in_head(r).unwrap_or_default();
            (changed, removed, head_parent_oid(r))
        }
        None => (HashSet::new(), HashSet::new(), None),
    };

    let cache_dir = repo_path.join(crate::commands::serve::cache_namespace(repo_path, None));

    // The delta is applied from `diff(parent, HEAD)`, but the artifact is built
    // from the WORKING TREE (`index`). Those only agree when the working tree is
    // CLEAN vs HEAD: then `index.files == HEAD` tree and the commit's diff is
    // exactly the set differing from a clean parent base → delta == full. If the
    // tree is dirty (e.g. a partial commit left another file's edges changed but
    // NOT in this commit's diff), the delta would keep the base graph's edges for
    // that file while a full rebuild reflects the dirty content → the committed
    // artifact would silently diverge. A status error degrades to "dirty" (Full).
    let tree_clean = repo
        .as_ref()
        .map(crate::commands::serve::working_tree_clean)
        .unwrap_or(false);

    // THE CORRECTNESS INVARIANT: apply the delta ONLY IF the cached graph was
    // built at exactly `parent(HEAD)` AND the working tree is clean vs HEAD.
    // `load_for_delta` deliberately skips the content-fingerprint gate (the
    // post-commit tree's fingerprint necessarily differs from the base's);
    // base-SHA equality plus a clean tree is what makes the delta safe.
    let kind = match (
        parent_oid.as_deref(),
        crate::cache::DerivedCache::load_for_delta(&cache_dir),
    ) {
        (Some(parent), Some(prior))
            if tree_clean && prior.base_commit.as_deref() == Some(parent) =>
        {
            // Reset the graph to the validated prior base and drive it forward
            // by the commit's delta, warm-starting PageRank from the prior
            // ranks (bit-identical to a cold recompute — tests/parity.rs).
            // rebuild_graph_delta falls back internally to a full rebuild on a
            // structural (add/remove) or schema change, so the result always
            // equals a full rebuild of the HEAD tree.
            let prior_pagerank = prior.pagerank;
            index.graph = prior.graph;
            index.rebuild_graph_delta(&changed, &removed);
            index.pagerank = crate::intelligence::pagerank::compute_pagerank_seeded(
                &index.graph,
                0.85,
                100,
                &prior_pagerank,
            );
            RebuildKind::Delta
        }
        _ => RebuildKind::Full,
    };

    persist_derived_cache(repo_path, &cache_dir, &mut index);
    Ok((serialize_graph_canonical(&index.graph), kind))
}

/// Persist a fully-valid shared derived cache (ADR-0167 schema) stamped with the
/// current HEAD, so a later `overview` gets a warm hit and the next commit's
/// post-commit has a validated edge-delta base.
///
/// Conventions + co-changes are mined here so the cache is a SAFE fingerprint
/// hit for `overview` (a partial cache would serve empty conventions). The
/// fingerprint is computed exactly as `build_index` does (content + HEAD oid),
/// guaranteeing the hit. Best-effort: a write failure never fails the hook.
fn persist_derived_cache(repo_path: &Path, cache_dir: &Path, index: &mut CodebaseIndex) {
    index.conventions = crate::conventions::build_convention_profile(index, repo_path);
    index.co_changes = index.conventions.git_health.co_changes.clone();

    let head_oid = crate::commands::serve::git_head_oid(repo_path);
    let fp_files: Vec<(String, String)> = index
        .files
        .iter()
        .map(|f| (f.relative_path.clone(), f.content.clone()))
        .collect();
    let fingerprint = crate::cache::content_fingerprint(&fp_files, &head_oid);
    // Stamp `base_commit = Some(HEAD)` ONLY when the working tree is CLEAN vs
    // HEAD, so the stamp truthfully means "graph == committed tree at this SHA"
    // (ADR-0179). A partial commit can leave OTHER files dirty (their edges not
    // in this commit's diff); stamping HEAD then would let the next commit delta
    // onto a base that never matched the clean committed tree. An empty oid, a
    // dirty tree, or any status error → `None`.
    let base_commit = if head_oid.is_empty() {
        None
    } else {
        match git2::Repository::open(repo_path) {
            Ok(repo) if crate::commands::serve::working_tree_clean(&repo) => Some(head_oid),
            _ => None,
        }
    };

    let derived = crate::cache::DerivedCache::new(
        fingerprint,
        index.graph.clone(),
        index.call_graph.clone(),
        index.pagerank.clone(),
        index.conventions.clone(),
        index.co_changes.clone(),
        base_commit,
    );
    let _ = derived.save(cache_dir);
}

/// Atomically write the canonical artifact under `.cxpak/` (write-tmp-then-rename
/// so readers never see a partial file — same pattern as conventions export).
fn write_artifact(repo_path: &Path, content: &str) -> std::io::Result<()> {
    let cxpak_dir = repo_path.join(".cxpak");
    std::fs::create_dir_all(&cxpak_dir)?;
    let out = cxpak_dir.join("graph.edges");
    let tmp = out.with_extension("edges.tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, &out)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Subcommand entry points
// ---------------------------------------------------------------------------

/// `cxpak hook post-commit [path]` — regenerate the canonical artifact.
///
/// **Best-effort and non-fatal:** every failure path logs to stderr and returns
/// `Ok(())` so the surrounding git workflow is never broken. Skipped entirely
/// when `CXPAK_NO_HOOK` is set.
pub fn post_commit(path: &Path) -> Result<(), Box<dyn Error>> {
    if std::env::var_os(NO_HOOK_ENV).is_some_and(|v| !v.is_empty()) {
        return Ok(());
    }
    if let Err(e) = run_post_commit(path) {
        // Never propagate: a non-zero exit here could confuse git tooling.
        eprintln!("cxpak: post-commit rebuild skipped (best-effort): {e}");
    }
    Ok(())
}

/// Fallible core of [`post_commit`], kept separate so the public entry can
/// swallow every error.
fn run_post_commit(path: &Path) -> Result<(), Box<dyn Error>> {
    // Skip when the commit touched no files at all (e.g. an empty/amend with no
    // tree change) — cheap early-out. A diff failure (e.g. unborn HEAD) is not
    // fatal: fall through to a full rebuild.
    if let Ok(repo) = git2::Repository::open(path) {
        if let Ok((changed, removed)) = changed_paths_in_head(&repo) {
            if changed.is_empty() && removed.is_empty() {
                return Ok(());
            }
        }
    }
    let (artifact, _kind) = build_artifact(path)?;
    write_artifact(path, &artifact)?;
    eprintln!("cxpak: regenerated {ARTIFACT_REL}");
    Ok(())
}

/// `cxpak hook merge-driver <ancestor> <current> <other>` — union-resolve a
/// conflict in the canonical artifact. Git invokes this with `%O %A %B`; the
/// merged result is written back to the `current` (`%A`) path. Exits 0 on
/// success (always, for the canonical artifact — a union never fails to
/// resolve).
pub fn merge_driver(_ancestor: &Path, current: &Path, other: &Path) -> Result<(), Box<dyn Error>> {
    // Missing side → treat as empty (git always provides all three, but be
    // robust). Ancestor is intentionally unused: pure union of ours ∪ theirs.
    let ours = std::fs::read_to_string(current).unwrap_or_default();
    let theirs = std::fs::read_to_string(other).unwrap_or_default();
    let merged = union_merge(&ours, &theirs);
    std::fs::write(current, merged)?;
    Ok(())
}

/// `cxpak hook install [path]` — wire the post-commit hook + the union merge
/// driver into the TARGET repo. Idempotent (re-install safe); writes only to
/// the target repo's `.git` and `.gitattributes`, never globally.
pub fn install(path: &Path) -> Result<(), Box<dyn Error>> {
    let repo = git2::Repository::open(path)
        .map_err(|e| format!("not a git repository at {}: {e}", path.display()))?;

    install_post_commit_hook(&repo)?;
    install_merge_driver_config(&repo)?;
    install_gitattributes(path)?;

    eprintln!(
        "cxpak: installed post-commit hook + '{MERGE_DRIVER_NAME}' merge driver into {}",
        path.display()
    );
    Ok(())
}

/// Append a fenced, managed block to `.git/hooks/post-commit` (creating the
/// hook with a shebang if absent), preserving any existing user hook body.
/// Idempotent: a re-install replaces the managed block in place.
fn install_post_commit_hook(repo: &git2::Repository) -> Result<(), Box<dyn Error>> {
    let hooks_dir = repo.path().join("hooks");
    std::fs::create_dir_all(&hooks_dir)?;
    let hook_path = hooks_dir.join("post-commit");

    let managed_block = format!(
        "{HOOK_BEGIN}\ncxpak hook post-commit \"$(git rev-parse --show-toplevel)\" || true\n{HOOK_END}\n"
    );

    let new_contents = match std::fs::read_to_string(&hook_path) {
        Ok(existing) => {
            if let (Some(start), Some(end)) = (existing.find(HOOK_BEGIN), existing.find(HOOK_END)) {
                // Replace the existing managed block in place (idempotent).
                let end_of_block = end + HOOK_END.len();
                // Include a trailing newline if present so we don't accrue blanks.
                let after = existing[end_of_block..]
                    .strip_prefix('\n')
                    .unwrap_or(&existing[end_of_block..]);
                format!("{}{}{}", &existing[..start], managed_block, after)
            } else {
                // Preserve the user's hook, append our block.
                let sep = if existing.ends_with('\n') { "" } else { "\n" };
                format!("{existing}{sep}{managed_block}")
            }
        }
        Err(_) => format!("#!/bin/sh\n{managed_block}"),
    };

    std::fs::write(&hook_path, new_contents)?;
    set_executable(&hook_path)?;
    Ok(())
}

/// Register the merge driver in the repo-LOCAL git config. Idempotent: setting
/// the same keys again is a no-op. We explicitly open the `Local` level so the
/// write can never land in the user's global/system config (hard SAFETY
/// constraint — install touches only the target repo).
fn install_merge_driver_config(repo: &git2::Repository) -> Result<(), Box<dyn Error>> {
    let mut config = repo.config()?.open_level(git2::ConfigLevel::Local)?;
    config.set_str(
        &format!("merge.{MERGE_DRIVER_NAME}.name"),
        "cxpak canonical graph artifact union merge",
    )?;
    config.set_str(
        &format!("merge.{MERGE_DRIVER_NAME}.driver"),
        "cxpak hook merge-driver %O %A %B",
    )?;
    Ok(())
}

/// Add the `<artifact> merge=cxpak-union` mapping to the repo's `.gitattributes`
/// if not already present. Idempotent.
fn install_gitattributes(path: &Path) -> Result<(), Box<dyn Error>> {
    let attrs_path = path.join(".gitattributes");
    let entry = format!("{ARTIFACT_REL} merge={MERGE_DRIVER_NAME}");
    let existing = std::fs::read_to_string(&attrs_path).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == entry) {
        return Ok(());
    }
    let sep = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let new_contents = format!("{existing}{sep}{entry}\n");
    std::fs::write(&attrs_path, new_contents)?;
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::watch::apply_incremental_update;

    /// Build a full index for `repo_path`'s working tree and serialize its graph
    /// — the "cold/full" reference the incremental path must match.
    fn full_artifact(repo_path: &Path) -> String {
        let counter = TokenCounter::new();
        let scanner = Scanner::new(repo_path).unwrap();
        let files = scanner.scan().unwrap();
        let (parse_results, content_map) =
            crate::cache::parse::parse_with_cache(&files, repo_path, &counter, false);
        let index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
        serialize_graph_canonical(&index.graph)
    }

    /// Commit every current working-tree file under `dir` as a single commit,
    /// returning the new commit's full oid (hex). LOCAL git2 only, explicit
    /// paths, no cwd use.
    fn commit_all(repo: &git2::Repository) -> String {
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        let parents: Vec<git2::Commit> =
            match repo.head().ok().and_then(|h| h.peel_to_commit().ok()) {
                Some(c) => vec![c],
                None => vec![],
            };
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, "c", &tree, &parent_refs)
            .unwrap()
            .to_string()
    }

    /// Repo-relative cache dir the post-commit derived cache is written to.
    fn cache_dir_of(dir: &Path) -> std::path::PathBuf {
        dir.join(crate::commands::serve::cache_namespace(dir, None))
    }

    /// Overwrite the persisted derived cache's `base_commit` in place (loads it
    /// non-fingerprint-gated, mutates, re-saves) so tests can drive the exact
    /// base-SHA the delta path validates against.
    fn set_cache_base_commit(dir: &Path, base: Option<&str>) {
        let cache_dir = cache_dir_of(dir);
        let mut prior = crate::cache::DerivedCache::load_for_delta(&cache_dir)
            .expect("a derived cache must already exist");
        prior.base_commit = base.map(|s| s.to_string());
        prior.save(&cache_dir).unwrap();
    }

    /// Content+HEAD fingerprint of the working tree, computed exactly as
    /// `build_index` does, to prove the post-commit-warmed cache is a valid
    /// fingerprint-gated hit.
    fn head_fingerprint(dir: &Path) -> String {
        let scanner = Scanner::new(dir).unwrap();
        let files = scanner.scan().unwrap();
        let pairs: Vec<(String, String)> = files
            .iter()
            .map(|f| {
                (
                    f.relative_path.clone(),
                    std::fs::read_to_string(&f.absolute_path).unwrap_or_default(),
                )
            })
            .collect();
        let head_oid = crate::commands::serve::git_head_oid(dir);
        crate::cache::content_fingerprint(&pairs, &head_oid)
    }

    /// Initialise a two-file repo (a.rs imports b.rs) and commit it, returning
    /// (repo, first-commit-oid). a.rs is a real graph node so the subsequent
    /// per-file edge-delta exercises the true delta path (not the internal
    /// structural fallback).
    fn init_two_file_repo(dir: &Path) -> (git2::Repository, String) {
        let repo = git2::Repository::init(dir).unwrap();
        // Gitignore cxpak's own cache dir, mirroring real installs (ADR-0017):
        // the derived/parse caches under `.cxpak/` are regenerated on every
        // build and must NOT count toward working-tree cleanliness. Without this,
        // `commit_all`'s `add_all("*")` would commit the cache, and the next
        // build rewriting it would make the tree read as dirty.
        std::fs::write(dir.join(".gitignore"), ".cxpak/\n").unwrap();
        std::fs::write(dir.join("a.rs"), "use crate::b;\npub fn a() {}\n").unwrap();
        std::fs::write(dir.join("b.rs"), "pub fn b() {}\n").unwrap();
        let c1 = commit_all(&repo);
        (repo, c1)
    }

    /// Three-file `src/`-layout repo (`src/a.rs` imports `crate::b`; `src/c.rs`
    /// is an unreferenced node) with `.cxpak/` gitignored, committed as one
    /// commit. Files live under `src/` so `crate::` imports actually resolve to
    /// real Import edges (root-level `crate::X` never resolves — see
    /// `resolve_rust_import`). Used by the edge-change delta-parity and
    /// dirty-tree-fallback tests.
    fn init_three_file_repo(dir: &Path) -> (git2::Repository, String) {
        let repo = git2::Repository::init(dir).unwrap();
        std::fs::write(dir.join(".gitignore"), ".cxpak/\n").unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        // `use crate::b::item;` (multi-segment) so the parser emits source
        // "crate::b", which resolve_rust_import maps to src/b.rs — a REAL edge.
        // (`use crate::b;` alone emits source "crate", which does not resolve.)
        std::fs::write(
            dir.join("src/a.rs"),
            "use crate::b::helper;\npub fn a() {}\n",
        )
        .unwrap();
        std::fs::write(dir.join("src/b.rs"), "pub fn helper() {}\n").unwrap();
        std::fs::write(dir.join("src/c.rs"), "pub fn helper() {}\n").unwrap();
        let c1 = commit_all(&repo);
        (repo, c1)
    }

    // --- TDD: SAFE base-SHA-validated edge-delta (ADR-0179) -------------------

    /// Base matches parent(HEAD) → the DELTA path runs AND the artifact is
    /// byte-identical to a full rebuild.
    #[test]
    fn post_commit_delta_taken_when_base_matches_and_equals_full() {
        let dir = tempfile::TempDir::new().unwrap();
        let (repo, _c1) = init_two_file_repo(dir.path());

        // Warm the cache at commit1 (Full — commit1 has no parent).
        let (_a, kind1) = build_artifact(dir.path()).unwrap();
        assert_eq!(kind1, RebuildKind::Full, "first build has no parent → Full");

        // Content-only change to a.rs, keeping the import edge, then commit2.
        std::fs::write(
            dir.path().join("a.rs"),
            "use crate::b;\npub fn a() {}\npub fn a2() {}\n",
        )
        .unwrap();
        commit_all(&repo);

        // Cache base_commit (commit1) == parent(HEAD) (commit1) → DELTA.
        let (artifact, kind) = build_artifact(dir.path()).unwrap();
        assert_eq!(
            kind,
            RebuildKind::Delta,
            "base matches parent(HEAD) → Delta"
        );
        assert_eq!(
            artifact,
            full_artifact(dir.path()),
            "delta artifact must be byte-identical to a full rebuild"
        );
    }

    /// No prior cache → full rebuild (still correct artifact).
    #[test]
    fn post_commit_falls_back_to_full_when_no_prior_cache() {
        let dir = tempfile::TempDir::new().unwrap();
        let (repo, _c1) = init_two_file_repo(dir.path());
        std::fs::write(
            dir.path().join("a.rs"),
            "use crate::b;\npub fn a() {}\n// x\n",
        )
        .unwrap();
        commit_all(&repo);

        // No build_artifact ran before → no derived cache to delta from.
        assert!(!cache_dir_of(dir.path()).join("derived.json").exists());
        let (artifact, kind) = build_artifact(dir.path()).unwrap();
        assert_eq!(kind, RebuildKind::Full, "no prior cache → Full");
        assert_eq!(artifact, full_artifact(dir.path()));
    }

    /// Prior `base_commit == None` → full rebuild (never delta onto an
    /// unverified base).
    #[test]
    fn post_commit_falls_back_to_full_when_base_is_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let (repo, _c1) = init_two_file_repo(dir.path());
        build_artifact(dir.path()).unwrap(); // warm cache at commit1
        std::fs::write(
            dir.path().join("a.rs"),
            "use crate::b;\npub fn a() {}\npub fn a2() {}\n",
        )
        .unwrap();
        commit_all(&repo);

        set_cache_base_commit(dir.path(), None); // erase the base

        let (artifact, kind) = build_artifact(dir.path()).unwrap();
        assert_eq!(kind, RebuildKind::Full, "base_commit None → Full");
        assert_eq!(artifact, full_artifact(dir.path()));
    }

    /// CORRUPTION GUARD: prior cache is 2+ commits behind
    /// (`base_commit != parent(HEAD)`) → full rebuild. Deltaing this commit's
    /// changed set onto a graph two commits stale would silently corrupt the
    /// artifact; this is the single most important safety case.
    #[test]
    fn post_commit_falls_back_to_full_when_base_is_two_commits_behind() {
        let dir = tempfile::TempDir::new().unwrap();
        let (repo, _c1) = init_two_file_repo(dir.path());
        build_artifact(dir.path()).unwrap(); // cache base = commit1

        // Two further commits WITHOUT refreshing the cache.
        std::fs::write(
            dir.path().join("a.rs"),
            "use crate::b;\npub fn a() {}\n// c2\n",
        )
        .unwrap();
        commit_all(&repo); // commit2
        std::fs::write(
            dir.path().join("a.rs"),
            "use crate::b;\npub fn a() {}\n// c3\n",
        )
        .unwrap();
        commit_all(&repo); // commit3; parent(HEAD)=commit2, cache base=commit1

        let (artifact, kind) = build_artifact(dir.path()).unwrap();
        assert_eq!(
            kind,
            RebuildKind::Full,
            "cache is 2 commits behind → base != parent(HEAD) → Full"
        );
        assert_eq!(
            artifact,
            full_artifact(dir.path()),
            "corruption guard: stale-base fallback must still equal a full rebuild"
        );
    }

    /// A version/grammar bump (old cache) → `load_for_delta` rejects it → full
    /// rebuild.
    #[test]
    fn post_commit_falls_back_to_full_on_version_mismatch() {
        let dir = tempfile::TempDir::new().unwrap();
        let (repo, _c1) = init_two_file_repo(dir.path());
        build_artifact(dir.path()).unwrap(); // warm cache at commit1
        std::fs::write(
            dir.path().join("a.rs"),
            "use crate::b;\npub fn a() {}\npub fn a2() {}\n",
        )
        .unwrap();
        commit_all(&repo);

        // Downgrade the persisted cache version so load_for_delta rejects it.
        let cache_dir = cache_dir_of(dir.path());
        let path = cache_dir.join("derived.json");
        let mut v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v["version"] = serde_json::json!(5);
        std::fs::write(&path, v.to_string()).unwrap();

        let (artifact, kind) = build_artifact(dir.path()).unwrap();
        assert_eq!(kind, RebuildKind::Full, "stale cache version → Full");
        assert_eq!(artifact, full_artifact(dir.path()));
    }

    /// After post_commit, the shared derived cache is a valid fingerprint-gated
    /// HIT for the new tree — proving the cross-process cache warming a later
    /// `overview` relies on.
    #[test]
    fn post_commit_warms_fingerprint_gated_cache() {
        let dir = tempfile::TempDir::new().unwrap();
        let (repo, _c1) = init_two_file_repo(dir.path());
        build_artifact(dir.path()).unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            "use crate::b;\npub fn a() {}\npub fn a2() {}\n",
        )
        .unwrap();
        commit_all(&repo);

        // Run the real entry point (best-effort) then assert a fingerprint hit.
        post_commit(dir.path()).unwrap();
        let fp = head_fingerprint(dir.path());
        assert!(
            crate::cache::DerivedCache::load(&cache_dir_of(dir.path()), &fp).is_some(),
            "post_commit must warm a fingerprint-gated cache hit for the new tree"
        );
    }

    /// Determinism: the DELTA artifact is byte-stable across repeated runs at the
    /// same HEAD (and equals a full rebuild).
    #[test]
    fn post_commit_delta_artifact_is_deterministic() {
        let dir = tempfile::TempDir::new().unwrap();
        let (repo, c1) = init_two_file_repo(dir.path());
        build_artifact(dir.path()).unwrap(); // base = commit1
        std::fs::write(
            dir.path().join("a.rs"),
            "use crate::b;\npub fn a() {}\npub fn a2() {}\n",
        )
        .unwrap();
        commit_all(&repo); // parent(HEAD) = commit1

        let (artifact1, kind1) = build_artifact(dir.path()).unwrap();
        assert_eq!(kind1, RebuildKind::Delta);

        // build_artifact re-stamped base = commit2; reset it to commit1 so the
        // second run also takes the delta path, then compare bytes.
        set_cache_base_commit(dir.path(), Some(&c1));
        let (artifact2, kind2) = build_artifact(dir.path()).unwrap();
        assert_eq!(kind2, RebuildKind::Delta);

        assert_eq!(artifact1, artifact2, "delta artifact must be byte-stable");
        assert_eq!(artifact1, full_artifact(dir.path()));
    }

    /// EDGE-CHANGING DELTA PARITY: on a clean commit whose ONLY change rewrites
    /// an import edge (a imports c instead of b — content-only, no file
    /// add/remove), the DELTA path is taken AND produces the CORRECT artifact:
    /// byte-identical to a full rebuild, containing the new `a→c` edge and NOT
    /// the stale `a→b`. Proves the delta path yields correct output on a real
    /// edge change when taken (the older delta-taken test kept the same import,
    /// so it would pass even if the delta did nothing).
    #[test]
    fn post_commit_delta_produces_correct_artifact_on_edge_change() {
        let dir = tempfile::TempDir::new().unwrap();
        let (repo, _c1) = init_three_file_repo(dir.path());

        // Warm the cache at commit1 (Full — no parent).
        let (_a, kind1) = build_artifact(dir.path()).unwrap();
        assert_eq!(kind1, RebuildKind::Full, "first build has no parent → Full");

        // Content-only edit: a now imports c instead of b. No file add/remove.
        std::fs::write(
            dir.path().join("src/a.rs"),
            "use crate::c::helper;\npub fn a() {}\npub fn a2() {}\n",
        )
        .unwrap();
        commit_all(&repo); // commit2, tree fully clean

        let (artifact, kind) = build_artifact(dir.path()).unwrap();
        assert_eq!(
            kind,
            RebuildKind::Delta,
            "clean tree + base == parent(HEAD) → Delta"
        );
        assert_eq!(
            artifact,
            full_artifact(dir.path()),
            "delta artifact must be byte-identical to a full rebuild"
        );
        assert!(
            artifact.contains("src/a.rs\tsrc/c.rs\timport"),
            "delta must reflect the new a→c edge, not just skip work"
        );
        assert!(
            !artifact.contains("src/a.rs\tsrc/b.rs\timport"),
            "delta must drop the stale a→b edge"
        );
    }

    /// CRITICAL REPRO (dirty-tree fallback): commit2 cleanly rewrites a's import
    /// (diff == {a.rs}, base == parent(HEAD)), but a DIFFERENT source file `b`
    /// carries an UNCOMMITTED edit that changes ITS edges (b→c), NOT in commit2's
    /// diff. A base-SHA-only delta would recompute only `a` and keep `b`'s stale
    /// (edge-less) base graph — silently diverging from a full rebuild that scans
    /// the dirty working tree and sees b→c. The clean-tree gate must force Full.
    /// BEFORE the fix this FAILS (Delta taken → wrong artifact, missing b→c);
    /// after, the dirty tree forces Full and the artifact matches a full rebuild.
    #[test]
    fn post_commit_falls_back_to_full_when_working_tree_dirty() {
        let dir = tempfile::TempDir::new().unwrap();
        let (repo, _c1) = init_three_file_repo(dir.path());

        build_artifact(dir.path()).unwrap(); // warm cache, base = commit1

        // commit2: content-only edit to a (a now imports c). ONLY a.rs committed.
        std::fs::write(
            dir.path().join("src/a.rs"),
            "use crate::c::helper;\npub fn a() {}\npub fn a2() {}\n",
        )
        .unwrap();
        commit_all(&repo); // parent(HEAD) == commit1 == cache base → would be Delta

        // Uncommitted edit to a DIFFERENT tracked file, NOT in commit2's diff,
        // that changes ITS edges: b now imports c (b→c). This is the silent-
        // corruption trigger — the committed diff is {src/a.rs} only.
        std::fs::write(
            dir.path().join("src/b.rs"),
            "use crate::c::helper;\npub fn b() {}\n",
        )
        .unwrap();

        let (artifact, kind) = build_artifact(dir.path()).unwrap();
        assert_eq!(
            kind,
            RebuildKind::Full,
            "dirty working tree must force Full (base-SHA delta would corrupt the artifact)"
        );
        assert_eq!(
            artifact,
            full_artifact(dir.path()),
            "Full fallback must reflect the uncommitted b→c edge"
        );
        assert!(
            artifact.contains("src/b.rs\tsrc/c.rs\timport"),
            "artifact must contain the uncommitted b→c edge a base-SHA delta would have missed"
        );
    }

    /// ROUTE A (truthful stamp): a build on a CLEAN tree stamps
    /// `base_commit = Some(HEAD)`; a build on a DIRTY tree (an uncommitted edit
    /// to a tracked source file) stamps `base_commit = None`, so a later commit
    /// never deltas onto a graph that was built from uncommitted content but
    /// mislabeled as the clean HEAD base.
    #[test]
    fn build_stamps_base_commit_none_on_dirty_tree_and_head_on_clean() {
        let dir = tempfile::TempDir::new().unwrap();
        let (_repo, c1) = init_two_file_repo(dir.path());

        // Clean tree at commit1 → stamp Some(HEAD).
        build_artifact(dir.path()).unwrap();
        let clean = crate::cache::DerivedCache::load_for_delta(&cache_dir_of(dir.path()))
            .expect("a derived cache must exist after build");
        assert_eq!(
            clean.base_commit.as_deref(),
            Some(c1.as_str()),
            "clean tree must stamp base_commit = Some(HEAD)"
        );

        // Uncommitted edit to a tracked source file → dirty tree → None.
        std::fs::write(
            dir.path().join("a.rs"),
            "use crate::b;\npub fn a() {}\npub fn dirty() {}\n",
        )
        .unwrap();
        build_artifact(dir.path()).unwrap();
        let dirty = crate::cache::DerivedCache::load_for_delta(&cache_dir_of(dir.path()))
            .expect("a derived cache must exist after build");
        assert!(
            dirty.base_commit.is_none(),
            "dirty tree must stamp base_commit = None (Route A: no mislabeled base)"
        );
    }

    // --- TDD headline test 1: post-commit incremental == full ----------------

    /// In a throwaway temp git repo, commit a set of files, then bring a
    /// pre-commit index up to the post-commit state via the SAME incremental
    /// machinery serve.rs uses (`apply_incremental_update` + `rebuild_graph_delta`)
    /// and assert the canonical artifact is byte-identical to a full rebuild —
    /// the incremental==full invariant the post-commit artifact relies on.
    #[test]
    fn post_commit_incremental_equals_full() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        // Initial state: a.rs imports nothing; commit it.
        std::fs::write(dir.path().join("a.rs"), "pub fn a() {}\n").unwrap();
        commit_all(&repo);

        // Build the "prior" index from the pre-commit working tree.
        let counter = TokenCounter::new();
        let scanner = Scanner::new(dir.path()).unwrap();
        let files = scanner.scan().unwrap();
        let (parse_results, content_map) =
            crate::cache::parse::parse_with_cache(&files, dir.path(), &counter, false);
        let mut prior =
            CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);

        // Second commit modifies a.rs (content-only change → exact edge delta).
        std::fs::write(dir.path().join("a.rs"), "pub fn a() {}\npub fn a2() {}\n").unwrap();
        commit_all(&repo);

        let (changed, removed) = changed_paths_in_head(&repo).unwrap();
        assert!(changed.contains("a.rs"), "expected a.rs in changed set");

        // Feed the commit's changed set through the existing machinery.
        apply_incremental_update(&mut prior, dir.path(), &changed, &removed);
        prior.rebuild_graph_delta(&changed, &removed);
        let incremental = serialize_graph_canonical(&prior.graph);

        let full = full_artifact(dir.path());
        assert_eq!(
            incremental, full,
            "incremental artifact must be byte-identical to a full rebuild"
        );
    }

    /// Best-effort safety: `post_commit` exits Ok even on nonsense input
    /// (a path that is not a git repo and has no source files).
    #[test]
    fn post_commit_is_non_fatal_on_garbage_input() {
        let dir = tempfile::TempDir::new().unwrap();
        // Not a git repo, no files. Must still return Ok (never break a workflow).
        let result = post_commit(dir.path());
        assert!(result.is_ok(), "post_commit must be non-fatal");
    }

    /// Best-effort safety: an internal error (a path that cannot be scanned)
    /// is swallowed — `post_commit` still returns Ok rather than propagating.
    #[test]
    fn post_commit_swallows_internal_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        // A regular file, not a directory: scanning it fails inside the fallible
        // core, which `post_commit` must catch and turn into Ok.
        let file = dir.path().join("not-a-dir");
        std::fs::write(&file, "x").unwrap();
        let result = post_commit(&file);
        assert!(result.is_ok(), "internal errors must not propagate");
        assert!(
            !file.join(".cxpak").exists(),
            "no artifact dir should be created for a bad path"
        );
    }

    /// post_commit writes the canonical artifact after a real commit.
    #[test]
    fn post_commit_writes_artifact() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.rs"), "use crate::b;\npub fn a() {}\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "pub fn b() {}\n").unwrap();
        commit_all(&repo);

        post_commit(dir.path()).unwrap();
        let artifact = dir.path().join(ARTIFACT_REL);
        assert!(artifact.exists(), "post_commit must write {ARTIFACT_REL}");
        // Lock the production artifact CONTENT, not just its existence: the file
        // the user commits and the merge driver consumes must be byte-identical
        // to a full canonical rebuild of the same tree.
        let written = std::fs::read_to_string(&artifact).unwrap();
        assert_eq!(
            written,
            full_artifact(dir.path()),
            "post_commit artifact must be the canonical full-rebuild serialization"
        );
    }

    // --- TDD headline test 2: union-merge driver -----------------------------

    /// A synthetic conflict (base + ours-adds-X + theirs-adds-Y) union-resolves
    /// to the deterministic union of both, sorted, no conflict markers, and is
    /// byte-identical regardless of ours/theirs order (commutative).
    #[test]
    fn union_merge_is_deterministic_and_commutative() {
        let base = "a.rs\tb.rs\timport\textracted\n";
        let ours = "a.rs\tb.rs\timport\textracted\na.rs\tx.rs\timport\textracted\n";
        let theirs = "a.rs\tb.rs\timport\textracted\na.rs\ty.rs\timport\textracted\n";

        let merged = union_merge(ours, theirs);

        // Both new edges present, base retained, sorted, no conflict markers.
        assert!(merged.contains("a.rs\tx.rs\timport\textracted"));
        assert!(merged.contains("a.rs\ty.rs\timport\textracted"));
        assert!(merged.contains("a.rs\tb.rs\timport\textracted"));
        assert!(!merged.contains("<<<<<<<"));
        assert!(!merged.contains(">>>>>>>"));

        let expected = "a.rs\tb.rs\timport\textracted\n\
                        a.rs\tx.rs\timport\textracted\n\
                        a.rs\ty.rs\timport\textracted\n";
        assert_eq!(merged, expected, "union must be sorted + deduped");

        // Commutative: swapping ours/theirs yields byte-identical output.
        assert_eq!(merged, union_merge(theirs, ours));

        // Idempotent across runs.
        assert_eq!(merged, union_merge(ours, theirs));

        // base unused but accepted by the driver contract.
        let _ = base;
    }

    /// The merge driver writes the union back to the `current` (%A) path and
    /// exits Ok.
    #[test]
    fn merge_driver_writes_union_to_current() {
        let dir = tempfile::TempDir::new().unwrap();
        let ancestor = dir.path().join("O");
        let current = dir.path().join("A");
        let other = dir.path().join("B");
        std::fs::write(&ancestor, "a.rs\tb.rs\timport\textracted\n").unwrap();
        std::fs::write(
            &current,
            "a.rs\tb.rs\timport\textracted\na.rs\tx.rs\timport\textracted\n",
        )
        .unwrap();
        std::fs::write(
            &other,
            "a.rs\tb.rs\timport\textracted\na.rs\ty.rs\timport\textracted\n",
        )
        .unwrap();

        merge_driver(&ancestor, &current, &other).unwrap();

        let result = std::fs::read_to_string(&current).unwrap();
        assert!(result.contains("a.rs\tx.rs\timport\textracted"));
        assert!(result.contains("a.rs\ty.rs\timport\textracted"));
        assert!(!result.contains("<<<<<<<"));
    }

    // --- serialization + install --------------------------------------------

    #[test]
    fn serialize_graph_canonical_is_sorted_and_terminated() {
        use crate::index::graph::EdgeType;
        let mut graph = DependencyGraph::new();
        graph.add_edge("z.rs", "a.rs", EdgeType::Import);
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        let out = serialize_graph_canonical(&graph);
        assert_eq!(
            out, "a.rs\tb.rs\timport\textracted\nz.rs\ta.rs\timport\textracted\n",
            "lines must be sorted with a trailing newline"
        );
    }

    #[test]
    fn serialize_empty_graph_is_empty_string() {
        let graph = DependencyGraph::new();
        assert_eq!(serialize_graph_canonical(&graph), "");
    }

    #[test]
    fn install_is_idempotent_and_scoped_to_target_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        drop(repo);

        install(dir.path()).unwrap();
        install(dir.path()).unwrap(); // re-install must be safe

        let hook = dir.path().join(".git/hooks/post-commit");
        assert!(hook.exists());
        let hook_body = std::fs::read_to_string(&hook).unwrap();
        assert_eq!(
            hook_body.matches(HOOK_BEGIN).count(),
            1,
            "managed block must appear exactly once after re-install"
        );

        let attrs = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
        assert_eq!(
            attrs
                .lines()
                .filter(|l| l.contains("merge=cxpak-union"))
                .count(),
            1,
            ".gitattributes entry must not duplicate"
        );

        // Merge driver registered in the repo-LOCAL config only.
        let repo = git2::Repository::open(dir.path()).unwrap();
        let config = repo.config().unwrap();
        assert_eq!(
            config.get_string("merge.cxpak-union.driver").unwrap(),
            "cxpak hook merge-driver %O %A %B"
        );
    }

    /// Install must preserve an existing user post-commit hook body.
    #[test]
    fn install_preserves_existing_user_hook() {
        let dir = tempfile::TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let hooks_dir = dir.path().join(".git/hooks");
        std::fs::create_dir_all(&hooks_dir).unwrap();
        let hook_path = hooks_dir.join("post-commit");
        std::fs::write(&hook_path, "#!/bin/sh\necho user-hook\n").unwrap();

        install(dir.path()).unwrap();

        let body = std::fs::read_to_string(&hook_path).unwrap();
        assert!(body.contains("echo user-hook"), "user hook must survive");
        assert!(body.contains(HOOK_BEGIN), "managed block must be appended");
    }

    /// Removing a file across commits must drop its edges (structural change →
    /// full rebuild fallback), still matching a full rebuild.
    #[test]
    fn incremental_removal_matches_full() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.rs"), "pub fn a() {}\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "pub fn b() {}\n").unwrap();
        commit_all(&repo);

        let counter = TokenCounter::new();
        let scanner = Scanner::new(dir.path()).unwrap();
        let files = scanner.scan().unwrap();
        let (parse_results, content_map) =
            crate::cache::parse::parse_with_cache(&files, dir.path(), &counter, false);
        let mut prior =
            CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);

        std::fs::remove_file(dir.path().join("b.rs")).unwrap();
        commit_all(&repo);

        let (changed, removed) = changed_paths_in_head(&repo).unwrap();
        assert!(removed.contains("b.rs"));
        let _ = apply_incremental_update(&mut prior, dir.path(), &changed, &removed);
        prior.rebuild_graph_delta(&changed, &removed);

        assert_eq!(
            serialize_graph_canonical(&prior.graph),
            full_artifact(dir.path())
        );
    }

    /// Empty (markerless) inputs union to empty string.
    #[test]
    fn union_of_empty_is_empty() {
        assert_eq!(union_merge("", ""), "");
        assert_eq!(union_merge("\n\n", ""), "");
    }

    /// Conflict markers in either side are dropped, never surfacing in output.
    #[test]
    fn union_strips_conflict_markers() {
        let ours = "<<<<<<< HEAD\na.rs\tb.rs\timport\textracted\n=======\n";
        let theirs = "a.rs\tc.rs\timport\textracted\n>>>>>>> branch\n";
        let merged = union_merge(ours, theirs);
        assert_eq!(
            merged,
            "a.rs\tb.rs\timport\textracted\na.rs\tc.rs\timport\textracted\n"
        );
    }

    /// The HEAD-commit diff classifies an initial commit's files as changed
    /// (no parent → diff against the empty tree).
    #[test]
    fn changed_paths_initial_commit_are_changed() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.rs"), "pub fn a() {}\n").unwrap();
        commit_all(&repo);
        let (changed, removed) = changed_paths_in_head(&repo).unwrap();
        assert!(changed.contains("a.rs"));
        assert!(removed.is_empty());
    }
}
