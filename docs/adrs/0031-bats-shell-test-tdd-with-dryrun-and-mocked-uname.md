---
id: '0031'
title: BATS as the test framework with TDD, mocked uname, and a --dry-run seam
status: ACCEPTED
date: 2026-03-11
triggered_by: The plugin is bash + markdown, not Rust; the cxpak cargo suite cannot cover it
loop: implementation
---

# ADR-0031: BATS as the test framework with TDD, mocked uname, and a --dry-run seam

## Context

The Claude Code plugin (cxpak v0.4.0) is composed of shell scripts and markdown files that live outside the Rust crate. The existing `cargo test` suite cannot validate them. The implementation plan selects BATS (Bash Automated Testing System) as the test framework for the shell layer and mandates writing failing tests first per task (TDD).

Platform detection is the hard part to test: the original `ensure-cxpak` design branched on `uname` output to pick a target triple and download a tarball. To make this testable without real hardware or network access, the design relies on a `--dry-run` flag that prints the resolved target/URL without downloading, and tests mock the `uname` binary onto `PATH` per test to drive each platform branch.

## Options considered

- **Option A — BATS with TDD, mocked uname, --dry-run (chosen):** Write per-task failing tests first; mock `uname` onto `PATH` to drive platform branches; `--dry-run` prints target/URL without downloading. Pros: pure-bash testing of bash, no network in unit tests, platform matrix covered deterministically. Cons: adds BATS as a dev dependency (`brew install bats-core`). Someone could still prefer this for its fit with the language under test.
- **Option B — shell tests via a cargo/Rust harness:** A reasonable alternative would have been driving the scripts from Rust integration tests for a single test runner. Awkward for bash and offers no natural way to mock `uname`. One could prefer it to avoid a second test framework.
- **Option C — manual / no automated tests for the shell layer:** A reasonable alternative would have been testing by hand for zero setup cost. Rejected because a broken download path silently breaks the entire plugin; someone might prefer it only for a throwaway prototype.

## Decision

Use BATS for all plugin tests, write failing tests before each script/file (TDD), and make `ensure-cxpak`'s platform detection testable via a `--dry-run` flag with `uname` mocked onto `PATH` per test. Cover all four target triples (`aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`), an unsupported-OS failure path, PATH-vs-cache precedence, and a real-git-repo integration suite exercising overview/trace/diff/clean end to end.

## Consequences

### Positive
- The bash layer gets first-class deterministic coverage.
- The platform matrix is tested without real hardware or network.
- Integration tests exercise the full resolve-then-run pipeline against a temporary git repo.

### Negative
- The stated test count assumes the originally-designed tarball-download code path. Parts of that path were later replaced by the Homebrew/cargo resolver, so the dry-run/uname tests now reference a code path the shipped script no longer has.

### Neutral
- BATS becomes a required local dev tool (`brew install bats-core`).
- The originally stated target was 63 unit + 6 integration tests; the tree has since grown beyond that.

## Revisit if
- The plugin gains non-bash components.
- Distribution changes invalidate the platform-detection tests.
