---
id: '0002'
title: Access git history via the git2 library rather than shelling out to the git binary
status: ACCEPTED
date: 2026-03-05
triggered_by: Git context (recent commits, churn, contributors, blame) must be extracted reliably across environments
loop: planning
---

# ADR-0002: Access git history via the git2 library rather than shelling out to the git binary

## Context

In v0.1.0, the Git Context section of the output needs commit history, per-file churn, and contributor aggregation. Extracting this either means invoking the `git` CLI and parsing its stdout, or reading the repository directly through a library. The design chooses library-level access through `git2-rs` (libgit2 bindings): "Git ops via git2-rs — no shelling out to git. Library-level access."

## Options considered

- **Option A — git2 (libgit2 bindings):** Walk revisions with a revwalk, diff trees against their parent for churn, and read authors directly through libgit2. Pros: no subprocess, structured access to commits/diffs/authors, no fragile text parsing, and portability across environments that may not have `git` on PATH. Cons: a heavier dependency (libgit2/openssl) and a more verbose API. Someone could prefer this for robustness and self-containment.
- **Option B — Shell out to the git CLI:** Run `git log` / `git blame` and parse stdout. Pros: simple, mirrors the commands developers already know. Cons: requires a `git` binary on PATH, depends on parsing human-oriented output that can drift between versions, and incurs subprocess overhead per call. Someone could prefer this to avoid the libgit2 build burden.

## Decision

Use the `git2` crate for all git-context operations: revwalk for history, diff-against-parent for per-file churn, and author/contributor aggregation in a single pass — "no shelling out to git." `extract_git_context()` caps history at `max_commits` (20 for overview) and aggregates churn and contributors in one revwalk.

## Consequences

### Positive
- No dependency on a `git` binary being present on PATH; access is structured and parse-free.

### Negative
- Pulls in libgit2/openssl; the shipped `Cargo.toml` enables the `git2` `vendored-openssl` feature to keep the build self-contained.

### Neutral
- `extract_git_context()` caps at `max_commits` (20 for overview) and aggregates churn plus contributors in a single revwalk.

## Revisit if
- The libgit2/openssl build burden outweighs the benefit.
- A required git feature turns out to be unavailable via libgit2.
