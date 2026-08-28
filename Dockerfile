# syntax=docker/dockerfile:1.7
# Build cxpak from source. For most users the published image is easier:
#   docker run --rm -v "$(pwd):/repo" ghcr.io/barnett-studios/cxpak overview .
# This Dockerfile is for building from a local checkout (development / forks).

# ── Builder ───────────────────────────────────────────────────────────────────
FROM rust:1.91-slim-bookworm@sha256:8514999d4786ef12efe89239e86b3d0a021b94b9d35108c8efe6c79ca7dc1a65 AS builder

# build-essential: C toolchain for ring (rustls) and the tree-sitter grammar crates.
# pkg-config: probed by some *-sys build scripts.
# No cmake/perl: git2 is default-features=false and reqwest uses rustls, so the
# build pulls no OpenSSL (see docs/adrs/0163).
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        build-essential \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache dependency compilation separately from source changes.
# assets/ must be present because src/ embeds it via include_str! at compile time.
COPY Cargo.toml Cargo.lock build.rs ./
COPY assets/ ./assets/
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release \
    && rm -rf src

COPY src/ ./src/
RUN cargo build --release

# ── Runtime ───────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim@sha256:96e378d7e6531ac9a15ad505478fcc2e69f371b10f5cdf87857c4b8188404716

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

COPY --from=builder /build/target/release/cxpak /usr/local/bin/cxpak

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
