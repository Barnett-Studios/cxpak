---
id: '0136'
title: Monorepo workspace support via path-prefix scoping and per-workspace cache namespaces
status: ACCEPTED
date: 2026-04-01
triggered_by: v1.3.0 adding monorepo support so a single repo can be analyzed per sub-package
loop: implementation
---

# ADR-0136: Monorepo workspace support via path-prefix scoping and per-workspace cache namespaces

## Context

Shipped in v1.3.0. Monorepos hold multiple packages in one git repo. Users wanted to scope cxpak analysis to one package without re-cloning, and cached indices for different packages must not collide.

## Options considered

- **Option A — path-prefix scoping with namespaced cache:** Thread `workspace: Option<String>` as a path prefix through the scanner, every CLI command, and every MCP tool handler. `Scanner::scan_workspace` filters scanned files by relative-path prefix (component-aware so `src/` does not match `src_utils/`); the cache directory is namespaced by sanitized prefix (`.cxpak/cache/root` for `None`, `.cxpak/cache/packages_api` for `Some("packages/api")`). Pros: minimal, uniform mechanism; reuses the existing scanner; cache isolation is automatic. Cons: path-prefix scoping is not true package-graph awareness — no per-package dependency boundaries. This is what shipped.
- **Option B — per-package index from explicit manifests:** A reasonable alternative would have been to parse `Cargo.toml`/`package.json`/etc. to define real package boundaries and build a separate index per package. It would give semantically accurate boundaries, but requires per-ecosystem manifest parsing and far more work. Reconstructed alternative; not formally evaluated.

## Decision

Add `workspace: Option<String>` threaded through `Scanner` (`scan_workspace` filters by relative-path prefix), all CLI commands (`--workspace` flag), and all MCP tool handlers. Cache directories are namespaced via `cache_namespace(repo_root, workspace)`: `None -> ".cxpak/cache/root"`, `Some("packages/api") -> ".cxpak/cache/packages_api"` (slashes replaced by underscores), so each workspace's index is isolated.

## Consequences

### Positive
- One repo can be analyzed per sub-package with no extra setup.
- Cache collisions between workspaces are impossible.
- Uniform prefix mechanism across CLI, MCP, and HTTP surfaces.

### Negative
- Prefix scoping does not model true inter-package dependencies.
- Slash-to-underscore namespacing could theoretically collide for unusual prefixes.

### Neutral
- `validate_workspace_path` in v1.6.0 reuses the same workspace concept for path-traversal defense on the HTTP API.

## Revisit if
- Users need true package-graph boundaries rather than path prefixes.
- Cache namespace collisions occur in practice.
