---
id: '0064'
title: Filename match takes priority over extension in detect_language
status: ACCEPTED
date: 2026-03-18
triggered_by: Extensionless files like Dockerfile and Makefile returned None from detect_language
loop: planning
---

# ADR-0064: Filename match takes priority over extension in detect_language

## Context

In v0.10.0, `detect_language()` relied on `path.extension()`, which returns `None` for
extensionless files such as `Dockerfile`, `Makefile`, and `GNUmakefile`. Two related
ambiguities also needed resolution: `.m` maps to both Objective-C and MATLAB, and
`.zsh` was excluded because `tree-sitter-bash` produces incorrect parses for zsh syntax.

Note a divergence between the v0.10.0 design doc and the shipped code: the design
planned to map `.m` to Objective-C unconditionally, but the implementation instead
disambiguates `.m` by content sniffing (see Decision).

## Options considered

- **Option A — check `file_name()` first, then extension:** Match
  `Dockerfile`/`Makefile`/`GNUmakefile` (and `Dockerfile.*`) by name before falling
  through to the extension match. Pros: handles extensionless files predictably via a
  simple ordered match. Cons: filename rules must be maintained as a separate match arm.

- **Option B — content sniffing / shebang detection:** A reasonable alternative would
  have been to inspect file contents to disambiguate language. Someone could prefer it
  to resolve the `.m` and `.zsh` ambiguities accurately. In practice this approach was
  adopted specifically for the `.m` case (`disambiguate_m_extension`), rather than
  rejected — though it was not used for the filename or `.zsh` resolution, which stay
  rule-based for predictability.

## Decision

Extend `detect_language()` to check `file_name()` first (`Dockerfile`, `Makefile`,
`GNUmakefile`, `Dockerfile.*`), then fall back to extension. `.R` is handled
case-sensitively before lowercasing. `.zsh` is not mapped. `.m` is disambiguated by
content sniffing via `disambiguate_m_extension()` (`src/scanner/mod.rs:186`, called at
`:251`): files whose head (first 256 bytes, trimmed) starts with `#` or `@` map to
`objc`, otherwise `matlab` (the default). This supersedes the design doc's planned
unconditional-Objective-C mapping for `.m`.

## Consequences

### Positive
- Extensionless build/container files are detected.
- Filename and extension behavior is simple and predictable; MATLAB `.m` files are
  correctly classified by default rather than misread as Objective-C.

### Negative
- An Objective-C `.m` file whose head does not begin with `#` or `@` falls through to
  `matlab`.
- zsh scripts get no parse.

### Neutral
- `.m` disambiguation reads the file head, so it is the one detection path that touches
  file content rather than path alone.

## Revisit if
- The `.m` content-sniffing heuristic proves insufficient and needs refinement.
- zsh diverges enough from bash that a dedicated grammar is warranted.
