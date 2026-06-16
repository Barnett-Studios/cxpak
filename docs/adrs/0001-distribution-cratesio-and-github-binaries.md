---
id: '0001'
title: Distribute via crates.io (cargo install) and pre-built GitHub Release binaries; no Homebrew
status: ACCEPTED
date: 2026-03-05
triggered_by: Deciding how end users obtain the cxpak binary
loop: planning
---

# ADR-0001: Distribute via crates.io (cargo install) and pre-built GitHub Release binaries; no Homebrew

## Context

In the v0.1.0 design, cxpak needed a delivery story that reached both Rust developers and users who do not have a Rust toolchain. The design specifies two distribution channels — `cargo install cxpak` from crates.io and pre-built binaries via GitHub Releases — and explicitly lists "Homebrew formula" under Not In Scope. The constraint that shaped this is automation: both channels can be driven from a CI release pipeline keyed on `vX.Y.Z` tags.

## Options considered

- **Option A — crates.io + GitHub Release binaries:** Publish the crate to crates.io and attach cross-compiled binaries to each GitHub Release. Pros: reaches Rust users via `cargo install` and non-Rust users via a direct binary download; fully CI-automatable. Cons: two channels to maintain, and no native macOS package-manager install. Someone could prefer this because it covers the widest audience without committing to a third-party packaging ecosystem.
- **Option B — Add a Homebrew formula:** Also ship a `brew` formula for macOS. Pros: native, familiar macOS install UX (`brew install cxpak`). Cons: extra maintenance of a tap/formula, and it was declared out of scope at design time to keep the initial release surface small. Someone could prefer this for the macOS-first developer experience — and in fact this is the option that was later adopted.

## Decision

Distribute cxpak through crates.io (`cargo install cxpak`) plus pre-built GitHub Release binaries cross-compiled for Linux and macOS, with a Homebrew formula explicitly out of scope. The release pipeline triggers on `vX.Y.Z` tags, cross-compiles the binaries, attaches them to a GitHub Release, and runs `cargo publish`.

## Consequences

### Positive
- Covers both Rust (`cargo install`) and binary-download users from a single tagged release.
- The release pipeline cross-compiles for Linux/macOS and publishes to crates.io automatically on `vX.Y.Z` tags.

### Negative
- The "no Homebrew" stance was later reversed. The Claude Code plugin's `ensure-cxpak` script installs via Homebrew (`brew tap` + `brew install`) or cargo, and the release pipeline gained a dedicated job that generates and pushes a `Formula/cxpak.rb` to a Homebrew tap — so brew is now a first-class supported install path.

### Neutral
- A version-sync discipline across `Cargo.toml` and the plugin manifests became necessary because `cargo publish` fails on a stale `Cargo.lock`; `Cargo.lock` must be regenerated and committed before tagging.

## Revisit if
- Demand for a native macOS package-manager install grows (it later did, via the plugin installer and the Homebrew tap, reversing this decision).
