---
id: '0066'
title: Expose search as POST /search rather than GET
status: ACCEPTED
date: 2026-03-18
triggered_by: Regex patterns contain characters that are problematic in URL query strings
loop: planning
---

# ADR-0066: Expose search as POST /search rather than GET

## Context

In v0.10.0 the search capability is also exposed over the HTTP server. The design
chose POST with a JSON body over GET because regex patterns contain characters (e.g.
`?`, `+`, `*`, brackets) that are awkward or unsafe to encode in URL query strings.
The sibling read routes (`/health`, `/stats`, `/overview`, `/trace`, `/diff`,
`/api_surface`) remain GET, so the POST choice is a deliberate divergence for `/search`.

## Options considered

- **Option A — `POST /search` with a JSON body:** Pattern and options travel in a JSON
  request body. Pros: avoids URL-encoding pitfalls for regex metacharacters. Cons: not
  idempotent-by-convention the way a GET read is.

- **Option B — `GET /search?pattern=...`:** Pattern in the query string. Pros:
  conventional for read operations, and cacheable. Cons: regex metacharacters are
  URL-problematic. Rejected for that reason, despite the cleaner REST fit.

## Decision

Expose search as `POST /search` with a JSON body. POST was chosen over GET because
regex patterns contain characters problematic in URL query strings. Shipped at
`src/commands/serve.rs:347`.

## Consequences

### Positive
- Regex patterns transmit cleanly without query-string encoding issues.

### Negative
- Diverges from the REST read-via-GET convention for what is logically a read.

## Revisit if
- Search needs to be cacheable or linkable via URL.
