---
id: '0070'
title: 'Hierarchical query expansion: always-on core synonyms + heuristic-activated domain synonyms'
status: ACCEPTED
date: 2026-03-19
triggered_by: Literal query terms miss semantically related code (e.g. 'auth' missing 'authentication')
loop: planning
---

# ADR-0070: Hierarchical query expansion: always-on core synonyms + heuristic-activated domain synonyms

## Context

Introduced in the cxpak v0.11.0 context-quality design. Literal query terms miss semantically related code: a query for `auth` should surface `authenticate`, and a query for `db` should surface database code.

The design uses query expansion with two layers: ~30 always-active core synonym entries (`CORE_SYNONYMS`) plus 8 domain-specific synonym maps (`DOMAIN_SYNONYMS`) covering Web, Database, Auth, Infra, Testing, Api, Mobile, and ML. Domain maps are activated only when repo file-pattern heuristics detect the relevant domain. Domain detection runs once at index build time and is cached on `CodebaseIndex.domains`; it keys off file extensions/filenames from paths rather than the language field. Expansion is applied only to the `term_frequency` and `symbol_match` signals — deliberately not to `path_similarity` (paths should match exactly).

## Options considered

- **Option A — Core (always-on) + domain (heuristic-activated) synonym maps:** Static synonym maps, with domains detected from repo file patterns at build time and cached. Pros: zero config, adapts to the repo automatically, detection cost paid once. Cons: static lists need manual curation, and heuristics can mis-detect domains. (Grounded — this is the design as written and shipped.)

- **Option B — Embedding-based semantic expansion:** Use vector similarity to expand queries. A reasonable alternative would have been this to learn relationships automatically. Cons: heavier and non-deterministic. (Embeddings were later added separately, as the 7th scoring signal, rather than as a query-expansion mechanism.) (Reconstructed; not formally evaluated as a query-expansion option.)

- **Option C — Apply expansion to all scoring signals including path:** Expand for `path_similarity` too. Pros: uniform treatment across signals. Cons: `auth` would wrongly match a path like `authorization/config.toml` by synonym, breaking exact path matching. (Grounded — this trade-off is explicitly discussed in the design's decision table.)

## Decision

Implement `expand_query(query, domains)` over a static `CORE_SYNONYMS` map plus `DOMAIN_SYNONYMS` keyed by 8 heuristically-detected domains. Detect domains once at index build and cache them on `CodebaseIndex`. Apply expansion only to `term_frequency` and `symbol_match`, not to `path_similarity`, `import_proximity`, or `recency`.

Confirmed shipped: `src/context_quality/expansion.rs` (30-key `CORE_SYNONYMS`, 8-domain `Domain` enum and `DOMAIN_SYNONYMS`, `detect_domains()` reading file paths). Expansion is wired into only `symbol_match` and `term_frequency` in `src/relevance/mod.rs`; `path_similarity` receives the raw query.

## Consequences

### Positive
- Zero-config semantic broadening that adapts to the repo.
- Domain detection cost is paid once at build, not per query.
- Exact path matching is preserved.

### Negative
- Synonym lists are hand-maintained and can drift.
- Heuristic domain detection can produce false positives (ML requires `.ipynb` to mitigate this).

### Neutral
- Synonyms are single atomic tokens chosen to match `split_identifier` output.

## Revisit if
- Synonym maintenance burden grows.
- Domain heuristics misfire on real repos.
- Embedding-based expansion supersedes the static synonym lists.
