---
id: '0028'
title: Distribute via a dedicated Homebrew tap that downloads prebuilt release tarballs, auto-updated by CI
status: ACCEPTED
date: 2026-03-10
triggered_by: cargo install is the only install path; a brew install option lowers friction for macOS/Linux users.
loop: planning
---

# ADR-0028: Distribute via a dedicated Homebrew tap that downloads prebuilt release tarballs, auto-updated by CI

## Context

Through v0.3.0, `cargo install` was the only install path for cxpak, which requires a Rust toolchain on the user's machine. A `brew install` option lowers friction for macOS and Linux users.

v0.4.0 adds a Homebrew tap. A formula can either build from source or download the prebuilt per-platform tarballs that the release workflow already produces. To keep the formula correct without manual edits on every release, a CI job updates the formula's version and SHA256 sums.

Note: the cited design doc (`2026-03-10-homebrew-caching-diff-design.md`) names the tap owner as `lyubomir-bozhinov`; during implementation this was changed to `Barnett-Studios`. The shipped CI workflow uses `Barnett-Studios`, and this record reflects the shipped value.

## Options considered

- **Option A — Tap formula downloads prebuilt GitHub Release tarballs; a CI job updates SHA/version:** A bottle-style formula referencing per-platform tarballs, with an `update-homebrew` CI job that downloads the artifacts, computes SHA256s, renders the formula template with the version and SHAs, and pushes the result to the tap repo using a PAT. Pros: fast install with no compilation, reuses the existing release artifacts, and the formula stays in sync automatically. Cons: requires a cross-repo PAT secret (`TAP_TOKEN`), and the formula must enumerate every platform/arch combination. Someone could prefer this for the fast, toolchain-free install experience.
- **Option B — Homebrew formula that builds from source:** A reasonable alternative would have been a formula that compiles cxpak from the crate. Pros: no need to publish per-platform tarballs. Cons: slow install and requires a Rust toolchain on the user's machine. Someone could prefer it to avoid managing a PAT secret and per-platform artifacts.

## Decision

Create a dedicated `Barnett-Studios/homebrew-tap` repo with a formula that downloads the prebuilt per-platform tarballs from GitHub Releases. Add an `update-homebrew` CI job (running after release) that downloads the artifacts, computes SHA256s, renders the formula template with version + SHAs, and pushes to the tap repo using a `TAP_TOKEN` PAT secret. Install via `brew tap Barnett-Studios/tap && brew install cxpak`.

Confirmed shipped in `.github/workflows/release.yml` (`update-homebrew` job depending on the release job, covering all four platform/arch tarballs).

## Consequences

### Positive
- One-line `brew install` with no compilation.
- Formula stays current automatically on every release.
- Reuses artifacts the release workflow already builds.

### Negative
- Introduces a cross-repo PAT secret (`TAP_TOKEN`) that must be managed.
- Formula must enumerate all four platform/arch combinations.

### Neutral
- The tap is a separate repository outside the cxpak codebase.

## Revisit if
- A new target triple is added (the formula needs another url/sha block).
- PAT management proves burdensome (consider a GitHub App instead).
