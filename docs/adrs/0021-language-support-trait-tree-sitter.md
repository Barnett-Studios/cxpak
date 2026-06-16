---
id: '0021'
title: New languages are added via the LanguageSupport trait backed by per-language tree-sitter crates behind feature flags
status: ACCEPTED
date: 2026-03-09
triggered_by: v0.3.0 expands language coverage (Ruby, C#, Swift, Kotlin) and the pattern for doing so needs to be uniform and optional.
loop: implementation
---

# ADR-0021: New languages are added via the LanguageSupport trait backed by per-language tree-sitter crates behind feature flags

## Context

v0.3.0 expands language coverage with Ruby, C#, Swift, and Kotlin. Each language ships as an optional `tree-sitter-{lang}` dependency gated by a `lang-{name}` feature, with a `src/parser/languages/{name}.rs` implementing a shared extraction trait and registered centrally. The goal is a uniform, mechanical recipe so future languages follow the same path and can be compiled out via features.

## Options considered

- **Option A — Per-language tree-sitter crate + `lang-{name}` feature flag + `LanguageSupport` impl:** Add the optional dep, put the feature in the default set, map the extension in the scanner, implement the trait, and register it centrally. Pros: uniform extensibility; languages can be compiled out via features; tree-sitter gives real AST-level extraction. Cons: each language adds a dependency and feature, plus tree-sitter grammar version churn. Chosen.
- **Option B — Regex/heuristic extraction per language:** A reasonable alternative would have been to extract symbols with per-language regexes instead of tree-sitter grammars. Pros: no grammar dependencies to track or version. Cons: fragile and loses the structural fidelity tree-sitter provides; someone could prefer it to avoid the dependency and binary-size cost of many grammars.

## Decision

Standardize the add-a-language recipe: add `tree-sitter-{lang}` as an optional dep, define a `lang-{name}` feature (added to `default`), map the extension in scanner `detect_language`, implement `LanguageSupport` in `src/parser/languages/{name}.rs`, and register it centrally (`src/parser/languages/mod.rs` and `src/parser/mod.rs`).

Applied to Ruby, C#, Swift, and Kotlin in v0.3.0; the same pattern now backs the project's full set of shipped languages (42 documented in the architecture overview). Confirmed shipped: `Cargo.toml` declares the optional deps and `lang-*` features in `default`; `src/scanner/mod.rs` maps the extensions; `src/parser/language.rs` defines `LanguageSupport`; per-language impls and central registration exist for all four.

## Consequences

### Positive
- Adding a language is a mechanical, well-defined process.
- Languages are independently feature-gated.
- Real AST-level extraction via tree-sitter.

### Negative
- Each language adds a crate dependency and grammar-version maintenance burden.

### Neutral
- Visibility rules are encoded per-language (e.g., Ruby `private`/`protected` keywords).

## Revisit if
- tree-sitter grammar version conflicts become unmanageable.
- Binary size from many grammars becomes a concern.
