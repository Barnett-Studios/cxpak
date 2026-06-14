---
id: '0050'
title: Template-based codebase narrative generated from index signals (no LLM)
status: ACCEPTED
date: 2026-03-12
triggered_by: Structured overview output misses the 'what is this project?' question
loop: planning
---

# ADR-0050: Template-based codebase narrative generated from index signals (no LLM)

## Context
Proposed during v0.8.0 planning. Overview output is structured data (tree, modules, signatures) — good for LLMs but it does not answer "what is this project?" in prose. A short narrative could be derived deterministically from signals already in the index rather than by calling an LLM.

This was a Phase 4 / Task 10 stretch goal (planned file `src/output/narrative.rs`). It was never built: `src/output/narrative.rs` does not exist, and the shipped `OutputSections` struct (src/output/mod.rs) has seven sections — metadata, directory_tree, module_map, dependency_graph, key_files, signatures, git_context — with no narrative/prose field. This record documents the deferred decision, not a shipped feature.

## Options considered
- **Option A — template-based generation from index signals (chosen direction, not yet implemented):** generate 3-5 sentences from signals already in the index (primary language/framework, entry point, dependency count and notable dependencies, plus a size/complexity tier) filled into a template, with no LLM call. Pros: deterministic, offline, reproducible — consistent with cxpak's deterministic-context philosophy; no API token or latency. Cons: less fluent than LLM prose; the template must cover many project shapes.
- **Option B — LLM-generated narrative:** A reasonable alternative would have been to send index signals to an LLM to write the summary. Pros: more natural prose. Cons: requires network/API key; nondeterministic; latency and cost — contradicts cxpak being a deterministic context tool. The design doc only excludes LLMs in passing ("no LLM needed"); it was not formally deliberated as a weighed alternative.

## Decision
Adopt the template-based approach for the overview narrative section (3-5 sentences) populated from index signals, with no LLM call. This was explicitly a stretch goal — lower priority than the action and daemon work — and was deferred. As of the current codebase it has not been implemented: no `src/output/narrative.rs` and no narrative section in `OutputSections`.

## Consequences
### Positive
- The chosen direction (when built) would produce a deterministic, offline, reproducible narrative consistent with cxpak's deterministic philosophy.

### Negative
- Template prose would be less fluent than an LLM would produce, and the template must handle diverse project shapes to avoid awkward output.

### Neutral
- Explicitly a stretch goal, lower priority than the action and daemon.
- Not shipped: no narrative section exists in the current output pipeline.

## Revisit if
- The narrative feature is picked up in a later release.
- Template output proves too rigid across project types.
- A deterministic-enough local model becomes available for nicer prose.
