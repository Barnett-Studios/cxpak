---
id: '0088'
title: Add a 7th embedding signal via local candle MiniLM by default with optional BYOK remote providers
status: ACCEPTED
date: 2026-03-22
triggered_by: Want semantic similarity scoring with zero configuration but optional higher quality
loop: planning
---

# ADR-0088: Add a 7th embedding signal via local candle MiniLM with optional BYOK remote providers

## Context

v1.0.0 adds semantic similarity as a 7th relevance signal (weight 0.15). The goal is a signal that works with zero configuration but can be upgraded to higher quality on demand. Local inference uses `all-MiniLM-L6-v2` (384-dim, ~30MB) via candle in SafeTensors format, auto-downloaded at server startup. Optional remote providers (OpenAI / Voyage AI / Cohere / OpenAI-compatible) are configured via `.cxpak.json`. Any failure gracefully falls back to the 6 deterministic signals. Both `embeddings` and `daemon` are part of the default feature set.

## Options considered

- **Option A — Local MiniLM default + BYOK remote, graceful fallback:** candle SafeTensors MiniLM auto-downloaded to `~/.cxpak/models`; `.cxpak.json` selects a remote provider; the signal weight is 0.15 regardless of source; on any failure fall back to the 6 deterministic signals. Pros: zero config, never hard-fails, optional quality upgrade; embedding signatures only keeps the signal semantic. Cons: ~30MB model download and candle/tokenizers in the default binary increase build/binary size. This was the chosen option.
- **Option B — Larger local model:** Use a bigger embedding model for higher quality. Pros: better embeddings. Cons: MiniLM is 30MB/384-dim/fast and judged good enough for code similarity; a larger model trades that off for download size and latency.
- **Option C — Remote-only (no local model):** Require an API key for embeddings. A reasonable alternative would have been to ship no local model and require BYOK, avoiding the model download and shrinking the binary; it was not formally evaluated. It would break the zero-config philosophy.
- **Option D — Embed full file content:** Embed whole files instead of signatures. Pros: more text per symbol. Cons: bodies add noise; signatures are the semantic identity (per ICSE 2026), so signature-only embedding is higher signal.

## Decision

Add `embedding_similarity` as signal #7 (weight 0.15; the field is always present, 0.0 when inactive). Default to local `all-MiniLM-L6-v2` via candle in SafeTensors format (not ONNX, which candle lacks a general-purpose runtime for), auto-downloaded to `~/.cxpak/models` at server startup, embedding public symbol signatures only into a flat-matrix `EmbeddingIndex` serialized to `.cxpak/embeddings.bin`. Support BYOK OpenAI / Voyage AI / Cohere via `.cxpak.json` with a defined resolution order, and fall back to the 6 deterministic signals on any failure. Add `embeddings` and `daemon` to the default features.

## Consequences

### Positive
- Zero-config semantic signal that never hard-fails.
- BYOK enables better code embeddings and self-hosted/proxy endpoints without changing the 0.15 weight.
- Signatures-only embedding stays high-signal; the flat-matrix layout is cache-friendly (~1ms search).

### Negative
- ~30MB model download and candle+tokenizers bloat the default binary (~2MB+); SafeTensors is required because candle lacks an ONNX runtime.
- The remote provider needs a `reqwest` blocking client because the build runs synchronously.

### Neutral
- Shipped code uses candle 0.9 (the spec said 0.8 / "use whatever is current") and added `bincode` to the embeddings feature.
- Embedding-related fields are `#[cfg(feature = "embeddings")]` gated; the weight field is not.

## Revisit if
- A better small code-embedding model becomes available.
- The binary size from default embeddings becomes a problem for users.
- candle gains a usable ONNX runtime.
