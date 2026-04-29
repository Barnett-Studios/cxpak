#!/usr/bin/env bash
# Trim cxpak's `target/` to keep dev-disk usage sane.
#
# What balloons it (in order of typical contribution):
#   1. Per-test binaries — every tests/*.rs becomes a separate executable
#      (~50 here), each linking the full lib + heavy deps (wasmtime, candle,
#      43 tree-sitter parsers).  Profile overrides in Cargo.toml
#      (`debug = "line-tables-only"`, deps stripped) cut this 40-50%, but
#      the count is fundamental.
#   2. Incremental artefacts — `target/debug/incremental/` is per-crate,
#      per-edit, never auto-pruned.  10-30 GB on an active branch.
#   3. Coverage instrumentation — `target/llvm-cov-target/` duplicates the
#      whole build for instrumented compilation.  Single full coverage
#      run = +20-30 GB.
#   4. Stale generated artefacts from old branches — `cargo sweep` removes
#      target files older than N days.
#
# Usage:
#   bash scripts/disk-cleanup.sh           # safe defaults: incremental + cov + 30d sweep
#   bash scripts/disk-cleanup.sh --aggressive   # also runs `cargo clean`
#
# Safety contract:
#   This script will REFUSE to delete anything outside `<project>/target/`.
#   Every `rm -rf` is gated on `cd && pwd -P` containment under a pre-resolved
#   `$TARGET_DIR` (portable on BSD/GNU; macOS bash 3.2 + BSD `realpath` lack
#   `-e`).  The script also refuses to run if it cannot positively identify
#   itself as living in the cxpak repo (Cargo.toml present, `name = "cxpak"`),
#   or if `target/` is a symlink (could redirect outside).
#
# For a more permanent cure, set a SHARED target dir across all your Rust
# projects in ~/.cargo/config.toml (single biggest win for someone with
# multiple Rust projects):
#
#   [build]
#   target-dir = "/Users/<you>/.cache/cargo-target"
#
# That deduplicates dependency builds across workspaces.  Note: with a
# shared target-dir set, this script will refuse to run because the
# project-local `target/` directory will not exist — that is correct;
# clean the shared dir manually or via its own `cargo sweep` invocation.

set -euo pipefail

# ─── Resolve script + project paths canonically ─────────────────────────────
# `pwd -P` strips symlinks; `BASH_SOURCE[0]` is robust under `bash script.sh`,
# `./script.sh`, sourcing, or `bash -c`.
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
PROJECT_ROOT=$(cd "$SCRIPT_DIR/.." && pwd -P)

# ─── Verify we're actually in the cxpak repo ────────────────────────────────
# Refuses to run if the script has been copied/moved into another tree.
if [[ ! -f "$PROJECT_ROOT/Cargo.toml" ]]; then
    echo "REFUSE: $PROJECT_ROOT/Cargo.toml not found — not a Rust project root." >&2
    exit 1
fi
if ! grep -qE '^name[[:space:]]*=[[:space:]]*"cxpak"' "$PROJECT_ROOT/Cargo.toml"; then
    echo "REFUSE: $PROJECT_ROOT/Cargo.toml is not the cxpak crate manifest." >&2
    exit 1
fi

# ─── Resolve and validate TARGET_DIR ────────────────────────────────────────
# A symlinked target/ could redirect deletes outside the project; refuse it.
if [[ ! -e "$PROJECT_ROOT/target" ]]; then
    echo "[noop] $PROJECT_ROOT/target does not exist — nothing to clean."
    exit 0
fi
if [[ -L "$PROJECT_ROOT/target" ]]; then
    echo "REFUSE: $PROJECT_ROOT/target is a symlink — refusing to follow it." >&2
    exit 1
fi
if [[ ! -d "$PROJECT_ROOT/target" ]]; then
    echo "REFUSE: $PROJECT_ROOT/target exists but is not a directory." >&2
    exit 1
fi
TARGET_DIR=$(cd "$PROJECT_ROOT/target" && pwd -P)

# Defence in depth: TARGET_DIR must itself sit inside PROJECT_ROOT.
if [[ "$TARGET_DIR" != "$PROJECT_ROOT/target" ]]; then
    echo "REFUSE: resolved TARGET_DIR ($TARGET_DIR) is not $PROJECT_ROOT/target." >&2
    exit 1
