# v1.0.0 Implementation Plan: One Call, Perfect Context

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the capstone release — `cxpak_auto_context` (one-call optimal context), noise filtering (three layers), local + BYOK embeddings (7th scoring signal), and `cxpak_context_diff` (session deltas). Stable MCP API under semver.

**Architecture:** Two new modules: `src/auto_context/` (orchestration, noise filtering, briefing assembly) and `src/embeddings/` (model management, vector index, provider abstraction). `auto_context` composes ALL existing cxpak infrastructure (expansion, scoring, PageRank, seeds, schema, blast radius, API surface, test mapping, degradation, annotations) into a single pipeline. Embeddings add a 7th signal to relevance scoring. `context_diff` uses in-memory snapshots. `daemon` feature added to `default`.

**Tech Stack:** Rust, candle (local inference), tokenizers, reqwest (API providers), serde_json (config parsing)

**Spec:** `docs/superpowers/specs/2026-03-22-v100-design.md`

---

## File Structure

### New Files
- `src/auto_context/mod.rs` — `auto_context()` orchestration pipeline, `AutoContextOpts`, `AutoContextResult`
- `src/auto_context/noise.rs` — blocklist, similarity dedup, relevance floor, `FilteredFile`
- `src/auto_context/briefing.rs` — fill-then-overflow budget allocation, section assembly
- `src/auto_context/diff.rs` — `ContextSnapshot`, `ContextDelta`, snapshot store/compare
- `src/embeddings/mod.rs` — public API, `EmbeddingIndex`, `EmbeddingProvider` trait
- `src/embeddings/config.rs` — `.cxpak.json` parsing, `EmbeddingConfig`, provider defaults
- `src/embeddings/local.rs` — candle model loading, local MiniLM inference (behind `embeddings` feature)
- `src/embeddings/remote.rs` — HTTP provider for OpenAI/VoyageAI/Cohere APIs (behind `embeddings` feature)
- `src/embeddings/index.rs` — vector index build, incremental update, cosine similarity search

### Modified Files
- `src/main.rs` — add `pub mod auto_context;` and `#[cfg(feature = "embeddings")] pub mod embeddings;`
- `Cargo.toml` — add candle/tokenizers/reqwest deps, add `embeddings` to `default`, add `daemon` to `default`
- `src/relevance/mod.rs` — add `embedding_similarity` weight to `SignalWeights` (always present, 0.0 when inactive), add 6-vs-7 signal weight selection
- `src/relevance/signals.rs` — add `#[cfg(feature = "embeddings")] pub fn embedding_similarity_signal()`
- `src/index/mod.rs` — add `#[cfg(feature = "embeddings")] pub embedding_index: Option<EmbeddingIndex>`
- `src/commands/serve.rs` — add `auto_context` + `context_diff` MCP tools (#10, #11), add `ContextSnapshot` state, reorder tools/list to put auto_context first, add HTTP endpoints

---

## Stream 1: Noise Filtering

### Task 1: Scaffold `auto_context` module

**Files:**
- Create: `src/auto_context/mod.rs`, `noise.rs`, `briefing.rs`, `diff.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Create module files with minimal content**

`src/auto_context/mod.rs`:
```rust
pub mod briefing;
pub mod diff;
pub mod noise;
```

Empty scaffolds for submodules.

- [ ] **Step 2: Add BOTH modules to `src/main.rs` in one edit**

Add both lines to avoid merge conflicts if Streams 1 and 2 run in parallel:
```rust
pub mod auto_context;
#[cfg(feature = "embeddings")]
pub mod embeddings;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check`

- [ ] **Step 4: Commit**

```bash
git add src/auto_context/ src/main.rs
git commit -m "feat: scaffold auto_context module for v1.0.0"
```

### Task 2: Implement blocklist noise filter

**Files:**
- Modify: `src/auto_context/noise.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blocklist_vendor() {
        assert!(is_blocklisted("vendor/lib/utils.rs"));
        assert!(is_blocklisted("node_modules/express/index.js"));
        assert!(is_blocklisted("third_party/grpc/lib.go"));
    }

    #[test]
    fn test_blocklist_build_output() {
        assert!(is_blocklisted("dist/bundle.js"));
        assert!(is_blocklisted("build/output.css"));
        assert!(is_blocklisted("target/debug/cxpak"));
        assert!(is_blocklisted("__pycache__/mod.pyc"));
    }

    #[test]
    fn test_blocklist_minified() {
        assert!(is_blocklisted("app.min.js"));
        assert!(is_blocklisted("styles.min.css"));
    }

    #[test]
    fn test_blocklist_generated() {
        assert!(is_blocklisted("types.generated.ts"));
        assert!(is_blocklisted("schema_pb.go"));
        assert!(is_blocklisted("proto_pb2.py"));
    }

    #[test]
    fn test_blocklist_lock_files() {
        assert!(is_blocklisted("package-lock.json"));
        assert!(is_blocklisted("yarn.lock"));
        assert!(is_blocklisted("Cargo.lock"));
        assert!(is_blocklisted("poetry.lock"));
    }

    #[test]
    fn test_blocklist_source_maps() {
        assert!(is_blocklisted("bundle.js.map"));
    }

    #[test]
    fn test_not_blocklisted() {
        assert!(!is_blocklisted("src/api/handler.rs"));
        assert!(!is_blocklisted("tests/auth_test.rs"));
        assert!(!is_blocklisted("src/deadlock.rs")); // must NOT match lock files
        assert!(!is_blocklisted("src/file_lock.py"));
    }

    #[test]
    fn test_generated_marker_detection() {
        assert!(has_generated_marker("// Code generated by protoc. DO NOT EDIT.\npackage pb"));
        assert!(has_generated_marker("# AUTO-GENERATED FILE\nimport foo"));
        assert!(has_generated_marker("/* DO NOT EDIT */\nconst x = 1;"));
        assert!(has_generated_marker("// @generated\nfn main() {}"));
        assert!(!has_generated_marker("fn main() { // this is not generated }"));
    }
}
```

- [ ] **Step 2: Implement blocklist + generated marker detection**

```rust
const NOISE_PATH_PATTERNS: &[&str] = &[
    "vendor/", "node_modules/", "third_party/", "external/",
    "dist/", "build/", "target/", ".next/", "__pycache__/", "out/",
    ".min.js", ".min.css",
    ".generated.", "_generated.", ".gen.",
    "_pb.go", "_pb2.py", ".pb.cc", ".pb.h",
    ".map",
];

