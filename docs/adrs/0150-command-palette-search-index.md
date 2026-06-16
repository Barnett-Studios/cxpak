---
id: '0150'
title: Rust-precomputed search index with subsequence fuzzy match and 20k entry cap
status: ACCEPTED
date: 2026-04-17
triggered_by: Command palette (Cmd+K) needs searchable files/symbols/modules/views
loop: planning
---

# ADR-0150: Rust-precomputed search index with subsequence fuzzy match and 20k entry cap

## Context

Designed for v2.1.0. The command palette (Cmd+K) must search across all files, public symbols, module prefixes, and views. The team had to decide where the index is computed, how matching ranks, and how to bound cost on large repos so the UI thread stays responsive.

## Options considered

- **Option A — Precompute `SearchEntry` index in Rust, embed as JSON, fuzzy-match in JS:** `build_search_index()` emits a sorted `Vec<SearchEntry>{label,kind,context,detail,target}`; JS does subsequence fuzzy match with tiered ranking, capped at 20,000 entries and 50 displayed results. Pros: deterministic ordering computed once; no server calls at runtime; bounded UI work. Cons: the cap drops the lowest-PageRank files/symbols on 50k-file monorepos. This is the chosen and shipped design.
- **Option B — Compute the index client-side from embedded raw view data:** Derive searchable entries in JS from the already-embedded dashboard/architecture data. Pros: no separate index payload. Cons: duplicated logic in JS; nondeterministic ordering; harder to test in Rust. A reasonable alternative would have been this to avoid a second payload; it was rejected to keep a single, testable Rust source of truth.

## Decision

Compute the search index once in Rust via `build_search_index()` producing a deterministically sorted `Vec<SearchEntry>` (sorted by `(kind, label, context)`), embed it as a JSON tag, and have the palette perform JS-side subsequence fuzzy matching with a tiered rank (exact > prefix > substring > subsequence) plus a composite tie-break key. Cap the index at 20,000 entries (keep all views + module prefixes, then files/symbols by PageRank descending) and display at most 50 results.

## Consequences

### Positive
- Deterministic, server-call-free palette; bounded scan time protects the UI thread on huge repos.
- Single Rust source of truth, testable without a browser.

### Negative
- On 50k-file monorepos the lowest-PageRank files/symbols are excluded (with a warn log).

### Neutral
- The empty-query state shows 6 views + top-10 PageRank files (max 16).

## Revisit if
- The palette feels incomplete on very large repos due to the 20k cap.
- Subsequence matching proves insufficient and a real fzf-style scorer is wanted.
