#!/usr/bin/env bash
#
# Feature-matrix build verification.
#
# cxpak ships several optional features (`plugins`, `embeddings`, `lsp`,
# `daemon`, `visual`).  The default-features set excludes `plugins` (its
# guest-binding stub is a known shortcut), so a developer running
# `cargo build` won't catch a regression that breaks the plugin build.
# This script exercises every meaningful combination so CI catches
# feature-flag drift before tag-time.
#
# Usage:
#   bash scripts/feature-matrix.sh         # check every combination
#   bash scripts/feature-matrix.sh build   # build only, skip tests
#
# Exit non-zero on the first failing combination so CI reports clearly.

set -euo pipefail

# Force the toolchain pinned in CLAUDE.md so this script behaves
# identically locally and on CI.
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-1.94.1}"

mode="${1:-test}"
case "$mode" in
    build|test) ;;
    *) echo "usage: $0 [build|test]" >&2; exit 2 ;;
esac

# Each row: human label + flag set passed to cargo.  `--no-default-features`
# is added by the runner; the second field is what follows it.
combinations=(
    "default                       :"
    "no-default                    :--no-default-features"
    "minimal-rust                  :--no-default-features --features lang-rust"
    "core-no-plugins-no-embeddings :--no-default-features --features visual,daemon,lsp,lang-rust,lang-typescript"
    "core-with-embeddings          :--no-default-features --features visual,daemon,lsp,embeddings,lang-rust"
    "plugins-only                  :--features plugins"
    "all-features                  :--all-features"
)

failures=0
for entry in "${combinations[@]}"; do
    label="${entry%%:*}"
    flags="${entry##*:}"
    label_trimmed="${label// /}"
    echo
    echo "── ${label_trimmed} ──"
    echo "  cargo $mode $flags"
    if cargo "$mode" --quiet $flags 2>&1 | tail -5; then
        echo "  ✓ ${label_trimmed} ${mode} ok"
    else
        echo "  ✗ ${label_trimmed} ${mode} FAILED"
        failures=$((failures + 1))
    fi
done

echo
if [[ $failures -gt 0 ]]; then
    echo "feature-matrix: ${failures} combination(s) failed" >&2
    exit 1
fi
echo "feature-matrix: all combinations green"