const NOISE_EXACT_FILENAMES: &[&str] = &[
    "package-lock.json", "yarn.lock", "Cargo.lock", "pnpm-lock.yaml",
    "poetry.lock", "Gemfile.lock", "composer.lock",
];

const GENERATED_MARKERS: &[&str] = &[
    "// Code generated", "# AUTO-GENERATED", "/* DO NOT EDIT */",
    "// DO NOT EDIT", "@generated", "# This file is auto-generated",
];

pub fn is_blocklisted(path: &str) -> bool {
    let filename = path.rsplit('/').next().unwrap_or(path);
    if NOISE_EXACT_FILENAMES.contains(&filename) { return true; }
    NOISE_PATH_PATTERNS.iter().any(|p| path.contains(p))
}

pub fn has_generated_marker(content: &str) -> bool {
    let header: String = content.lines().take(5).collect::<Vec<_>>().join("\n");
    GENERATED_MARKERS.iter().any(|m| header.contains(m))
}
```

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/auto_context/noise.rs
git commit -m "feat: blocklist noise filter with generated marker detection"
```

### Task 3: Implement similarity dedup

**Files:**
- Modify: `src/auto_context/noise.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_jaccard_identical_symbols() {
    // Two files with same symbols → similarity 1.0
}

#[test]
fn test_jaccard_90_percent_overlap() {
    // 9 of 10 symbols shared → excluded (>0.80)
}

#[test]
fn test_jaccard_50_percent_overlap() {
    // 5 of 10 symbols shared → kept (<0.80)
}

#[test]
fn test_jaccard_no_overlap() {
    // No shared symbols → similarity 0.0
}

#[test]
fn test_dedup_keeps_higher_pagerank() {
    // Two similar files, A has PageRank 0.8, B has 0.3 → B filtered, A kept
}
```

- [ ] **Step 2: Implement `jaccard_symbol_similarity()` and `dedup_similar_files()`**

Per spec: Jaccard on symbol name sets. O(N²) on candidate set (typically 15-30 files).

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/auto_context/noise.rs
git commit -m "feat: similarity dedup noise filter (Jaccard >0.80 threshold)"
```

### Task 4: Implement relevance floor + filter orchestrator

**Files:**
- Modify: `src/auto_context/noise.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_relevance_floor_excludes_low_score() {
    // File with score 0.05 → excluded
}

#[test]
fn test_relevance_floor_boundary() {
    // File with score 0.15 → included (at boundary)
}

#[test]
fn test_relevance_floor_high_score() {
    // File with score 0.50 → included
}

#[test]
fn test_relevance_floor_dependency_below() {
    // Dependency with score 0.08 → excluded even though it's a dependency
}

#[test]
fn test_filter_orchestrator() {
    // Combine all 3 layers: blocklist → dedup → floor
    // Verify filtered_out contains reasons for each exclusion
}

#[test]
fn test_filter_preserves_order() {
    // After filtering, remaining files still sorted by score descending
}
```

- [ ] **Step 2: Implement `apply_relevance_floor()` and `filter_noise()` orchestrator**

```rust
pub const DEFAULT_RELEVANCE_FLOOR: f64 = 0.15;

pub struct FilteredFile {
    pub path: String,
    pub reason: String,
}

pub struct NoiseFilterResult {
    pub kept: Vec<ScoredFileWithContent>,
    pub filtered_out: Vec<FilteredFile>,
}

pub fn filter_noise(
    candidates: Vec<ScoredFileWithContent>,
    index: &CodebaseIndex,
    pagerank: &HashMap<String, f64>,
) -> NoiseFilterResult {
    // 1. Blocklist + generated markers
    // 2. Similarity dedup
    // 3. Relevance floor
    // Collect filtered_out with reasons at each step
}
```

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/auto_context/noise.rs
git commit -m "feat: relevance floor + noise filter orchestrator with filtered_out reasons"
```