fi

# ─── safe_rm_inside_target ──────────────────────────────────────────────────
# Canonicalises a directory via `cd "$dir" && pwd -P` (portable on BSD bash
# 3.2 — GNU `realpath -e` is unavailable on stock macOS) and refuses unless
# the resolved path is *strictly inside* $TARGET_DIR.  $TARGET_DIR itself is
# forbidden; only descendants are deletable.  Trailing-slash check on the
# prefix prevents sibling-name attacks (e.g., `<root>/target-evil/...`).
# Symlinks are refused outright — even a symlink whose name is inside target/
# can point anywhere.  Non-directory inputs are refused; the script's known
# call-sites all pass directories.
safe_rm_inside_target() {
    local input="$1"
    if [[ ! -e "$input" ]]; then
        echo "[skip] $input does not exist (already gone?)"
        return 0
    fi
    if [[ -L "$input" ]]; then
        echo "REFUSE: $input is a symlink — refusing to follow it." >&2
        return 1
    fi
    if [[ ! -d "$input" ]]; then
        echo "REFUSE: $input is not a directory." >&2
        return 1
    fi
    local resolved
    if ! resolved=$(cd -- "$input" && pwd -P); then
        echo "REFUSE: cannot canonicalise $input." >&2
        return 1
    fi
    if [[ "$resolved" == "$TARGET_DIR" ]]; then
        echo "REFUSE: would delete TARGET_DIR itself ($resolved)." >&2
        return 1
    fi
    if [[ "$resolved" != "$TARGET_DIR"/* ]]; then
        echo "REFUSE: $input resolves to $resolved — outside $TARGET_DIR." >&2
        return 1
    fi
    rm -rf -- "$resolved"
}

mode="${1:-default}"

before=$(du -sh "$TARGET_DIR" 2>/dev/null | cut -f1 || echo "—")

if [[ "$mode" == "--aggressive" ]]; then
    echo "[aggressive] running cargo clean (full nuke) bound to ${PROJECT_ROOT}…"
    # `--manifest-path` binds cargo to this project; cargo only ever touches
    # the target-dir configured for this manifest, never a different project.
    cargo clean --manifest-path "$PROJECT_ROOT/Cargo.toml"
else
    # Incremental cache — safe to drop, will rebuild on next compile.
    if [[ -d "$TARGET_DIR/debug/incremental" ]]; then
        size=$(du -sh "$TARGET_DIR/debug/incremental" 2>/dev/null | cut -f1)
        echo "[incremental] removing target/debug/incremental ($size)…"
        safe_rm_inside_target "$TARGET_DIR/debug/incremental"
    fi
    if [[ -d "$TARGET_DIR/release/incremental" ]]; then
        size=$(du -sh "$TARGET_DIR/release/incremental" 2>/dev/null | cut -f1)
        echo "[incremental] removing target/release/incremental ($size)…"
        safe_rm_inside_target "$TARGET_DIR/release/incremental"
    fi

    # Coverage instrumentation duplicates the build tree — only useful for
    # the most recent run.  Drop it; re-runs are cheap once the regular
    # build is warm.
    if [[ -d "$TARGET_DIR/llvm-cov-target" ]]; then
        size=$(du -sh "$TARGET_DIR/llvm-cov-target" 2>/dev/null | cut -f1)
        echo "[coverage] removing target/llvm-cov-target ($size)…"
        safe_rm_inside_target "$TARGET_DIR/llvm-cov-target"
    fi

    # Stale artefacts older than 30 days.  Requires `cargo sweep` —
    # `cargo install cargo-sweep` if missing.  Bound to this manifest so it
    # cannot touch any other project's target dir.
    if command -v cargo-sweep >/dev/null 2>&1; then
        echo "[sweep] removing artefacts older than 30 days (manifest=$PROJECT_ROOT/Cargo.toml)…"
        cargo sweep --time 30 --manifest-path "$PROJECT_ROOT/Cargo.toml" || true
    else
        echo "[sweep] cargo-sweep not installed — \`cargo install cargo-sweep\` for stale-artefact cleanup."
    fi
fi

after=$(du -sh "$TARGET_DIR" 2>/dev/null | cut -f1 || echo "—")
echo
echo "target/ size: $before → $after"
