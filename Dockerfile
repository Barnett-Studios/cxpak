# syntax=docker/dockerfile:1.7
# ── Builder ───────────────────────────────────────────────────────────────────
FROM rust:1.91-slim-bookworm AS builder

# cmake + perl are required by vendored OpenSSL (git2 dep).
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        cmake \
        build-essential \
        perl \
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
# Touch main.rs so cargo considers the real source newer than the cached dep artifacts.
RUN touch src/main.rs && cargo build --release

# ── Runtime ───────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

# ca-certificates: needed for HTTPS when embeddings downloads model weights on first use.
# libgcc-s1:       Rust panic unwinding runtime.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        libgcc-s1 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/cxpak /usr/local/bin/cxpak

# all-MiniLM-L6-v2 weights are downloaded on first use to ~/.cxpak/models/.
# Mount a host path here to persist them across container runs:
#   docker run -v cxpak-models:/root/.cxpak ...
VOLUME ["/root/.cxpak"]

WORKDIR /repo
EXPOSE 3000

ENTRYPOINT ["cxpak"]