---

## Stream 2: Embeddings

### Task 5: Scaffold embeddings module + config parsing

**Files:**
- Create: `src/embeddings/mod.rs`, `config.rs`, `local.rs`, `remote.rs`, `index.rs`
- Modify: `src/main.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Add dependencies to Cargo.toml**

```toml
# Verify latest candle version on crates.io at implementation time.
# CPU-only — do NOT enable CUDA/Metal feature flags.
candle-core = { version = "0.8", optional = true }
candle-nn = { version = "0.8", optional = true }
candle-transformers = { version = "0.8", optional = true }
tokenizers = { version = "0.21", optional = true }
reqwest = { version = "0.12", features = ["json", "blocking"], optional = true }
# "blocking" feature is required because embedding API calls happen inside
# CodebaseIndex::build() which runs in a synchronous context (no Tokio runtime).
# RemoteEmbeddingProvider uses reqwest::blocking::Client, not the async client.

[features]
default = [
    # ... all existing lang features ...
    "daemon",
    "embeddings",
]
embeddings = [
    "dep:candle-core", "dep:candle-nn", "dep:candle-transformers",
    "dep:tokenizers", "dep:reqwest",
]
```

- [ ] **Step 2: Create module scaffolds**

- [ ] **Step 3: `src/main.rs` already updated in Task 1 — skip**

- [ ] **Step 4: Verify compilation**

Run: `cargo check`

- [ ] **Step 5: Commit**

```bash
git add src/embeddings/ src/main.rs Cargo.toml Cargo.lock
git commit -m "feat: scaffold embeddings module with candle/tokenizers/reqwest deps"
```

### Task 6: Implement `.cxpak.json` config parsing

**Files:**
- Modify: `src/embeddings/config.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_parse_minimal_config() {
    let json = r#"{"embeddings":{"provider":"openai"}}"#;
    let config = EmbeddingConfig::from_json(json).unwrap();
    assert_eq!(config.provider, "openai");
    assert_eq!(config.model, "text-embedding-3-small"); // default
    assert_eq!(config.api_key_env, "OPENAI_API_KEY"); // default
}

#[test]
fn test_parse_full_config() {
    let json = r#"{"embeddings":{"provider":"voyageai","model":"voyage-code-3","api_key_env":"MY_KEY","base_url":"https://custom.api.com/v1","dimensions":1024,"batch_size":64}}"#;
    let config = EmbeddingConfig::from_json(json).unwrap();
    assert_eq!(config.provider, "voyageai");
    assert_eq!(config.model, "voyage-code-3");
    assert_eq!(config.api_key_env, "MY_KEY");
    assert_eq!(config.base_url, "https://custom.api.com/v1");
    assert_eq!(config.dimensions, 1024);
    assert_eq!(config.batch_size, 64);
}

#[test]
fn test_parse_local_default() {
    let config = EmbeddingConfig::default();
    assert_eq!(config.provider, "local");
    assert_eq!(config.model, "all-MiniLM-L6-v2");
    assert_eq!(config.dimensions, 384);
}

#[test]
fn test_parse_no_embeddings_section() {
    let json = r#"{"other":"stuff"}"#;
    let config = EmbeddingConfig::from_json(json);
    assert!(config.is_none() || config.unwrap().provider == "local");
}

#[test]
fn test_parse_from_file_not_found() {
    let config = EmbeddingConfig::from_repo_root("/nonexistent/path");
    assert_eq!(config.provider, "local"); // fallback
}

#[test]
fn test_provider_defaults_voyageai() {
    let json = r#"{"embeddings":{"provider":"voyageai"}}"#;
    let config = EmbeddingConfig::from_json(json).unwrap();
    assert_eq!(config.model, "voyage-code-3");
    assert_eq!(config.api_key_env, "VOYAGE_API_KEY");
    assert_eq!(config.dimensions, 1024);
}

#[test]
fn test_provider_defaults_cohere() {
    let json = r#"{"embeddings":{"provider":"cohere"}}"#;
    let config = EmbeddingConfig::from_json(json).unwrap();
    assert_eq!(config.model, "embed-english-v3.0");
    assert_eq!(config.api_key_env, "COHERE_API_KEY");
}
```

- [ ] **Step 2: Implement `EmbeddingConfig` with provider defaults**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    pub provider: String,
    pub model: String,
    pub api_key_env: String,
    pub base_url: String,
    pub dimensions: usize,
    pub batch_size: usize,
}

impl EmbeddingConfig {
    pub fn from_repo_root(path: &Path) -> Self { ... }
    pub fn from_json(json: &str) -> Option<Self> { ... }
    fn apply_provider_defaults(&mut self) { ... }
}
```

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/embeddings/config.rs
git commit -m "feat: .cxpak.json config parsing with provider defaults"
```

### Task 7: Implement embedding index + cosine similarity

**Files:**
- Modify: `src/embeddings/index.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_index_add_and_search() {
    let mut index = EmbeddingIndex::new(384);
    index.add("auth.rs", &vec![0.1; 384]);
    index.add("api.rs", &vec![0.9; 384]);
    let query = vec![0.85; 384];
    let results = index.search(&query, 2);
    assert_eq!(results[0].0, "api.rs"); // closest to query
}

