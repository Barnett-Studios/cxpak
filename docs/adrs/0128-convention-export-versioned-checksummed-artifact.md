---
id: '0128'
title: Versioned, SHA256-checksummed convention export with deterministic canonical JSON
status: ACCEPTED
date: 2026-04-01
triggered_by: v1.6.0 establishing a convention export standard (cxpak conventions export/diff)
loop: implementation
---

# ADR-0128: Versioned, SHA256-checksummed convention export with deterministic canonical JSON

## Context
Introduced in v1.6.0. The team wanted to persist a repo's `ConventionProfile` as a shareable, diffable artifact and detect when conventions change over time. A checksum that varied with JSON key ordering or with the generation timestamp would be useless for diffing — every run would appear changed. The hash therefore had to be computed over canonical, stable JSON of the profile only, excluding the timestamp and any non-deterministic field ordering.

## Options considered
- **Option A — `ConventionExport` with `version`/`generated_at`/`generator`/`repo`/`profile`/`checksum`; SHA256 over canonical (BTreeMap-sorted) JSON of the profile only:** `compute_checksum` recursively sorts object keys into a stable `Value` before hashing; `diff_exports` short-circuits on equal checksums, then does a top-level field diff. Pros: deterministic checksum independent of key order and timestamp, cheap change detection, human-readable versioned artifact. Cons: the field diff is top-level-category granular (reports which category changed, not which field). Someone could prefer it for the fast, stable equality check and the committable artifact.
- **Option B — Checksum over the whole export including `generated_at`:** Hash the serialized `ConventionExport` directly. Pros: simpler — no separate canonicalization step. Cons: the timestamp would change the checksum on every run, breaking diffing; the plan explicitly hashes only the profile for this reason. Someone could prefer it for its implementation simplicity if checksum stability across runs were not a requirement.
- **Option C — No checksum, diff profiles directly every time:** A reasonable alternative would have been to always perform a full field comparison. Pros: no hashing dependency. Cons: no fast equality short-circuit and no stable identity for an exported profile. Someone could prefer it to avoid adding a crypto dependency.

## Decision
Add `src/conventions/export.rs` defining `ConventionExport { version: "1.0", generated_at (RFC3339 via chrono), generator ("cxpak <version>"), repo, profile, checksum }`, persisted to `.cxpak/conventions.json`. `compute_checksum` hashes canonical JSON of the profile only — recursively sorting object keys via `BTreeMap` so neither key ordering nor the timestamp affects the hash. `diff_exports` short-circuits when checksums match, otherwise reports the changed top-level convention categories. Two CLI subcommands (`conventions export`, `conventions diff`) wrap these. `sha2` and `chrono` become non-optional dependencies.

## Consequences
### Positive
- Deterministic, order- and timestamp-independent checksum enables reliable diffing across runs.
- Versioned, human-readable artifact suitable for committing and sharing.
- Fast equality short-circuit before any field diffing.
### Negative
- The field diff is top-level-category granular, not per-field.
- Adds `sha2` and `chrono` as required (non-optional) dependencies.
### Neutral
- `ConventionExport`/`ConventionDiff` and the `.cxpak/conventions.json` artifact are documented in the shipped CLAUDE.md.

## Amendment (cxpak#70) — sorted keys are not sorted arrays

The decision above says the hash must be unaffected by "any non-deterministic field ordering", and
`compute_checksum` sorted object **keys** only. The lists under those keys kept whatever order their
producer emitted, and several producers drain a `HashMap` — `churn_30d` / `churn_180d` sorted by
modification count with no tiebreaker, `strict_layers` and the co-change edges not sorted at all — so
`HashMap`'s per-process random iteration order reached the artifact. Ten cold exports of an unchanged
four-file repo produced **ten different checksums**, and `conventions diff` reported drift on every
run whose cache was cold: a CI drift gate that fires always, which is as useless as one that never
fires and fails in the direction that gets it switched off.

Two changes, and the pairing is deliberate:

- the producers sort **totally** (count then path; from/to; file_a/file_b), so the emitted artifact
  is stable for a human diffing two committed files, and ranked lists keep their rank;
- the canonical image the checksum hashes orders **arrays as well as keys**, so a field that ever
  becomes non-deterministic again cannot move the checksum. Reordering a list without changing its
  contents is not drift.

`diff_exports` compares that same canonical image, so the checksum and the field-by-field walk cannot
disagree. The "checksum differs, fields identical" summary no longer blames `generated_at`, which
this ADR excludes from the hash by construction and which therefore could never have caused it.

## Revisit if
- Per-field (not per-category) diffing is required.
- The export schema needs a `2.0` version with breaking changes.
