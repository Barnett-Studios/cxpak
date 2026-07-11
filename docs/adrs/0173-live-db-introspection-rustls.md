# ADR-0173: Live DB introspection + schema drift (rustls, feature-gated)

- **Status:** ACCEPTED
- **Date:** 2026-06-24
- **Deciders:** cxpak maintainers (3.0.0, Phase A — Task A1)
- **Supersedes / Amends:** extends ADR-0097 (drift is descriptive only); preserves ADR-0163 (OpenSSL-free dependency tree)

## Context

cxpak already builds a *code* `SchemaIndex` from migrations, ORM models, and
SQL DDL (`src/schema/`). Task A1 adds the ability to connect to a **live**
Postgres or MySQL database, reflect its actual schema, and compute **schema
drift** — the divergence between what the code declares and what the database
actually is (e.g. an unapplied migration leaves a column code-only; a hand-made
column is live-only; a type changed out from under the migrations).

Three forces constrain the design:

1. **OpenSSL-free, portable static binary (ADR-0163).** Every release target
   must build without `openssl-sys`/`native-tls`. Any DB driver we pull must use
   rustls end to end.
2. **Credential safety.** A DSN carries host, user, and often password. Driver
   errors routinely embed the connection string. None of that may ever reach a
   log line, a persisted file, or an error message.
3. **Default build must stay DB-free and OpenSSL-free.** The DB drivers and the
   tokio runtime they need must not enter the default dependency tree or the
   shipped binary.

## Options considered

### DB driver selection (must be rustls / OpenSSL-free for BOTH dialects)

- **Postgres: `tokio-postgres` + `tokio-postgres-rustls`.** Pure-Rust, rustls
  TLS, purpose-built for catalog queries. *Chosen.*
- **MySQL — Option A: `mysql_async` (`default-features = false`,
  `features = ["minimal", "rustls-tls"]`).** Pure-Rust, rustls, tokio-native,
  lightweight, pairs naturally with `tokio-postgres`. *Chosen.*
- **MySQL — Option B: `sqlx` (`mysql`, `runtime-tokio-rustls`).** Also verified
  OpenSSL-free, but pulls the entire sqlx framework + compile-time DB-check
  macro machinery we do not need for read-only reflection. *Rejected* (weight
  without benefit).

`cargo tree -e features --features data-introspect | grep -iE "openssl|native-tls"`
is **empty** for the chosen pair, and the default tree has no DB deps at all.

### Async runtime lifecycle

- **Dedicated scoped `current_thread` runtime per call, `block_on`, drop after
  return.** *Chosen.* The runtime is built on a synchronous frame, drives
  connect+reflect to completion, and drops *after* `block_on` returns — never
  inside an async context, never nested in an existing runtime, so no
  nested-runtime panic is possible. cxpak's public entry point stays synchronous.
- **Reuse a global/shared runtime.** *Rejected.* Introspection is a rare,
  one-shot operation; a process-wide runtime would couple it to the daemon's
  lifecycle and risk nested-runtime drops.

### DSN / credential safety

- **`IntrospectError` with fixed, credential-free variants; driver errors
  summarized, never stored.** *Chosen.* `Debug`/`Display` emit only static
  strings; the DSN is borrowed, used to connect, and never copied into the
  result or any error. A gated live test forces an auth failure and asserts the
  sentinel password never appears in the error.
- **Propagate driver errors verbatim.** *Rejected* — driver errors embed the DSN.

## Decision

Add a **`data-introspect`** feature (OFF by default) gating
`tokio-postgres`, `tokio-postgres-rustls`, `rustls` (ring provider), `mysql_async`
(minimal + rustls-tls), and `tokio`. New module `src/schema/introspect.rs`:

- Pure, always-compiled core: `Dialect`, `IntrospectError`, the reflected-row
  types, and `map_reflected_to_index()` (deterministic rows → `SchemaIndex`,
  sorted tables/columns/keys, `<live:{dialect}>` sentinel `file_path`).
- Feature-gated live path: `introspect_live(dsn)` builds the scoped runtime,
  connects **read-only** (`SET default_transaction_read_only = on` /
  `SET SESSION TRANSACTION READ ONLY`), runs catalog `SELECT`s over
  `information_schema`/`pg_catalog`, and maps the result.

`src/intelligence/drift.rs` gains `build_schema_drift_report(live, code)` and
`SchemaDriftReport`/`SchemaDriftKind` — pure, no DB — flagging code-only
tables/columns, live-only tables/columns, and case-insensitive type mismatches.
The existing architecture-drift path is untouched. Drift is **descriptive
only** (ADR-0097): cxpak never mutates either schema.

## Consequences

- **Positive:** live-vs-code schema drift is now detectable; default build stays
  byte-identical, DB-free, and OpenSSL-free; the pure mapping + drift logic is
  fully unit-tested with synthetic rows and no live database; live tests are
  `#[ignore]`d behind `CXPAK_PG_DSN`/`CXPAK_MYSQL_DSN` env DSNs (no secret
  literals in source).
- **Negative:** `--features data-introspect` adds ~90 crates and a tokio
  runtime; the live path requires a reachable DB to exercise end to end.
- **Neutral:** read-only enforcement is best-effort (session-level) — cxpak only
  issues SELECTs regardless.

## Revisit-if

- A pure-Rust, rustls MySQL driver lighter than `mysql_async` appears, or sqlx
  becomes the project standard for other reasons.
- We need to reflect more than tables/columns/PK/FK (e.g. views, indexes,
  constraints) into drift, requiring richer catalog queries.
- Drift becomes *prescriptive* (suggesting migrations) — that would supersede
  ADR-0097 and needs its own ADR.
