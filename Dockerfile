# syntax=docker/dockerfile:1.7
# Build cxpak from source. For most users the published image is easier:
#   docker run --rm -v "$(pwd):/repo" ghcr.io/barnett-studios/cxpak overview .
# This Dockerfile is for building from a local checkout (development / forks).

# Base images are named, not hard-coded, because the names below are
# unqualified.  Docker resolves an unqualified name against docker.io; rootless
# podman ships no `unqualified-search-registries` by default, so the same line
# fails to resolve there.  Overriding the ARG lets such a host pass a fully
# qualified name without editing this file:
#   podman build --build-arg RUST_IMAGE=docker.io/library/rust:1.91-slim-bookworm ...
# The digests pin what the defaults resolve to; an override replaces both name
# and digest, so a caller who overrides takes on the pinning.
ARG RUST_IMAGE=rust:1.91-slim-bookworm@sha256:8514999d4786ef12efe89239e86b3d0a021b94b9d35108c8efe6c79ca7dc1a65
ARG RUNTIME_IMAGE=debian:bookworm-slim@sha256:96e378d7e6531ac9a15ad505478fcc2e69f371b10f5cdf87857c4b8188404716

# ── Builder ───────────────────────────────────────────────────────────────────
FROM ${RUST_IMAGE} AS builder

# build-essential: C toolchain for ring (rustls) and the tree-sitter grammar crates.
# pkg-config: probed by some *-sys build scripts.
# No cmake/perl: git2 is default-features=false and reqwest uses rustls, so the
# build pulls no OpenSSL (see docs/adrs/0163).
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        build-essential \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# assets/ must be present because src/ embeds it via include_str! at compile time.
# vendor/ carries the patched grammars named by [patch.crates-io] in Cargo.toml;
# without it `cargo build` cannot resolve the manifest.
COPY Cargo.toml Cargo.lock build.rs ./
COPY assets/ ./assets/
COPY vendor/ ./vendor/
COPY src/ ./src/

# Dependency caching is a cache mount rather than the usual stub-main.rs stage.
# That stage builds the dependencies once and the real stage builds again, and
# each commits /build/target as an image layer -- several GB written twice, which
# on a host near capacity fails AFTER a successful compile:
#   writing blob: ... (write /build/target/release/deps/libtracing_core-*.rlib:
#   no space left on device)
# A cache mount is not part of the image, so it commits nothing and the
# artifacts also survive between builds.  Because `COPY --from` cannot reach a
# cache mount, the binary is copied out to /out inside this same RUN.
RUN --mount=type=cache,target=/build/target,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo build --release \
    && mkdir -p /out && cp target/release/cxpak /out/cxpak

# ── Runtime ───────────────────────────────────────────────────────────────────
FROM ${RUNTIME_IMAGE}

LABEL org.opencontainers.image.source="https://github.com/Barnett-Studios/cxpak" \
      org.opencontainers.image.description="Token-budgeted codebase context for LLMs" \
      org.opencontainers.image.licenses="MIT"

# ca-certificates: HTTPS for the first-use embedding-model download.
# libgcc-s1: Rust panic-unwinding runtime (stripped from debian:*-slim).
# curl: HEALTHCHECK probe for `cxpak serve`.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        libgcc-s1 \
        curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home --user-group cxpak

# libgit2 refuses to open a repository whose owner differs from the running uid, and a
# bind-mounted host repo never belongs to 10001 — so `cxpak diff` failed with
# `Owner (-36)` on every documented container invocation (cxpak#60). `overview` does not
# open the repo through libgit2, which is why the README's headline command worked and
# this stayed invisible.
#
# libgit2 honours the system `safe.directory` and reads /etc/gitconfig itself, so this
# needs no git binary. Measured on the published 3.1.4 image: with this file the
# documented `docker run -v "$PWD:/repo" … diff` exits 0 and prints the diff; an
# otherwise identical rebuild without it exits 1 with `Owner (-36)`.
#
# The trust decision is the right one for this image: cxpak is a read-only analyser whose
# entire job is to open a repo it was explicitly handed on a mount.
RUN printf '[safe]\n\tdirectory = *\n' > /etc/gitconfig

COPY --from=builder /out/cxpak /usr/local/bin/cxpak

# Model weights (~30 MB) download on first use to $HOME/.cxpak/models. Mount a
# named volume at /home/cxpak/.cxpak to persist them across runs.
ENV HOME=/home/cxpak
USER 10001
WORKDIR /repo
VOLUME ["/home/cxpak/.cxpak"]
EXPOSE 3000

# Probes the HTTP server (`cxpak serve`). One-shot CLI runs exit before this matters.
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD curl -fsS http://localhost:3000/health || exit 1

ENTRYPOINT ["cxpak"]