#[test]
fn test_cosine_similarity() {
    let a = vec![1.0, 0.0, 0.0];
    let b = vec![1.0, 0.0, 0.0];
    assert!((cosine_similarity(&a, &b) - 1.0).abs() < 0.001);

    let c = vec![0.0, 1.0, 0.0];
    assert!(cosine_similarity(&a, &c).abs() < 0.001); // orthogonal
}

#[test]
fn test_index_incremental_update() {
    let mut index = EmbeddingIndex::new(384);
    index.add("auth.rs", &vec![0.1; 384]);
    index.add("auth.rs", &vec![0.9; 384]); // update
    assert_eq!(index.len(), 1); // not duplicated
}

#[test]
fn test_index_save_load() {
    let mut index = EmbeddingIndex::new(384);
    index.add("auth.rs", &vec![0.5; 384]);
    let path = tempdir.path().join("embeddings.bin");
    index.save(&path).unwrap();
    let loaded = EmbeddingIndex::load(&path).unwrap();
    assert_eq!(loaded.len(), 1);
}

#[test]
fn test_index_empty_search() {
    let index = EmbeddingIndex::new(384);
    let results = index.search(&vec![0.5; 384], 10);
    assert!(results.is_empty());
}
```

- [ ] **Step 2: Implement `EmbeddingIndex`**

**Use flat matrix layout for cache locality, NOT HashMap<String, Vec<f32>>:**

```rust
pub struct EmbeddingIndex {
    paths: Vec<String>,                    // path[i] corresponds to row i
    path_index: HashMap<String, usize>,    // path → row index (for incremental updates)
    vectors: Vec<f32>,                     // flat: vectors[i*dims..(i+1)*dims] = row i
    dims: usize,
}
```

This is 5-10x faster than per-file heap-allocated Vec<f32> due to cache locality. For 10k files × 384 dims = ~15MB contiguous memory, brute-force cosine similarity takes ~1ms. Serialized to `.cxpak/embeddings.bin` via bincode. **Add `bincode` to the `embeddings` feature deps:**

```toml
bincode = { version = "1", optional = true }
# Add "dep:bincode" to the embeddings feature list
```

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/embeddings/index.rs
git commit -m "feat: embedding vector index with cosine similarity search"
```

### Task 8: Implement local model provider (candle)

**Files:**
- Modify: `src/embeddings/local.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_local_model_download_and_cache() {
    // First call: downloads to ~/.cxpak/models/
    // Second call: loads from cache
}

#[test]
fn test_local_embed_text() {
    let model = LocalEmbeddingProvider::load_or_download().unwrap();
    let embedding = model.embed("pub fn authenticate(token: &str) -> Result<User, Error>");
    assert_eq!(embedding.len(), 384);
    // Verify non-zero
    assert!(embedding.iter().any(|&v| v != 0.0));
}

#[test]
fn test_local_embed_batch() {
    let model = LocalEmbeddingProvider::load_or_download().unwrap();
    let texts = vec!["fn foo()", "fn bar()", "struct User"];
    let embeddings = model.embed_batch(&texts);
    assert_eq!(embeddings.len(), 3);
}
```

- [ ] **Step 2: Implement `LocalEmbeddingProvider`**

**IMPORTANT: Use SafeTensors format, NOT ONNX.** candle does not have a general-purpose ONNX runtime. Download three files from HuggingFace `sentence-transformers/all-MiniLM-L6-v2`:
- `model.safetensors` (weights)
- `config.json` (model architecture config)
- `tokenizer.json` (tokenizer config)

Cache to `~/.cxpak/models/all-MiniLM-L6-v2/`.

Load via candle's BertModel:
```rust
let tokenizer = tokenizers::Tokenizer::from_file(cache_dir.join("tokenizer.json"))?;
let device = candle_core::Device::Cpu;
let vb = candle_nn::VarBuilder::from_safetensors(
    &[cache_dir.join("model.safetensors")],
    candle_core::DType::F32,
    &device,
)?;
let config: candle_transformers::models::bert::Config =
    serde_json::from_reader(std::fs::File::open(cache_dir.join("config.json"))?)?;
let model = candle_transformers::models::bert::BertModel::load(vb, &config)?;
```

Implement `embed()` (tokenize → forward pass → mean pooling → normalize) and `embed_batch()`.

- [ ] **Step 3: Run tests, verify pass** (requires network for first run)

- [ ] **Step 4: Commit**

```bash
git add src/embeddings/local.rs
git commit -m "feat: local embedding provider with candle MiniLM inference"
```

### Task 9: Implement remote API provider

**Files:**
- Modify: `src/embeddings/remote.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_remote_provider_openai_format() {
    // Mock HTTP response, verify request format matches OpenAI API
}

#[test]
fn test_remote_provider_missing_api_key() {
    let config = EmbeddingConfig { provider: "openai".into(), api_key_env: "NONEXISTENT_KEY".into(), ..Default::default() };
    let provider = RemoteEmbeddingProvider::new(&config);
    assert!(provider.is_err()); // graceful error
}

#[test]
fn test_remote_provider_custom_base_url() {
    // Verify base_url from config is used, not hardcoded
}
```

