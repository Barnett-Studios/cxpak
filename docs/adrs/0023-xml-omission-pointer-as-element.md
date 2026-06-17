---
id: '0023'
title: XML output emits omission pointers as <detail-ref> elements rather than HTML comments
status: ACCEPTED
date: 2026-03-09
triggered_by: The XML renderer escapes HTML-comment omission pointers into &lt;!-- ... --&gt;, making detail-file references unreadable in XML output.
loop: implementation
---

# ADR-0023: XML output emits omission pointers as <detail-ref> elements rather than HTML comments

## Context

Omission pointers (introduced with pack mode, ADR-0019) are written as HTML comments (`<!-- ... -->`). The XML renderer's `emit_section` runs every line through `escape_xml`, which mangles those pointer comments into `&lt;!-- ... --&gt;`, making the detail-file references unreadable in XML output.

## Options considered

- **Option A — Detect pointer lines and emit them as `<detail-ref>` XML elements:** In `emit_section`, recognize lines wrapped in `<!-- ... -->` and convert them to a `<detail-ref>` element with escaped inner text. Pros: pointers stay readable and well-formed in XML, and the references survive. Cons: `emit_section` now carries format-specific special-casing coupled to the marker format. Chosen.
- **Option B — Skip escaping for comment lines:** A reasonable alternative would have been to leave HTML comment lines unescaped in XML output. Pros: preserves the comment verbatim. Cons: mixing HTML comments into XML is awkward and less idiomatic than a real element; someone could prefer it to avoid adding element-emission logic.

## Decision

In `src/output/xml.rs` `emit_section`, detect lines that are HTML-comment omission pointers (trimmed line starts with `<!-- ` and ends with ` -->`) and emit them as `<detail-ref>...</detail-ref>` elements with the inner text escaped, instead of escaping the whole comment. Non-pointer lines continue through `escape_xml`.

Confirmed shipped: `emit_section` implements the detection and element emission, and a unit test (`test_xml_omission_pointer`) asserts the output contains `<detail-ref>` and not `<!--`.

## Consequences

### Positive
- XML detail-file references are readable and structured.
- No double-escaping of pointer markers.

### Negative
- `emit_section` carries pointer-detection logic coupled to the marker format.

### Neutral

## Revisit if
- The omission-pointer marker format changes and breaks the detection heuristic.
