---
id: '0170'
title: Publish official multi-arch container images from CI, built from release artifacts and signed
status: ACCEPTED
date: 2026-06-18
triggered_by: Docker support PR (#3) shipped Dockerfiles users build themselves; a downloaded-binary image had no integrity verification and there was no published image
loop: implementation
---

# ADR-0170: Publish official multi-arch container images from CI

## Context

The initial Docker contribution (#3) shipped two Dockerfiles: a multi-stage **source** build and a **standalone** image that fetched a release tarball at build time with `curl -fsSL … | tar -xz` — no checksum, no signature, and pinned by a mutable `ARG VERSION` default. It also published **no image**: every user had to build locally. cxpak already runs server surfaces (`cxpak serve` HTTP on :3000, `serve --mcp` over stdio), which are natural container entrypoints, so "Docker is a first-class deployment option" should mean a pull-and-run image, not a build recipe.

The `release.yml` workflow already builds, tests, and uploads per-target binaries on each `v*` tag. The same artifacts can feed an image-publish job, so the published image is byte-identical to the released binary with no rebuild.

A glibc constraint shapes the runtime base: the Linux release binaries are built on `ubuntu-latest` (24.04, glibc 2.39), so a `distroless`/`debian:bookworm` base (glibc 2.36) would fail to run them. `ubuntu:24.04` matches the builder and is the safe base today.

## Options considered

- **Option A — publish multi-arch images from CI, built from release artifacts (chosen):** a `docker` job (`needs: build`) downloads the Linux gnu artifacts, extracts them to `dist/linux/<arch>/cxpak`, and `buildx --platform linux/amd64,linux/arm64` builds `Dockerfile.dist`, which only `COPY`s the per-arch binary (selected by `$TARGETARCH`) — no QEMU, no recompile. Push to `ghcr.io/barnett-studios/cxpak`, sign keyless with cosign (GitHub OIDC), and attach SBOM + provenance. Pros: pull-and-run UX; integrity is intrinsic (artifacts from the same pipeline, not a re-download); reproducible digests; supersedes the unverified `curl | tar`. Cons: more CI surface (GHCR perms, OIDC, cosign).
- **Option B — keep shipping Dockerfiles only (status quo of #3):** users build locally. Pros: no registry/CI work. Cons: no published image; the standalone download path stays unverified; worse UX. Rejected.
- **Option C — build the image by compiling from source in CI:** Pros: one Dockerfile. Cons: a second full compile of candle + 43 grammars per release (slow), and it can drift from the released binary. Rejected in favor of reusing the tested artifacts.

## Decision

Option A. Add a `docker` job to `release.yml` that builds and pushes a signed, multi-arch (`amd64`/`arm64`) image to GHCR from the release artifacts via `Dockerfile.dist` (a non-root `ubuntu:24.04` runtime, base digest-pinned). Keep the source `Dockerfile` for development/forks (deps `build-essential` + `pkg-config` only — no OpenSSL toolchain after [ADR-0163](0163-windows-build-git2-no-default-features.md)). Remove `Dockerfile.standalone`; the published image replaces it. Both images run as a non-root user (uid 10001) with the embedding-model cache under `/home/cxpak/.cxpak`.

## Consequences

### Positive
- `docker run ghcr.io/barnett-studios/cxpak overview .` — no build, no Rust toolchain, no source checkout.
- Supply-chain integrity by construction: no unverified download, immutable digests to pin, cosign signature + SBOM + provenance to verify.
- Non-root images; smaller maintenance surface (one CI-built runtime image + one source Dockerfile).

### Negative
- New CI dependencies (GHCR `packages: write`, OIDC `id-token: write`, cosign) and a new failure point in the release.
- The runtime base is coupled to the release builder's glibc (`ubuntu:24.04`), so a minimal `distroless`/`scratch` image is deferred.

### Neutral
- `Dockerfile.dist` is also usable manually after extracting a release tarball into `dist/linux/<arch>/`.
- Cache writes into a host-mounted `/repo` degrade gracefully under the non-root user; persistence requires `--user` or a writable mount.

## Revisit if
- A minimal image is wanted: build a `musl` static target → `FROM scratch`, or move the Linux release build to an older glibc so `distroless` works.
- An air-gapped, batteries-included variant is needed: bake the embedding weights into a `:full` tag.
- GHCR is replaced or augmented by another registry (Docker Hub, ECR).
