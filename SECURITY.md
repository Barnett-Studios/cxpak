# Security Policy

## Reporting a vulnerability

**Please do not report security vulnerabilities through public GitHub issues,
discussions, or pull requests.**

Report them privately through GitHub's
[security advisory form](https://github.com/Barnett-Studios/cxpak/security/advisories/new).
This lets us investigate and ship a fix before the issue is public.

Please include:

- a description of the vulnerability and its impact,
- the cxpak version (`cxpak --version`) and how it was built (which features),
- steps to reproduce, ideally a minimal proof of concept.

We aim to acknowledge a report within a few days and will keep you updated as we
work on a fix. We follow coordinated disclosure: once a fix is available we'll
publish an advisory and credit you, unless you'd prefer to remain anonymous.

## Supported versions

cxpak is pre-1.0 in spirit for security backports: fixes land on the **latest
published release line** (currently `3.1.x`) and are shipped in a new patch
release. We don't backport security fixes to older minor or major versions —
upgrade to the latest release to stay covered.

| Version | Supported |
|---|---|
| latest `3.1.x` | :white_check_mark: |
| older releases | :x: (upgrade to latest) |

## Security-relevant surfaces

Most cxpak usage is local and read-only — it indexes files you point it at. The
surfaces worth a security researcher's attention:

- **HTTP server (`cxpak serve`)** — Bearer-token auth on all `/v1/*` routes when
  `--token` is set; the token is compared in **constant time**
  (`subtle::ConstantTimeEq`) to avoid timing side-channels. Binding to a
  non-loopback address requires a token. `/health` is intentionally open as a
  liveness probe.
- **MCP / LSP servers** — stdio transports that index a single repository. They
  read source and expose analysis; they do not execute project code.
- **Live database introspection (`data-introspect`, off by default)** — connects
  to a running Postgres/MySQL over rustls (no OpenSSL), issues a **read-only**
  session, and **never logs or persists the DSN**. Off unless explicitly built
  with the feature.
- **WASM plugin loader (`plugins`, off by default and non-functional)** — see
  below. It is listed here to be explicit that it is *not* a security surface
  yet, because it cannot execute plugin code at all.

## The WASM plugin loader does not run plugins

`PluginLoader::load()` reads the module, enforces a 10 MiB size cap, verifies a
SHA-256 checksum in constant time, compiles it, instantiates it — and then
returns `Err("WASM plugin loaded (N bytes) but guest function binding not yet
implemented")`. There is no WIT bridge, so **no guest function has ever been
callable**. A test asserts exactly that error.

The feature is excluded from `default`, so a stock `cargo install cxpak` cannot
reach this code path at all.

**Do not read the wasmtime configuration as a security control.** Some of it is
real — memory growth is capped at 64 MiB by a `ResourceLimiter`, and
epoch-interruption is enabled with a 10 s deadline. But it has never guarded
anything, and it is incomplete in ways that matter before it could:

- `table_growing` returns `Ok(true)` unconditionally — table growth is unbounded.
- Fuel metering is explicitly disabled (`consume_fuel(false)`); the only CPU
  bound is the epoch deadline.
- The 1 MiB cap on a plugin's returned value is applied *after* `serde_json`
  deserialization, so the allocation it is meant to bound has already happened.

Earlier revisions of this document described the sandbox as an active control.
That was wrong: it described the intended design of a code path that cannot run.
Those three gaps must be closed, and the guest bridge built, before any of this
is a control worth reasoning about — tracked in
[#42](https://github.com/Barnett-Studios/cxpak/issues/42).

## What is not a vulnerability

- Pointing cxpak at a repository you don't control and disliking what it emits —
  cxpak reports what it finds; it doesn't run the analyzed code.
- A finding against the `plugins` loader that depends on executing guest code.
  It cannot execute guest code. Findings about the *loading* path — the size cap,
  the checksum comparison, module compilation, instantiation — are in scope, and
  a report there is welcome, because those steps do run.

## Dependencies

The dependency tree is OpenSSL-free (rustls throughout). Dependabot security
updates are enabled so transitive advisories surface as PRs; where a transitive
advisory is capped by a parent crate's constraint, we bump the parent.
