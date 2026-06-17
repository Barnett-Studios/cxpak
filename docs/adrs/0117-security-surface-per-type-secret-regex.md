---
id: '0117'
title: Security surface uses five deterministic detections with per-type secret regexes, not entropy matching
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.4.0 security surface analysis
loop: planning
---

# ADR-0117: Security surface uses five deterministic detections with per-type secret regexes, not entropy matching

## Context

Released in v1.4.0. The security surface analysis must flag likely security issues in a codebase. For secret detection specifically, the choice is between generic high-entropy string matching (catches arbitrary formats but produces many false positives) and curated per-type regex patterns (lower recall on unknown formats, far fewer false positives). The broader surface needs detections that are deterministic and explainable — each finding traceable to a named rule — rather than probabilistic flags.

## Options considered

- **Option A — five deterministic detections, per-type secret regexes with curated excludes (chosen):** (1) unprotected endpoints via auth-chain reachability; (2) input-validation gaps in high-PageRank public functions taking `String` params with no validation calls; (3) secret patterns via per-type regex (AWS `AKIA`, GitHub `ghp_`, generic assignment, connection strings, Slack `xox`); (4) SQL injection from interpolation in embedded SQL; (5) per-file exposure score. Excludes tests, `.env.example`, docs, lock files; auth and secret patterns configurable via `.cxpak.json`. Pros: deterministic and explainable, named pattern per match, low false-positive rate, configurable. Cons: the regex set must be maintained; misses secrets not matching a known pattern; unprotected-endpoint detection depends on real handler-name extraction.
- **Option B — generic entropy-based secret scanning:** The design explicitly contrasts against flagging any high-entropy string as a possible secret. Pros: catches arbitrary, unknown secret formats. Cons: high false-positive rate. Rejected — the doc explicitly chooses per-type regex over entropy matching.

## Decision

Build the security surface from five deterministic detections:

1. **Unprotected endpoints** — HTTP routes whose handler-to-registration call chain does not pass a known auth pattern (configurable via `.cxpak.json` `auth_patterns`).
2. **Input validation gaps** — high-PageRank public functions taking `String` params with no validation calls.
3. **Secret patterns** — per-type regex, explicitly NOT generic entropy matching: AWS access key `AKIA[0-9A-Z]{16}`, GitHub PAT `ghp_[a-zA-Z0-9]{36}`, generic password/secret/api_key/token assignment, connection strings `://[^:]+:[^@]+@`, Slack `xox[baprs]-...`. Excludes tests, `.env.example`, docs, and lock files.
4. **SQL injection** — string interpolation in `embedded_sql` edges, per language.
5. **Per-file exposure score.**

Unprotected-endpoint detection requires the v1.3.0 work that fixed `detect_routes()` to extract real handler names per framework (the field was previously always the literal `"handler"`) and made the call graph store both directions. Shipped in `src/intelligence/security.rs`.

## Consequences

### Positive
- Detections are deterministic, named, and explainable.
- Per-type regex keeps secret false positives low.
- Auth and secret patterns are configurable for custom setups.

### Negative
- The regex set must be maintained and misses unknown secret formats.
- Unprotected-endpoint detection is gated on real handler-name extraction (a v1.3.0 prerequisite).

### Neutral
- Curated exclusion list (tests, `.env.example`, docs, lock files) deliberately trades some recall for precision.

## Revisit if
- Per-type regexes miss too many real secrets and entropy matching is reconsidered.
- Auth-chain reachability proves unreliable across frameworks.
