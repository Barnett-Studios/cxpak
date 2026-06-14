---
id: '0041'
title: GitHub Action as a separate composite-action repo posting PR comments
status: ACCEPTED
date: 2026-03-12
triggered_by: v0.8.0 integrations goal — automatic PR context
loop: planning
---

# ADR-0041: GitHub Action as a separate composite-action repo posting PR comments

## Context

`cxpak diff` produces exactly the context a reviewer needs, but in v0.8.0 it had to be run manually. The highest-leverage integration identified was automatic PR comments via a GitHub Action. This ADR records the planned design for that integration; see the Consequences section for shipment status.

## Options considered

- **Option A — Separate repo, composite action, binary from Releases:** Publish a composite action that downloads the cxpak Linux binary from GitHub Releases, runs `cxpak diff` against `origin/<base_ref>`, and posts a collapsible PR comment. Pro: Marketplace-publishable independently, no Docker build, fast (downloads a prebuilt binary), decoupled release cadence. Con: two repos to maintain, and it depends on the Releases artifact existing for the requested version. Someone could prefer this for its fast cold start and clean Marketplace listing. (Considered and chosen in the design.)
- **Option B — Docker container action:** A reasonable alternative would have been shipping the action as a container image bundling cxpak. Pro: a hermetic, version-pinned environment. Con: slower cold start (image pull) and heavier to publish. Someone could prefer it for reproducibility.
- **Option C — Action inside the main cxpak repo:** A reasonable alternative would have been keeping `action.yml` in the cxpak repo rather than a dedicated repo. Pro: a single repo to maintain. Con: couples action release to crate release and produces a messier Marketplace listing. Someone could prefer it to avoid a second repository.

## Decision

Ship the GitHub Action as a separate repo implemented as a composite action: download the prebuilt Linux binary from GitHub Releases, run `cxpak diff` against the PR base, and post the output as a collapsible `<details>` PR comment marked with an HTML comment (`<!-- cxpak -->`) for idempotent updates. The cxpak repo dogfoods it via its own workflow.

Note on repository slug: the design doc (and the verbatim source quote below) names `lyubomir-bozhinov/cxpak-action` and `lyubomir-bozhinov/cxpak` Releases. The actual project repo is `Barnett-Studios/cxpak` (git remote and `Cargo.toml` `repository`), so the `lyubomir-bozhinov` slug in the design is stale relative to reality; treat `Barnett-Studios` as the correct org.

## Consequences

### Positive
- Marketplace-distributable; one-click setup; fast (prebuilt binary).
- Idempotent comments via the `<!-- cxpak -->` marker avoid PR spam.
- Decoupled from crate release.

### Negative
- Two repositories to keep in sync.
- Relies on the Releases tarball for the requested version existing.

### Neutral
- Action versioned independently (tag `v1`); needs `pull-requests: write` permission.
- Shipment unverified: the GitHub Action (Feature 1 of the v0.8.0 plan) cannot be confirmed shipped from the cxpak repo. The promised dogfood workflow `.github/workflows/cxpak-context.yml` does not exist, and git history contains no commit referencing `cxpak-action` or the `<!-- cxpak -->` marker outside the design/plan docs themselves; the sibling `cxpak-action` repo cannot be confirmed from here. Feature 2 (daemon mode) of the same v0.8.0 plan did ship (`src/daemon/`, the `daemon` feature).

## Revisit if
- The action needs an OS/arch matrix beyond `x86_64-linux`.
- Composite-action download flakiness pushes toward a container action.
- Shipment is confirmed or the dogfood workflow is added, in which case the status note should be updated.