- [ ] **Step 2: Implement `RemoteEmbeddingProvider`**

HTTP client via reqwest. Supports OpenAI-compatible API format (covers OpenAI, VoyageAI, Ollama, any compatible proxy). Cohere has slightly different format — handle with a match on provider name.

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/embeddings/remote.rs
git commit -m "feat: remote embedding provider for OpenAI/VoyageAI/Cohere APIs"
```

### Task 10: Implement provider orchestrator + integrate with CodebaseIndex

**Files:**
- Modify: `src/embeddings/mod.rs`
- Modify: `src/index/mod.rs`

- [ ] **Step 1: Implement provider resolution**

```rust
pub fn create_provider(config: &EmbeddingConfig) -> Result<Box<dyn EmbeddingProvider>, String> {
    match config.provider.as_str() {
        "local" => Ok(Box::new(LocalEmbeddingProvider::load_or_download()?)),
        "openai" | "voyageai" | "cohere" => Ok(Box::new(RemoteEmbeddingProvider::new(config)?)),
        other => Err(format!("Unknown embedding provider: {other}")),
    }
}
```

Resolution order:
1. `.cxpak.json` exists → parse config → create provider
2. No config → create local provider
3. Any failure → return None (graceful fallback)

- [ ] **Step 2: Add `embedding_index` to `CodebaseIndex`**

```rust
#[cfg(feature = "embeddings")]
pub embedding_index: Option<crate::embeddings::EmbeddingIndex>,
```

Build during `CodebaseIndex::build()`: load config from repo root, create provider, embed all public symbol signatures, store index.

**Model download happens at index build time (server startup), NOT mid-request.** This means:
- `cxpak serve --mcp` → `build_index()` → embedding init (downloads model if needed, ~10s first time) → server ready
- User sees download progress in stderr at startup, not a silent mid-request pause
- If download fails at startup, server starts without embeddings (graceful fallback logged)
- In MCP handler, embeddings are already loaded — no blocking download during `auto_context`

- [ ] **Step 3: Write integration tests**

```rust
#[test]
fn test_index_builds_embeddings_when_available() { ... }
#[test]
fn test_index_no_embeddings_when_feature_off() { ... }
#[test]
fn test_index_graceful_fallback_on_failure() { ... }
```

- [ ] **Step 4: Run tests, verify pass**

- [ ] **Step 5: Commit**

```bash
git add src/embeddings/mod.rs src/index/mod.rs
git commit -m "feat: embedding provider orchestrator + CodebaseIndex integration"
```

### Task 11: Add embedding as signal #7 in relevance scoring

**Files:**
- Modify: `src/relevance/mod.rs`
- Modify: `src/relevance/signals.rs`

- [ ] **Step 1: Add `embedding_similarity` to `SignalWeights`**

Field always present (not cfg-gated). Value 0.0 when inactive.

Two `Default`-like constructors:
```rust
impl SignalWeights {
    pub fn with_embeddings() -> Self { /* 7 weights summing to 1.0 */ }
    pub fn without_embeddings() -> Self { /* 6 weights summing to 1.0 (v0.13.0 values) */ }
}
```

`MultiSignalScorer::new_for_index(index)` picks weights based on whether embeddings are available. **To handle cfg-gating, add a helper method on `CodebaseIndex`:**

```rust
impl CodebaseIndex {
    pub fn has_embedding_index(&self) -> bool {
        #[cfg(feature = "embeddings")]
        { self.embedding_index.is_some() }
        #[cfg(not(feature = "embeddings"))]
        { false }
    }
}
```

`new_for_index` calls `index.has_embedding_index()` — compiles correctly with or without the `embeddings` feature.

- [ ] **Step 2: Implement `embedding_similarity_signal()` (cfg-gated)**

- [ ] **Step 3: Update `score()` to include 7th signal when available**

- [ ] **Step 4: Write tests**

```rust
#[test]
fn test_weights_with_embeddings_sum_to_one() { ... }
#[test]
fn test_weights_without_embeddings_sum_to_one() { ... }
#[test]
fn test_scorer_uses_7_signals_when_available() { ... }
#[test]
fn test_scorer_uses_6_signals_when_unavailable() { ... }
```

- [ ] **Step 5: Run all relevance tests**

Run: `cargo test relevance --verbose`

- [ ] **Step 6: Commit**

```bash
git add src/relevance/mod.rs src/relevance/signals.rs
git commit -m "feat: embedding_similarity as signal #7 in relevance scoring"
```

---

## Stream 3: Auto Context Orchestrator

### Task 12: Implement fill-then-overflow budget allocation

**Files:**
- Modify: `src/auto_context/briefing.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_target_files_packed_first() { ... }
#[test]
fn test_tests_packed_second() { ... }
#[test]
fn test_schema_packed_third() { ... }
#[test]
fn test_api_surface_packed_fourth() { ... }
#[test]
fn test_blast_radius_packed_last() { ... }
#[test]
fn test_budget_exhausted_mid_section_degrades() { ... }
#[test]
fn test_generous_budget_everything_full_detail() { ... }
#[test]
fn test_tiny_budget_aggressive_degradation() { ... }
```

- [ ] **Step 2: Implement `allocate_and_pack()`**

Takes prioritized sections (target files, tests, schema, API surface, blast radius), a token budget, and packs them in order. Uses v0.11.0's `render_symbol_at_level()` for degradation. Each section gets as much budget as remains after higher-priority sections.

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/auto_context/briefing.rs
git commit -m "feat: fill-then-overflow budget allocation for auto_context sections"
```

