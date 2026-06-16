---
id: '0004'
title: Per-language extraction behind a LanguageSupport trait with a runtime registry
status: ACCEPTED
date: 2026-03-05
triggered_by: Need a clean, uniform boundary for adding languages and extracting symbols/imports/exports
loop: planning
---

# ADR-0004: Per-language extraction behind a LanguageSupport trait with a runtime registry

## Context

In v0.1.0, each supported language must extract symbols, imports, and exports from a tree-sitter AST. cxpak needs a uniform boundary for this so per-language logic stays isolated and adding a language is mechanical. The design makes this a trait — "Language trait — each language implements extraction of symbols, imports, exports. Clean boundary if plugin architecture is ever needed." The implementation plan formalizes it as the `LanguageSupport` trait (`ts_language()`, `extract()`, `name()`) plus a `LanguageRegistry` keyed by language name, with registration gated by feature flags.

## Options considered

- **Option A — Trait + registry abstraction:** Define a `LanguageSupport` trait and register implementations in a `HashMap` registry keyed by language name. Pros: a clean per-language boundary, a uniform `extract()` contract returning `ParseResult`, easy mechanical addition of languages, and a boundary that is plugin-ready. Cons: indirection via dynamic dispatch (`Box<dyn LanguageSupport>`). Someone could prefer this for testability and the clean plugin boundary.
- **Option B — Per-language match/switch in one parser module:** A reasonable alternative would have been a single parser function that branches on a language string. Pros: no trait machinery. Cons: a monolithic module, poor separation of concerns, harder per-language testing, and no clean boundary to hang a plugin architecture on. (This alternative was not formally evaluated in the design; it is reconstructed here as the natural counterfactual.)

## Decision

Define a `LanguageSupport` trait (`ts_language()`, `extract() -> ParseResult`, `name()`) implemented once per language and registered in a `LanguageRegistry`, giving a clean boundary "if a plugin architecture is ever needed." Registry registration is gated per language by feature flags.

## Consequences

### Positive
- Each language is an isolated unit that can be tested separately.
- Adding a language is a mechanical, documented recipe.
- The boundary later enabled the WASM plugin SDK.

### Negative
- Dynamic dispatch via `Box<dyn LanguageSupport>`.

### Neutral
- `ParseResult { symbols, imports, exports }` becomes the universal extraction contract every language implements.
- Registry registration is feature-flag gated, so the default build set of languages is controlled by Cargo features.

## Revisit if
- Extraction needs cross-language context that the per-file, per-language trait cannot provide.
