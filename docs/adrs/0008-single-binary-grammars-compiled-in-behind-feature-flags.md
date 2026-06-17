---
id: '0008'
title: Ship a single binary with all tree-sitter grammars compiled in, each behind a Cargo feature flag
status: ACCEPTED
date: 2026-03-05
triggered_by: Need to support many languages without runtime grammar loading or external dependencies
loop: planning
---

# ADR-0008: Ship a single binary with all tree-sitter grammars compiled in, each behind a Cargo feature flag

## Context

Tree-sitter grammars can be loaded dynamically at runtime or compiled in statically. The v0.1.0 design picks static compilation: cxpak ships as a single binary with all supported language grammars compiled in, each behind a `lang-{name}` Cargo feature flag with all enabled by default and optionally slimmable per build. The design explicitly lists "Plugin/dynamic grammar loading" under Not In Scope.

## Options considered

- **Option A — All grammars compiled in, gated by feature flags:** Statically link every grammar; per-language `lang-{name}` flags let consumers slim the build. Pros: a self-contained binary with no runtime dependencies, slimmable per build. Cons: a larger default binary, and adding a language requires a recompile. Someone could prefer this for trivial distribution and reproducible builds.
- **Option B — Dynamic/plugin grammar loading:** Load grammar shared libraries at runtime. Pros: add languages without recompiling and keep the core binary smaller. Cons: runtime dependency management and harder distribution. The design considered this and placed it explicitly under Not In Scope. Someone could prefer it to add third-party grammars without rebuilding cxpak.

## Decision

Compile all supported tree-sitter grammars into a single binary, each behind a `lang-{name}` Cargo feature flag with all enabled in `default`, and exclude dynamic/plugin grammar loading from scope.

## Consequences

### Positive
- Self-contained binary, trivial to distribute.
- Consumers can slim the build by disabling language features.

### Negative
- The binary grows with each language added.

### Neutral
- Establishes the `lang-{name}` feature-flag convention reused for every language added later.
- A general WASM plugin SDK (behind a `plugins` Cargo feature flag, not in `default`, with a stubbed loader) was later added. This is adjacent to — not a direct reversal of — the dynamic-grammar-loading stance: it is a broader capability/finding plugin system, and dynamic grammar loading specifically remains out of the default path.

## Revisit if
- Binary size becomes a distribution problem.
- A genuine need for third-party or runtime grammars emerges (a related WASM plugin SDK later arrived, though not for grammars specifically).