### Task 13: Implement `auto_context()` orchestration pipeline

**Files:**
- Modify: `src/auto_context/mod.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_auto_context_happy_path() {
    // Build index, call auto_context("fix auth bug")
    // Verify: sections present, budget respected, annotations on all files
}

#[test]
fn test_auto_context_empty_repo() { ... }
#[test]
fn test_auto_context_single_file() { ... }
#[test]
fn test_auto_context_focus_param() { ... }
#[test]
fn test_auto_context_no_tests_flag() { ... }
#[test]
fn test_auto_context_no_blast_radius_flag() { ... }
#[test]
fn test_auto_context_tiny_budget() { ... }
#[test]
fn test_auto_context_huge_budget() { ... }
#[test]
fn test_auto_context_noise_filtered() {
    // Include vendor/ file in index, verify it appears in filtered_out
}
#[test]
fn test_auto_context_budget_summary_accurate() { ... }
#[test]
fn test_auto_context_annotations_present() { ... }
#[test]
fn test_auto_context_no_matches() { ... }
```

- [ ] **Step 2: Implement `auto_context()` pipeline**

Compose all existing infrastructure. **NOTE: The spec's function sketch uses illustrative names. The actual API calls are:**

```rust
pub fn auto_context(
    task: &str,
    index: &CodebaseIndex,
    opts: &AutoContextOpts,
) -> AutoContextResult {
    // 1. Query expansion
    let expanded = crate::context_quality::expansion::expand_query(task, &index.domains);

    // 2. Relevance scoring — use with_expansion(), NOT a nonexistent score_all_expanded()
    let scorer = crate::relevance::MultiSignalScorer::new_for_index(index)
        .with_expansion(expanded);
    let all_scored = scorer.score_all(task, index);

    // 3. Seed selection + fan-out
    let seeds = crate::relevance::seed::select_seeds_with_graph(
        &all_scored, index,
        crate::relevance::seed::SEED_THRESHOLD,
        50,
        Some(&index.graph),
    );

    // 4. Noise filtering (NEW — from auto_context::noise)
    let filtered = crate::auto_context::noise::filter_noise(seeds, index, &index.pagerank);

    // 5. Test file mapping — query index.test_map directly
    let mut with_tests = filtered.kept.clone();
    if opts.include_tests {
        for file in &filtered.kept {
            if let Some(test_files) = index.test_map.get(&file.path) {
                for tf in test_files {
                    // Add test file to candidate list if not already present
                    // (look up in index.files, add with role "test_file")
                }
            }
        }
    }

    // 6. Schema context — extract from index.schema directly
    let schema_context = index.schema.as_ref().map(|s| {
        // For each target file, find related tables via graph edges
        // Collect TableSchema summaries (table name, column count, FK count)
        // Render at Signature detail level
    });

    // 7. Blast radius — call existing function directly
    let blast = if opts.include_blast_radius {
        let top_paths: Vec<&str> = with_tests.iter().take(5).map(|f| f.path.as_str()).collect();
        Some(crate::intelligence::blast_radius::compute_blast_radius(
            &top_paths, &index.graph, &index.pagerank, &index.test_map, 3, opts.focus.as_deref(),
        ))
    } else { None };

    // 8. API surface — call existing function directly
    let api = crate::intelligence::api_surface::extract_api_surface(
        index, opts.focus.as_deref(), "all", 0, // budget handled by briefing step
    );

    // 9. Fill-then-overflow budget (NEW — from auto_context::briefing)
    let packed = crate::auto_context::briefing::allocate_and_pack(
        with_tests, schema_context, api, blast, opts.tokens, index,
    );

    // 10. Result assembly (annotations applied inside allocate_and_pack)
    AutoContextResult {
        task: task.to_string(),
        budget: packed.budget,
        sections: packed.sections,
        filtered_out: filtered.filtered_out,
    }
}
```

**Bridge functions needed:** Steps 5, 6, and 8 use inline logic to adapt existing APIs to the pipeline's data flow. These are NOT separate public functions — they are inline in `auto_context()`. The only new public functions are `filter_noise()` (Task 4) and `allocate_and_pack()` (Task 12).
```

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/auto_context/mod.rs
git commit -m "feat: auto_context orchestration pipeline composing all cxpak intelligence"
```

---

## Stream 4: Context Diff

### Task 14: Implement `ContextSnapshot` + delta computation

