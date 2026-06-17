---
id: '0104'
title: Conventions become a portable, versioned, checksummed export artifact (.cxpak/conventions.json)
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.6.0 makes conventions a portable standard for commit/diff/CI use
loop: planning
---

# ADR-0104: Conventions become a portable, versioned, checksummed export artifact (.cxpak/conventions.json)

## Context
Convention profiles are computed in-memory at index time. The v1.6.0 ("The Platform") roadmap wants them to be a portable standard: committed to a repo for onboarding, diffable between branches/PRs, enforceable in CI, and comparable across repos. To support those use cases the profile must be serializable to a stable, versioned, integrity-checked on-disk artifact rather than recomputed live every time.

## Options considered
- **Option A — Versioned `ConventionExport` with SHA256 checksum to `.cxpak/conventions.json`:** wrap the profile in a struct carrying `version`, `generated_at` (ISO 8601), `generator` (e.g. `"cxpak 1.6.0"`), `repo`, the `profile`, and a `checksum` (SHA256 of the profile content). CLI commands `conventions export` and `conventions diff` operate against it. Pros: commit-to-repo onboarding, branch/PR convention diffing, CI drift enforcement against a committed baseline, cross-repo comparison, and a checksum that detects tampering or corruption; the `version` field allows forward-compatible evolution. Cons: another on-disk artifact format to version and maintain. (Grounded — this is the shipped design.)
- **Option B — Recompute conventions live every time they are needed:** never persist; always derive from the current index. A reasonable alternative would have been to keep conventions purely ephemeral and avoid any artifact maintenance. Someone could prefer this to dodge a new versioned format. Rejected because live recomputation gives no committed baseline for CI, no branch diffing, and no cross-repo comparison — the entire point of the v1.6.0 work. (Reconstructed — not formally evaluated in the source.)

## Decision
Persist conventions as a portable artifact:

```
ConventionExport {
    version,        // "1.0"
    generated_at,   // ISO 8601
    generator,      // "cxpak 1.6.0"
    repo,
    profile,        // ConventionProfile
    checksum,       // SHA256 of profile content
}
```

written to `.cxpak/conventions.json`. The CLI commands `cxpak conventions export` and `cxpak conventions diff` support committing conventions to a repo, diffing between branches, CI enforcement against a committed baseline, and cross-repo comparison. The checksum is computed over a stable (ordered) serialization of the profile so it is reproducible.

## Consequences
### Positive
- New team members see the codebase's conventions instantly from the committed file.
- PRs can diff convention changes; CI can fail on drift against the committed baseline.
- The SHA256 checksum guards artifact integrity against corruption or tampering.

### Negative
- A new versioned on-disk format must be maintained and evolved compatibly.

### Neutral
- The artifact is written atomically (temp file then rename) and lives under `.cxpak/`, sharing the directory's lifecycle with other cxpak state.

## Revisit if
- The export schema needs to evolve in a breaking way (the `version` field is the lever).
- Checksum-based diffing proves insufficient and semantic convention diffs are required.
