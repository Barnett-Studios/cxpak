# cxpak

![Rust](https://img.shields.io/badge/Rust-1.91+-orange.svg)
![CI](https://github.com/Barnett-Studios/cxpak/actions/workflows/ci.yml/badge.svg)
![Crates.io](https://img.shields.io/crates/v/cxpak)
![Downloads](https://img.shields.io/crates/d/cxpak)
![License](https://img.shields.io/badge/License-MIT-green.svg)

**Spends CPU cycles so you don't spend tokens.**

cxpak indexes your codebase using tree-sitter across 43 languages, builds a typed dependency graph, and produces token-budgeted context bundles that give LLMs a briefing packet instead of a flashlight in a dark room. It understands your code's architecture, conventions, risk profile, and data layer -- then packs exactly what the LLM needs, nothing more.

## What it looks like

<p align="center">
<img src="docs/images/dashboard.png" alt="Dashboard" width="100%">
</p>

<details>
<summary>Architecture Explorer -- directed dependency graph with risk coloring</summary>
<img src="docs/images/architecture.png" alt="Architecture Explorer" width="100%">
</details>

<details>
<summary>Architecture -- tooltip with PageRank and metadata on hover</summary>
<img src="docs/images/architecture-tooltip.png" alt="Architecture Tooltip" width="100%">
</details>

<details>
<summary>Risk Heatmap -- treemap sized by token count, colored by risk score</summary>
<img src="docs/images/risk.png" alt="Risk Heatmap" width="100%">
</details>

<details>
<summary>Flow Diagram -- call graph with directional arrows</summary>
<img src="docs/images/flow.png" alt="Flow Diagram" width="100%">
</details>

<details>
<summary>Diff View -- before/after with blast radius overlay</summary>
<img src="docs/images/diff.png" alt="Diff View" width="100%">
</details>

## Install

```bash
brew tap Barnett-Studios/tap && brew install cxpak   # macOS/Linux
cargo install cxpak                                   # any platform, incl. Windows
```

On Windows, `cargo install cxpak` works, or download the prebuilt
`cxpak-x86_64-pc-windows-msvc.zip` from the [latest release](https://github.com/Barnett-Studios/cxpak/releases/latest).

## Docker

Docker is a first-class deployment option — useful anywhere you want a reproducible, isolated install without managing a Rust toolchain: CI pipelines, sandboxed servers, Windows machines, or air-gapped environments.

### Official image (recommended)

Multi-arch (`amd64` / `arm64`) images are published to GitHub Container Registry on every release — no build, no Rust toolchain, no source checkout:

```bash
docker run --rm -v "$(pwd):/repo" ghcr.io/barnett-studios/cxpak overview .
```

Pin a tag or an immutable digest for reproducible deploys:

```bash
docker run --rm -v "$(pwd):/repo" ghcr.io/barnett-studios/cxpak:2.2.1 overview .
docker run --rm -v "$(pwd):/repo" ghcr.io/barnett-studios/cxpak@sha256:<digest> overview .
```

Images are signed with [cosign](https://github.com/sigstore/cosign) (keyless) and carry SBOM + build-provenance attestations. Verify before deploying:

```bash
cosign verify ghcr.io/barnett-studios/cxpak:2.2.1 \
  --certificate-identity-regexp '^https://github.com/Barnett-Studios/cxpak/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

### From source

```bash
docker build -t cxpak .
```

Builds the full default feature set from your local checkout. First build is slow (candle ML deps); subsequent builds reuse a cached dependency layer.

### Self-hosted / air-gapped

[`Dockerfile.standalone`](Dockerfile.standalone) fetches the pre-built release binary, verifies its SHA-256 checksum, and packages it into an `ubuntu:24.04` runtime — no source checkout or Rust toolchain required. All base images and the downloaded binary are digest-pinned for reproducible builds.

```bash
docker build -f Dockerfile.standalone -t cxpak .
```

To pin a specific release, pass its version and per-arch checksums (available on the [releases page](https://github.com/Barnett-Studios/cxpak/releases)):

```bash
docker build -f Dockerfile.standalone \
  --build-arg VERSION=2.3.0 \
  --build-arg SHA256_AMD64=c98d142aec62a70bb5ecccdf44120aaa55641a26b27d5a52821a093c79dd8cac \
  --build-arg SHA256_ARM64=2f7cc078446a65bdb8f2cbcc81b2e1932431b066d560fe99e90519c7afd3d580 \
  -t cxpak:2.3.0 .
```

### Usage

The container runs as a non-root user; the embedding model weights (~30 MB, downloaded on first use) live under `/home/cxpak/.cxpak` — mount a named volume there to persist them across runs.

**macOS / Linux:**
```bash
# One-shot command
docker run --rm -v "$(pwd):/repo" ghcr.io/barnett-studios/cxpak overview .

# HTTP server (--bind 0.0.0.0 required to reach the container from the host;
# --token is mandatory when binding to a non-loopback address)
docker run -d -p 3000:3000 \
  -v "$(pwd):/repo" \
  -v cxpak-models:/home/cxpak/.cxpak \
  ghcr.io/barnett-studios/cxpak serve --bind 0.0.0.0 --token mysecret .

# MCP — stdio only, one repo per instance (see note below)
docker run --rm -i -v "$(pwd):/repo" ghcr.io/barnett-studios/cxpak serve --mcp .
```

**Windows (PowerShell):**
```powershell
# One-shot command
docker run --rm -v ${PWD}:/repo ghcr.io/barnett-studios/cxpak overview .

# HTTP server
docker run -d -p 3000:3000 `
  -v ${PWD}:/repo `
  -v cxpak-models:/home/cxpak/.cxpak `
  ghcr.io/barnett-studios/cxpak serve --bind 0.0.0.0 --token mysecret .

# Verify (use curl.exe — PowerShell's curl alias does not work here)
curl.exe http://localhost:3000/health

# MCP — stdio only, one repo per instance (see note below)
docker run --rm -i -v ${PWD}:/repo ghcr.io/barnett-studios/cxpak serve --mcp .
```

Replace `mysecret` with any non-empty secret of your choice. `/health` is open (GET); every `/v1/*` endpoint is a POST and requires the bearer token:
```bash
curl http://localhost:3000/health                                        # no auth required
curl -X POST -H "Authorization: Bearer mysecret" http://localhost:3000/v1/conventions
```

> **HTTP vs MCP:** These are two separate transports — you cannot use the HTTP server as an MCP endpoint.
>
> **MCP scope:** Each MCP instance indexes exactly one repository — the path passed at startup (`.` in the examples above, which maps to the mounted `/repo`). To serve multiple repos simultaneously, run one container per repo and register each in your MCP client config. The HTTP server has the same single-repo scope.

## Quick start

```bash
# See your codebase the way an LLM should
cxpak overview .

# Trace a symbol through the dependency graph
cxpak trace "handle_request" .

# Generate an interactive dashboard
cxpak visual --visual-type dashboard .

# Get a guided reading order for onboarding
cxpak onboard .
```

## Use with AI tools

### Claude Code / Cursor (MCP)

Add to `.mcp.json` in your project root:

```json
{
  "mcpServers": {
    "cxpak": {
      "command": "cxpak",
      "args": ["serve", "--mcp", "."]
    }
  }
}
```

Your AI tool gets 26 codebase intelligence tools -- `cxpak_auto_context` is the main entry point. One call, optimal context:

| Category | Tools |
|----------|-------|
| **Context** | `auto_context`, `overview`, `trace`, `diff`, `search`, `context_for_task`, `pack_context`, `context_diff` |
| **Intelligence** | `health`, `risks`, `architecture`, `blast_radius`, `predict`, `drift`, `dead_code`, `call_graph` |
| **Security** | `security_surface`, `data_flow`, `cross_lang` |
| **Conventions** | `conventions`, `verify`, `briefing` |
| **Visual** | `visual`, `onboard` |
| **Surface** | `api_surface`, `stats` |

### Claude Code Plugin

```
/plugin install cxpak
```

Auto-triggers on architecture questions and change reviews. Slash commands: `/cxpak:overview`, `/cxpak:trace`, `/cxpak:diff`, `/cxpak:clean`.

### HTTP Server

```bash
cxpak serve .                          # port 3000
cxpak serve --token my-secret .        # with Bearer auth on /v1/ endpoints
cxpak watch .                          # file watcher with hot index
```

### LSP

```bash
cxpak lsp .                            # stdio, works with any LSP client
```

CodeLens, hover, diagnostics, workspace symbols, plus 14 custom `cxpak/*` methods. Supports `didOpen`/`didChange`/`didClose` for in-editor reactivity.

## Core capabilities

### Auto Context

`cxpak_auto_context` is the primary entry point. Give it a task and token budget; it returns exactly what the LLM needs.

The pipeline: query expansion with domain-specific synonyms, 7-signal relevance scoring (keyword, symbol, path, domain, import proximity, PageRank, embeddings), seed selection, noise filtering, test/schema/blast-radius enrichment, progressive degradation (Full > Trimmed > Documented > Signature > Stub), and per-file annotations explaining why each file was included.

Every response starts with a Repository DNA section -- a ~1000 token convention summary so the LLM knows how your team writes code before it sees any.

### Intelligence

| Feature | What it does |
|---------|-------------|
| **Health Score** | Composite metric across conventions, test coverage, churn stability, coupling, cycles, dead code |
| **Risk Ranking** | Files ranked by churn x blast radius x test gap -- the ones most likely to cause problems |
| **Architecture** | Per-module coupling, cohesion, circular dependencies, boundary violations, god files |
| **Blast Radius** | Change impact: direct dependents, transitive dependents, test files, schema dependents, each with risk scores |
| **Change Prediction** | Structural + historical (180-day co-change) + call-graph signals, confidence 0.3--0.9 |
| **Architecture Drift** | Compare against stored baselines; auto-saves snapshots for trend tracking |
| **Dead Code** | Symbols with zero callers, ranked by importance (PageRank x visibility) |
| **Call Graph** | Cross-file call edges with Exact/Approximate confidence levels |
| **Security Surface** | Unprotected endpoints, secrets, SQL injection, validation gaps, exposure scores across 12 frameworks |
| **Data Flow** | Trace values source-to-sink through the call graph; reports module/language/security boundary crossings |
| **Cross-Language** | HTTP, FFI, gRPC, GraphQL, shared schema, and exec bridges between languages |

### Visual Intelligence

Six interactive views, self-contained HTML with D3.js. No build step, no CDN.

```bash
cxpak visual --visual-type dashboard .
cxpak visual --visual-type architecture .
cxpak visual --visual-type risk .
cxpak visual --visual-type flow --symbol handle_request .
cxpak visual --visual-type timeline .
cxpak visual --visual-type diff --files "src/api.rs,src/db.rs" .
```

Export formats: HTML, Mermaid, SVG, PNG, C4 DSL, JSON.

Layout engine: Sugiyama method with SCC condensation, barycenter crossing minimization, Brandes-Kopf coordinate assignment, and 7+/-2 cognitive clustering.

### Conventions

Extracts a quantified convention profile from what your team actually does: naming, imports, error handling, dependencies, testing, visibility, function length, git health. Each pattern has counts, percentages, and strength labels (Convention >= 90%, Trend >= 70%, Mixed).

`cxpak_verify` checks code changes against observed conventions -- only flags violations in changed lines. `cxpak conventions export/diff` enables CI drift detection with SHA256 checksums.

### Onboarding

```bash
cxpak onboard .
```

Generates a dependency-ordered reading guide: files topologically sorted, grouped into phases by module, ordered by PageRank. Each file lists key symbols to focus on and an estimated reading time.

## Language support (43)

**Full extraction** (functions, classes, methods, imports, exports):
Rust, TypeScript, JavaScript, Python, Java, Go, C, C++, Ruby, C#, Swift, Kotlin, Bash, PHP, Dart, Scala, Lua, Elixir, Zig, Haskell, Groovy, Objective-C, R, Julia, OCaml, MATLAB, Clojure

**Structural extraction** (selectors, keys, blocks):
CSS, SCSS, Markdown, JSON, YAML, TOML, Dockerfile, HCL/Terraform, Protobuf, Svelte, Makefile, HTML, GraphQL, XML

**Database DSLs:** SQL, Prisma

## Data layer awareness

cxpak understands your data layer and uses it to build a richer dependency graph:

- **Schema detection** -- SQL DDL, Prisma, Django, SQLAlchemy, TypeORM, ActiveRecord
- **Migration sequences** -- Rails, Alembic, Flyway, Django, Knex, Prisma, Drizzle
- **Embedded SQL linking** -- inline SQL in application code creates edges to table definitions
- **10 typed edge types** -- Import, ForeignKey, ViewReference, EmbeddedSql, OrmModel, MigrationSequence, CrossLanguage, and more

## Embeddings

Semantic similarity as the 7th scoring signal. Local inference with all-MiniLM-L6-v2 (zero config, ~30 MB), or bring your own key for OpenAI, Voyage AI, or Cohere via `.cxpak.json`. Falls back gracefully to 6 deterministic signals on any failure.

## WASM Plugin SDK

Extend cxpak with custom analyzers and detectors:

- Plugin manifest with SHA-256 checksum verification before WASM compilation
- wasmtime sandbox: epoch interruption (CPU), 64 MB memory cap, capability enforcement
- File pattern scoping and content access control

## Workspace support

For monorepos: `--workspace packages/api` scopes scanning to a subdirectory while keeping the full repo as the git root.

## Caching

Parse results cached in `.cxpak/cache/` keyed on file mtime and size. Cache invalidates automatically when tree-sitter grammar versions change. Atomic writes with advisory locking for concurrent process safety. `cxpak clean .` to reset.

## Stable API

v2.0.0 establishes semver for the MCP API. Tool names, parameters, and response structures are stable across 2.x.

## Architecture decisions

Every architecturally significant decision is recorded as an ADR in [`docs/adrs/`](docs/adrs/) -- what was chosen, the options considered, and the conditions under which to revisit it. The records span parsing, the typed dependency graph, relevance scoring, token budgeting, the MCP/HTTP/LSP surfaces, and distribution, reconstructed across v0.1.0 -> v2.2.1. Start with [the index](docs/adrs/INDEX.md).

## License

MIT

---

Built by [Barnett Studios](https://barnett-studios.com/) -- building products, teams, and systems that last.