**Files:**
- Modify: `src/auto_context/diff.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_snapshot_creation() {
    // Create snapshot from index → verify file hashes populated
}

#[test]
fn test_diff_no_changes() {
    // Snapshot, then diff with same index → empty delta
}

#[test]
fn test_diff_file_modified() {
    // Snapshot, modify file content, diff → file in modified_files
}

#[test]
fn test_diff_file_added() { ... }
#[test]
fn test_diff_file_deleted() { ... }
#[test]
fn test_diff_new_symbol() { ... }
#[test]
fn test_diff_removed_symbol() { ... }
#[test]
fn test_diff_graph_edge_change() { ... }
#[test]
fn test_diff_no_snapshot() {
    // No prior auto_context → recommendation to call auto_context first
}
#[test]
fn test_diff_recommendation_text() { ... }
```

- [ ] **Step 2: Implement `ContextSnapshot`, `ContextDelta`, `compute_diff()`**

```rust
pub struct ContextSnapshot {
    pub file_hashes: HashMap<String, u64>,
    pub symbol_set: HashMap<String, Vec<String>>,
    pub edge_set: HashSet<(String, String, String)>,
}

pub struct ContextDelta { ... }

pub fn create_snapshot(index: &CodebaseIndex) -> ContextSnapshot { ... }
pub fn compute_diff(snapshot: &ContextSnapshot, index: &CodebaseIndex) -> ContextDelta { ... }
```

- [ ] **Step 3: Run tests, verify pass**

- [ ] **Step 4: Commit**

```bash
git add src/auto_context/diff.rs
git commit -m "feat: context snapshot and delta computation for session diffs"
```

---

## Stream 5: MCP + HTTP Wiring

### Task 15: Wire `auto_context` as MCP tool

**Files:**
- Modify: `src/commands/serve.rs`

- [ ] **Step 1: Add `ContextSnapshot` to MCP server state**

```rust
type SharedSnapshot = Arc<RwLock<Option<ContextSnapshot>>>;

struct AppState {
    index: SharedIndex,
    repo_path: SharedPath,
    snapshot: SharedSnapshot,  // NEW
}
```

Update the FULL MCP stdio call stack to thread `SharedSnapshot`:
- `run_mcp()` → create `SharedSnapshot`, pass to `mcp_stdio_loop()`
- `mcp_stdio_loop()` → pass to `mcp_stdio_loop_with_io()`
- `mcp_stdio_loop_with_io()` → pass to `handle_tool_call()`
- `handle_tool_call()` → accepts `snapshot: &SharedSnapshot`

All four function signatures gain the snapshot parameter. The HTTP path threads it via `AppState`.

- [ ] **Step 2: Add `cxpak_auto_context` to tools/list — LISTED FIRST (position 0)**

Move auto_context to the top of the tools array with prominent description.

- [ ] **Step 3: Implement handler**

Parse args → call `auto_context()` → store snapshot (write lock) → serialize result.

- [ ] **Step 4: Add `POST /auto_context` HTTP endpoint**

- [ ] **Step 5: Write MCP round-trip test**

- [ ] **Step 6: Commit**

```bash
git add src/commands/serve.rs
git commit -m "feat: add cxpak_auto_context MCP tool (#10, listed first)"
```

### Task 16: Wire `context_diff` as MCP tool

**Files:**
- Modify: `src/commands/serve.rs`

- [ ] **Step 1: Add `cxpak_context_diff` to tools/list**

- [ ] **Step 2: Implement handler**

Parse args → read snapshot (read lock) → if None, return "call auto_context first" → compute diff → serialize.

For `since: "HEAD~1"` or other git ref: use existing diff infrastructure instead of snapshot.

- [ ] **Step 3: Add `POST /context_diff` HTTP endpoint**

- [ ] **Step 4: Write MCP round-trip tests**

- [ ] **Step 5: Update tools/list test to expect 11 tools**

- [ ] **Step 6: Commit**

```bash
git add src/commands/serve.rs
git commit -m "feat: add cxpak_context_diff MCP tool (#11)"
```

### Task 17: Add `daemon` to default features

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add `"daemon"` to `default` feature list**

- [ ] **Step 2: Verify compilation with default features**

