---
id: '0020'
title: Detail file extensions track the --format flag (.md/.json/.xml)
status: ACCEPTED
date: 2026-03-09
triggered_by: Pack mode hardcoded .md detail filenames, so JSON/XML runs wrote markdown content into .md files regardless of the requested format.
loop: implementation
---

# ADR-0020: Detail file extensions track the --format flag (.md/.json/.xml)

## Context

As of v0.3.0, pack mode (ADR-0019) writes full per-section analysis to detail files under `.cxpak/`. The renderer `render_single_section` already produces output per `--format`, but the detail file names and the omission pointers that reference them were hardcoded to `.md`. As a result, JSON and XML runs wrote correctly-formatted content into files with a misleading `.md` extension, and the omission pointers referenced the wrong filename.

## Options considered

- **Option A — Derive the extension from `OutputFormat` via a `detail_file_ext` helper:** Map `Markdown -> md`, `Xml -> xml`, `Json -> json` and thread the result through both filenames and the omission-pointer targets. Pros: detail files are self-consistent with their content and the requested format; pointers reference the right filename. Cons: filenames now depend on a runtime flag rather than being literals. Chosen.
- **Option B — Always write `.md` detail files:** A reasonable alternative would have been to keep the markdown extension regardless of format. Pros: simpler — filenames stay literal. Cons: produces a misleading extension on JSON/XML content, which is the defect being fixed; someone could prefer it only to avoid computing the extension at runtime.

## Decision

Add `detail_file_ext(format: &OutputFormat) -> &'static str` (`Markdown -> "md"`, `Xml -> "xml"`, `Json -> "json"`) and use `format!("tree.{ext}")`, `format!("modules.{ext}")`, etc. for both the detail filenames and the omission-pointer targets, so `.cxpak/` files carry the correct extension for the chosen `--format`.

Shipped in `src/commands/overview.rs`: `detail_file_ext` is defined and threaded into every detail filename; the marker/pointer helpers in `src/budget/degrader.rs` take the detail filename, so pointers inherit the correct extension. A unit test (`test_detail_file_ext`) asserts the three mappings.

## Consequences

### Positive
- Detail files have correct, parseable extensions per format.
- Omission pointers reference the right filename.

### Negative
- Filename strings are now computed rather than literals.

### Neutral

## Revisit if
- A new output format is added (needs a new ext mapping in `detail_file_ext`).
