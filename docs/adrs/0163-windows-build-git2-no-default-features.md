---
id: '0163'
title: Ship a Windows binary by removing OpenSSL from the dependency tree (git2 + reqwest)
status: ACCEPTED
date: 2026-06-14
triggered_by: Repo polish for public release — release.yml shipped only Linux/macOS, omitting Windows for a "vendored libgit2 linking issue"
loop: implementation
---

# ADR-0163: Ship a Windows binary by removing OpenSSL from the dependency tree

## Context

The release workflow (`.github/workflows/release.yml`) built four targets — Linux x86_64/aarch64 and macOS x86_64/aarch64 — and explicitly skipped Windows with the comment *"Windows: cargo install cxpak (vendored libgit2 linking issue)"*. Windows users could `cargo install cxpak` but got no prebuilt binary. The root cause was OpenSSL, which entered the build by **two independent paths** — only the first blocked Windows, but a naive fix to it regressed Linux, so both had to be addressed:

1. **git2's transport.** `Cargo.toml` declared `git2 = { features = ["vendored-openssl"] }`, and git2's default `https`/`ssh` features pull `openssl-sys` (via `libgit2-sys`) and `libssh2-sys`. cxpak's entire git2 usage is local and read-mostly — `Repository::open`/`init`/`discover`, `revparse_single`, revwalk, diff-against-parent, `Signature`, `Oid`; there is no `Remote`, `Cred`, `clone`, `fetch`, or `push` in `src/`. The HTTPS/SSH transport is never exercised, so its OpenSSL was dead weight, and on MSVC it failed to link.

2. **reqwest's TLS.** The `embeddings` feature uses `reqwest` (for remote embedding providers and the HuggingFace model download) with its default `native-tls` backend. `cargo tree -i openssl-sys --target all` showed that on **Linux** this pulls `openssl-sys` via `hyper-tls → native-tls`; on macOS/Windows, `native-tls` uses the OS TLS (Secure Transport / SChannel) and pulls nothing.

The interaction is what made the fix non-trivial. Setting `git2` to `default-features = false` removes `openssl-sys` on macOS/Windows and `libssh2-sys` everywhere — but on Linux it also drops the `vendored-openssl` flag that built a *static* OpenSSL, leaving `reqwest`'s `openssl-sys` to link the *system* libssl dynamically. That would make the Linux release binary require `libssl` on the user's machine — a portability regression versus the previous vendored-static build. The clean resolution is to take OpenSSL out of *both* paths.

This was verified empirically: after both changes, `cargo tree -i openssl-sys --target all --all-features` reports no such package, and a `--release --features daemon` build compiles in ~90s.

## Options considered

- **Option A — drop git2 default features AND switch reqwest to rustls (chosen):** `git2 = { default-features = false }` removes the unused git2 transport (and `libssh2-sys`); `reqwest = { default-features = false, features = ["json","blocking","rustls-tls"] }` replaces `native-tls` with rustls (a pure-Rust TLS stack with bundled roots). Net: `openssl-sys` leaves the graph on every target, binaries are static and portable everywhere, and Windows links cleanly. Pros: openssl-free, no system-library dependency, no per-platform CI tweaks. Cons: changes the embeddings HTTPS stack from native-tls to rustls. Acceptable — cxpak only makes plain HTTPS calls to public-CA endpoints (HuggingFace, OpenAI/Voyage/Cohere), which rustls handles.
- **Option B — drop git2 default features only (git2-fix minimal):** Unblocks Windows, but leaves `reqwest` on `native-tls`, so the Linux release binary flips from vendored-static to system-dynamic OpenSSL. A reasonable choice if one wanted the smallest diff and accepted a `libssl` runtime dependency on Linux (or added a `libssl-dev`/vendored step to the Linux job), but it trades away the Linux binary's self-containment. Rejected to avoid the portability regression.
- **Option C — migrate git access to gitoxide (`gix`) and keep native-tls:** Removes libgit2/C from the git side entirely. Pros: no C toolchain for git. Cons: a real refactor (`src/git/mod.rs` plus ~15 `Repository::init` fixture sites) and it does nothing about reqwest's OpenSSL on Linux, so it solves the smaller half of the problem at higher cost. Deferred (see ADR for git2 usage, [ADR-0002](0002-git2-library-no-shelling-out.md)).

## Decision

Option A. Set `git2 = { version = "0.19", default-features = false }` and `reqwest = { version = "0.12", default-features = false, features = ["json", "blocking", "rustls-tls"] }`, then add `x86_64-pc-windows-msvc` (on `windows-latest`) to the release matrix, packaged as a `.zip` of `cxpak.exe`. Homebrew remains macOS/Linux; Windows distribution is the release `.zip` plus the already-working `cargo install cxpak`.

## Consequences

### Positive
- Windows gets a prebuilt binary for the first time.
- `openssl-sys` and `libssh2-sys` leave the dependency graph on **every** target. No OpenSSL build toolchain or `libssl` runtime dependency on any platform; release binaries are statically self-contained, including on Linux.
- Smaller dependency tree and faster, more reproducible builds across the board.

### Negative
- `git2` can no longer perform remote operations — invisible today (cxpak does only local git) but a constraint on future remote-history features.
- The embeddings HTTPS client now uses rustls rather than the OS-native TLS stack. For the public-CA endpoints cxpak calls this is equivalent; an environment that relied on OS trust-store customization for those calls would need its roots reflected in rustls.

### Neutral
- Supersedes the `vendored-openssl` rationale in [ADR-0002](0002-git2-library-no-shelling-out.md); the decision to use git2 for local access still stands.
- The daemon's `SIGTERM` handling is already `#[cfg(unix)]`-gated with a `ctrl_c()` fallback under `#[cfg(not(unix))]`, so the Windows target compiles without signal-handling changes.

## Revisit if
- cxpak needs to talk to a remote git server (clone/fetch/push) — re-enable git2's `https` feature (rustls-backed) or adopt `gix` (Option C).
- The vendored `libgit2-sys` build breaks on a future MSVC / `windows-latest` image — reconsider `gix` or a system-libgit2 path.
- A consumer needs OS-native TLS trust for the embeddings calls — reconsider the reqwest TLS backend (e.g. `rustls-tls-native-roots`).