Run: `cargo check`

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "feat: add daemon to default features (MCP server always available)"
```

---

## Stream 6: Integration + Documentation + QA

### Task 18: Integration tests

**Files:**
- Add tests

- [ ] **Step 1: Write end-to-end tests**

```rust
#[test]
fn test_auto_context_matches_manual_workflow() {
    // Call auto_context → verify results match what context_for_task + pack_context would produce
}
#[test]
fn test_auto_context_with_embeddings_improves_semantic() {
    // "find authentication logic" → with embeddings finds validate_jwt, without might not
}
#[test]
fn test_auto_context_noise_filtering_end_to_end() {
    // Repo with vendor/ + generated files → all filtered
}
#[test]
fn test_context_diff_after_file_change() {
    // auto_context → modify file → context_diff → delta accurate
}
#[test]
fn test_full_mcp_session() {
    // auto_context → context_diff → auto_context again (new snapshot replaces old)
}
#[test]
fn test_byok_config_loading() {
    // .cxpak.json with voyageai config → provider detected
}
#[test]
fn test_graceful_embedding_fallback() {
    // Invalid API key → falls back to local → falls back to 6 signals
}
#[test]
fn test_auto_context_all_sections_present() { ... }
#[test]
fn test_large_repo_all_features() { ... }
#[test]
fn test_empty_repo() { ... }
```

- [ ] **Step 2: Run all tests**

Run: `cargo test --verbose`

- [ ] **Step 3: Commit**

```bash
git add tests/ src/
git commit -m "test: integration tests for v1.0.0 auto_context + embeddings + context_diff"
```

### Task 19: Documentation

**Files:**
- Modify: `README.md`, `.claude/CLAUDE.md`, `plugin/README.md`

- [ ] **Step 1: Rewrite README.md for v1.0.0**

- `auto_context` as hero feature
- Noise filtering explained
- Embeddings (local + BYOK) documented
- `.cxpak.json` configuration reference
- Full 11-tool MCP reference
- Stable API guarantee
- Changelog from 0.10.0 → 1.0.0

- [ ] **Step 2: Update CLAUDE.md**

- Add auto_context + embeddings modules to architecture
- Update tool count to 11
- Document stable API

- [ ] **Step 3: Update plugin/README.md**

- auto_context as primary tool
- Updated tool list

- [ ] **Step 4: Commit**

```bash
git add README.md .claude/CLAUDE.md plugin/README.md
git commit -m "docs: v1.0.0 documentation — auto_context, embeddings, stable API"
```

### Task 20: Version bump

- [ ] **Step 1: Bump to 1.0.0** in Cargo.toml, plugin.json, marketplace.json, ensure-cxpak

- [ ] **Step 2: Commit**

```bash
git add Cargo.toml plugin/.claude-plugin/plugin.json .claude-plugin/marketplace.json plugin/lib/ensure-cxpak
git commit -m "chore: bump version to 1.0.0"
```

### Task 21: Pre-Release QA + CI Validation

**This task MUST pass before tagging and pushing.**

- [ ] **Step 1: Run full test suite** — `cargo test --verbose`
- [ ] **Step 2: Run clippy** — `cargo clippy --all-targets -- -D warnings`
- [ ] **Step 3: Run formatter** — `cargo fmt -- --check`
- [ ] **Step 4: Run coverage** — `cargo tarpaulin --verbose --all-features --workspace --timeout 120 --fail-under 90`

- [ ] **Step 5: Manual QA — auto_context**

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_auto_context","arguments":{"task":"fix the authentication bug"}}}' | cargo run --features daemon -- serve --mcp .
```
Verify: structured briefing with all sections, annotations, filtered_out, budget summary.

- [ ] **Step 6: Manual QA — noise filtering**

Create temp repo with `vendor/` and generated files. Run auto_context. Verify they appear in `filtered_out`.

- [ ] **Step 7: Manual QA — embeddings (local)**

```bash
# First run — should download model
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_auto_context","arguments":{"task":"find authentication code"}}}' | cargo run -- serve --mcp .
```
Verify: model downloads to `~/.cxpak/models/`, results include embedding-boosted matches.

- [ ] **Step 8: Manual QA — embeddings (BYOK)**

Create `.cxpak.json` with OpenAI provider. Set `OPENAI_API_KEY`. Run auto_context. Verify API provider used.

- [ ] **Step 9: Manual QA — context_diff**

Call auto_context. Modify a file. Call context_diff. Verify delta is accurate.

- [ ] **Step 10: Manual QA — auto_context listed first in tools/list**

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | cargo run -- serve --mcp .
```
Verify: `cxpak_auto_context` is the first tool in the array.

- [ ] **Step 11: Simulate CI**

```bash
cargo build --verbose && cargo test --verbose && cargo clippy --all-targets -- -D warnings && cargo fmt -- --check && cargo tarpaulin --verbose --all-features --workspace --timeout 120 --fail-under 90
```

- [ ] **Step 12: Tag and push**

```bash
git tag v1.0.0
git push origin main --tags
```

---

## Task Summary

| Stream | Tasks | Dependencies |
|---|---|---|
| 1. Noise Filtering | Tasks 1-4 | Sequential |
| 2. Embeddings | Tasks 5-11 | Task 5 first (scaffold), then 6-9 can partially parallel, 10-11 sequential |
| 3. Auto Context | Tasks 12-13 | Tasks 1-4 (noise), Task 11 (embedding signal) |
| 4. Context Diff | Task 14 | Task 1 (module scaffold) — can run parallel with Tasks 12-13 since it only needs CodebaseIndex structure, not auto_context itself |
| 5. MCP Wiring | Tasks 15-17 | Tasks 13 + 14 |
| 6. Integration + QA | Tasks 18-21 | All prior |

**Parallelizable:** After Task 1 (scaffold + both main.rs edits), Streams 1 and 2 are independent. Task 14 (context diff) can also run parallel with Tasks 12-13 since it only needs the CodebaseIndex structure.

**Critical path:** Task 1 → (Tasks 2-4 ∥ Tasks 5-11 ∥ Task 14) → Tasks 12-13 → Tasks 15-17 → Tasks 18-21

**Total: 21 tasks, 70 new tests, 100% branch coverage on new modules, 95%+ on modified modules, 90%+ overall CI gate. Task 21 is the release gate — no tag/push until all QA passes. This is v1.0.0.**
