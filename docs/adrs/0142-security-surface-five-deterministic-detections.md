---
id: '0142'
title: 'Security surface: five deterministic regex/heuristic detections with exclusions and redaction'
status: ACCEPTED
date: 2026-04-01
triggered_by: v1.4.0 security surface analysis (cxpak_security_surface) for unprotected endpoints, secrets, SQLi, validation gaps, exposure
loop: implementation
---

# ADR-0142: Security surface: five deterministic regex/heuristic detections with exclusions and redaction

## Context

cxpak v1.4.0 added a security signal layer (`cxpak_security_surface`) without running a heavy SAST engine. It had to be deterministic, low-false-positive (excluding test/lock/example/doc files), and safe — never echoing full secrets. Unprotected-endpoint detection depended on real handler names, which forced an upstream fix to route detection.

## Options considered

- **Option A — Five regex/heuristic detections with file exclusions, secret redaction, and parameterized-query awareness (chosen):** Secrets (5 typed regex), SQL injection (per-language interpolation patterns, skipping parameterized queries), validation gaps (high-PageRank public functions with unvalidated string params), unprotected endpoints (routes whose handler lacks a nearby auth keyword), and exposure scores. Pros: fast, deterministic, no external SAST; exclusions and redaction reduce false positives and leakage; configurable `auth_patterns`. Cons: regex heuristics miss obfuscated/indirect cases and can false-positive. Preferred as the right scope for a context tool.
- **Option B — Integrate an external SAST tool:** A reasonable alternative would have been to shell out to semgrep or similar. Pros: far deeper analysis. Cons: heavy dependency, slow, non-deterministic across versions, out of scope for a context tool. Someone could prefer it where deep vulnerability coverage outweighs determinism and footprint.

## Decision

Add `src/intelligence/security.rs` computing a `SecuritySurface` from five deterministic detections:

- **Secret patterns** — 5 typed regex (AWS access key, GitHub PAT, password/secret/api_key/token assignment of >=8 chars, connection string, Slack token) with snippet redaction (first 4 chars + "...") and exclusion of test/lock/example/doc files.
- **SQL injection** — per-language interpolation regex that skips parameterized queries (`$1`, `?`, `:name`, `@param`).
- **Validation gaps** — only on high-PageRank (>=0.5) files.
- **Unprotected endpoints** — flagged when a route's handler has no `DEFAULT_AUTH_PATTERNS` keyword nearby.
- **Exposure scores** — `pub_symbol_count * inbound_edges * (1 - test_coverage)`.

This required first fixing `RouteEndpoint.handler` to extract real handler names across 12 frameworks (falling back to `"<anonymous>"` for inline closures) rather than the literal `"handler"`.

## Consequences

### Positive
- Deterministic, fast security signal with no external SAST dependency.
- Exclusions and redaction limit false positives and prevent secret leakage in output.
- `auth_patterns` configurable via `.cxpak.json`.

### Negative
- Regex/keyword heuristics miss indirect auth, dynamic SQL, and obfuscated secrets.
- Proximity-based auth detection can both miss and over-credit protection.

### Neutral
- `RouteEndpoint.handler` real-name extraction (12 frameworks) was made a prerequisite task and is reused by data-flow security-boundary tagging in v1.5.0.

## Revisit if
- False-positive/negative rates warrant a real SAST integration.
- The auth-proximity heuristic proves too coarse for common frameworks.

## Sources

- `2026-04-01-v140-implementation-plan.md`: "`security.rs` — `build_security_surface()` runs 5 deterministic detections: unprotected endpoints (real handler names from api_surface), input validation gaps (high-PageRank files), secret patterns (per-type regex, 5 types), SQL injection (interpolation detection per language), and exposure scores"
- `2026-04-01-v140-implementation-plan.md`: "For frameworks where no handler can be extracted (inline closures, anonymous functions), fall back to `\"<anonymous>\"` rather than `\"handler\"` so it is clearly not a real name."
- `2026-04-01-v140-implementation-plan.md`: "/// Returns true if the SQL string uses parameterized placeholders ($1, ?, :name, @param)."
