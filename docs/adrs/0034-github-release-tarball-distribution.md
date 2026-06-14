---
id: '0034'
title: First-use distribution via GitHub Releases tarballs with uname-based platform mapping
status: ACCEPTED
date: 2026-03-11
triggered_by: ensure-cxpak must obtain a binary for the user's platform when none is present
loop: planning
---

# ADR-0034: First-use distribution via GitHub Releases tarballs with uname-based platform mapping

## Context

In the cxpak v0.4.0 plugin, when neither `PATH` nor the cache has the binary, `ensure-cxpak` downloads from GitHub Releases. The design fixes the platform-to-tarball mapping for four targets (Darwin arm64/x86_64, Linux aarch64/x86_64) and explicitly drops Windows because Claude Code does not run on Windows natively. The implementation uses the `releases/latest/download` URL with `curl -fsSL` and `tar` extraction.

## Options considered

- **Option A — GitHub Releases latest-download tarballs, 4 targets, no Windows (chosen):** `uname -s`/`-m` maps to one of four target triples; download `cxpak-<target>.tar.gz` from `releases/latest/download`. Pros: no package manager required, always latest, simple URL. Cons: no version pinning, couples to GitHub availability, no Windows. Someone could prefer this for the minimal prerequisites.
- **Option B — Homebrew/cargo install:** A reasonable alternative would have been installing through a package manager for version management and signed channels. Rejected at design time because it requires the package manager to be present; one could prefer it for reproducibility and pinning. (This is in fact the approach the shipped resolver later adopted.)

## Decision

Download from GitHub Releases via `https://github.com/<repo>/releases/latest/download/cxpak-<target>.tar.gz`, mapping Darwin + arm64/x86_64 and Linux + aarch64/x86_64 to their target triples and failing on any other OS/arch. Windows is explicitly unsupported.

## Consequences

### Positive
- No package manager prerequisite for first use.
- Always pulls the latest release.

### Negative
- No version pinning in this scheme; the shipped `ensure-cxpak` later replaced raw tarball download with Homebrew/cargo install plus a pinned `REQUIRED_VERSION` check.

### Neutral
- Four supported targets only; Windows excluded by design.

## Revisit if
- Version pinning becomes necessary.
- A signed/package-manager distribution is preferred.
- Windows support is required.
