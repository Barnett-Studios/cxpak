---
id: '0068'
title: Adopt 30 additional tree-sitter grammars to reach 42-language coverage
status: ACCEPTED
date: 2026-03-18
triggered_by: Expanding cxpak parsing coverage from 12 to 42 languages
loop: planning
---

# ADR-0068: Adopt 30 additional tree-sitter grammars to reach 42-language coverage

## Context

As of the v0.10.0 design, cxpak supported 12 languages via tree-sitter. The design proposed adding 30 more grammars, all sourced from crates.io, each pinned and verified to compile against `tree-sitter = "0.25"`. Languages were split into Tier 1 (full symbol extraction: functions, classes, methods, imports, exports) and Tier 2 (structural extraction for markup/config/data).

Specific grammar selections embed sub-decisions that shipped in code: PHP uses `LANGUAGE_PHP` (not `LANGUAGE_PHP_ONLY`); XML uses `LANGUAGE_XML` (not `LANGUAGE_DTD`); OCaml is registered as two languages (implementation and interface) sharing one `lang-ocaml` feature flag.

## Options considered

- **Option A — Add 30 tree-sitter grammars with a Tier 1 / Tier 2 split:** Each grammar is an optional crate behind a per-language `lang-*` feature flag, all enabled in `default`; Tier 1 gets full extraction, Tier 2 gets structural-unit extraction. Pros: broad coverage, reuses the existing `LanguageSupport` trait pattern unchanged, and `--no-default-features` plus selected flags keeps minimal builds fast. Cons: adds ~30 C parsers, so clean builds take 3–5 minutes, and 30 new external crate dependencies to track for security and version drift. (Grounded — this is the design as written and shipped.)

- **Option B — Regex/heuristic extraction for non-Tier-1 languages:** Skip tree-sitter for config/markup and extract structure with regexes. A reasonable alternative would have been this to avoid the C-grammar compile cost. Cons: fragile and inconsistent with the existing tree-sitter-based parser architecture. (Reconstructed; not formally evaluated.)

- **Option C — Stay at 12 languages:** Do not expand coverage. A reasonable alternative would have been this to avoid build-time and maintenance cost. Cons: misses most real-world polyglot repositories. (Reconstructed; not formally evaluated.)

## Decision

Add all 30 grammars as optional crates behind per-language `lang-*` feature flags, all enabled in `default`. Tier 1 languages get full extraction; Tier 2 languages (CSS, SCSS, Markdown, JSON, YAML, TOML, Dockerfile, HCL, Protobuf, Svelte, Makefile, HTML, GraphQL, XML) get structural-unit extraction.

Confirmed shipped: 42 languages are now supported (per `CLAUDE.md`), with all `lang-*` flags in `default` and the Tier 1 / Tier 2 split realized in `src/parser/languages/`.

## Consequences

### Positive
- 42-language coverage.
- Per-language opt-out (`--no-default-features` plus selected flags) keeps minimal builds fast.
- Reuses the existing `LanguageSupport` trait pattern unchanged.

### Negative
- Clean builds slowed to 3–5 minutes by compiling 30 C grammars from scratch.
- 30 new external crate dependencies to track for security and version drift.

### Neutral
- An `all-languages` feature alias was deemed unnecessary because `default` already enables everything.

## Revisit if
- A grammar crate stops compiling against a future tree-sitter version.
- Build time becomes a CI bottleneck.
- A higher-quality grammar appears for an already-covered language.
