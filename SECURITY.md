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
- **WASM plugin SDK (`plugins`, off by default)** — plugins run in a wasmtime
  sandbox with a SHA-256 checksum verified before compilation, plus memory and
  CPU (epoch-interruption) caps. Only load plugins you trust; the checksum pins
  what you approved, it does not vouch for the author.

## What is not a vulnerability

- Running an untrusted third-party WASM plugin and having it access repository
  content — that's the plugin's declared capability; vet plugins before loading.
- Pointing cxpak at a repository you don't control and disliking what it emits —
  cxpak reports what it finds; it doesn't run the analyzed code.

## Dependencies

The dependency tree is OpenSSL-free (rustls throughout). Dependabot security
updates are enabled so transitive advisories surface as PRs; where a transitive
advisory is capped by a parent crate's constraint, we bump the parent.
