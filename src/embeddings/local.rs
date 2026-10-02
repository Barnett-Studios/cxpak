use std::path::{Path, PathBuf};
use std::time::Duration;

/// Local inference provider using candle + all-MiniLM-L6-v2 (SafeTensors).
///
/// Model files are cached in `~/.cxpak/models/all-MiniLM-L6-v2/`.
pub struct LocalEmbeddingProvider {
    model: candle_transformers::models::bert::BertModel,
    tokenizer: tokenizers::Tokenizer,
    device: candle_core::Device,
    dims: usize,
}

/// Pinned at a specific commit (`1110a243fdf4706b3f48f1d95db1a4f5529b4d41`),
/// not `main` — `main` is a mutable ref that can change under us, silently
/// swapping weights a prior run already verified (cxpak#38). Bumping this
/// requires updating `MODEL_FILE_CHECKSUMS` below to match the new commit's
/// bytes.
const HF_BASE: &str =
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/1110a243fdf4706b3f48f1d95db1a4f5529b4d41";
const MODEL_DIMS: usize = 384;
const CACHE_SUBDIR: &str = "all-MiniLM-L6-v2";

/// Known-good SHA256 of each model file at `HF_COMMIT`, computed from the
/// actual bytes served by Hugging Face for that pinned commit. A byte
/// mismatch — a corrupted download, a tampered mirror, or `HF_COMMIT` having
/// drifted out of sync with this table — is rejected rather than loaded.
const MODEL_FILE_CHECKSUMS: &[(&str, &str)] = &[
    (
        "config.json",
        "953f9c0d463486b10a6871cc2fd59f223b2c70184f49815e7efbcab5d8908b41",
    ),
    (
        "tokenizer.json",
        "be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037",
    ),
    (
        "model.safetensors",
        "53aa51172d142c89d9012cce15ae4d6cc0ca6895895114379cacb4fab128d9db",
    ),
];

/// Network timeout for a single model-file download. `model.safetensors` is
/// ~90MB; a stalled or throttled connection must fail the embedding provider
/// rather than hang the caller indefinitely.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);

impl LocalEmbeddingProvider {
    /// Build the provider, downloading the model weights if necessary.
    pub fn new() -> Result<Self, String> {
        let cache_dir = model_cache_dir()?;
        ensure_model_files(&cache_dir)?;

        let device = candle_core::Device::Cpu;

        let tokenizer = tokenizers::Tokenizer::from_file(cache_dir.join("tokenizer.json"))
            .map_err(|e| format!("tokenizer load error: {e}"))?;

        // Load SafeTensors weights via buffered loader (no unsafe).
        let safetensors_bytes = std::fs::read(cache_dir.join("model.safetensors"))
            .map_err(|e| format!("model read error: {e}"))?;
        let vb = candle_nn::VarBuilder::from_buffered_safetensors(
            safetensors_bytes,
            candle_core::DType::F32,
            &device,
        )
        .map_err(|e| format!("varbuilder error: {e}"))?;

        let config_file = std::fs::File::open(cache_dir.join("config.json"))
            .map_err(|e| format!("config open error: {e}"))?;
        let config: candle_transformers::models::bert::Config =
            serde_json::from_reader(config_file).map_err(|e| format!("config parse error: {e}"))?;

        let model = candle_transformers::models::bert::BertModel::load(vb, &config)
            .map_err(|e| format!("model load error: {e}"))?;

        Ok(Self {
            model,
            tokenizer,
            device,
            dims: MODEL_DIMS,
        })
    }

    /// Embed a single text. Returns a normalized 384-dim vector.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let batch = self.embed_batch(&[text])?;
        batch
            .into_iter()
            .next()
            .ok_or_else(|| "empty batch result".to_string())
    }

    /// Embed a batch of texts. Returns one normalized vector per input.
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        use candle_core::Tensor;

        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| format!("tokenize error: {e}"))?;

        // Pad all sequences to the same length.
        let max_len = encodings.iter().map(|e| e.len()).max().unwrap_or(0);
        if max_len == 0 {
            return Ok(texts.iter().map(|_| vec![0.0f32; self.dims]).collect());
        }

        let n = texts.len();

        let mut input_ids_data = Vec::with_capacity(n * max_len);
        let mut attention_mask_data = Vec::with_capacity(n * max_len);
        let mut token_type_ids_data = Vec::with_capacity(n * max_len);

        for enc in &encodings {
            let ids = enc.get_ids();
            let mask = enc.get_attention_mask();
            let ttids = enc.get_type_ids();

            input_ids_data.extend(ids.iter().map(|&x| x as i64));
            attention_mask_data.extend(mask.iter().map(|&x| x as i64));
            token_type_ids_data.extend(ttids.iter().map(|&x| x as i64));

            // Pad to max_len.
            let pad = max_len - ids.len();
            for _ in 0..pad {
                input_ids_data.push(0);
                attention_mask_data.push(0);
                token_type_ids_data.push(0);
            }
        }

        let input_ids = Tensor::from_vec(input_ids_data, (n, max_len), &self.device)
            .map_err(|e| format!("tensor error: {e}"))?;
        let attention_mask = Tensor::from_vec(attention_mask_data, (n, max_len), &self.device)
            .map_err(|e| format!("tensor error: {e}"))?;
        let token_type_ids = Tensor::from_vec(token_type_ids_data, (n, max_len), &self.device)
            .map_err(|e| format!("tensor error: {e}"))?;

        let output = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_mask))
            .map_err(|e| format!("model forward error: {e}"))?;

        let mean = mean_pool(&output, &attention_mask)?;

        // L2-normalize each row.
        let mean_data: Vec<f32> = mean
            .flatten_all()
            .map_err(|e| format!("flatten error: {e}"))?
            .to_vec1()
            .map_err(|e| format!("to_vec1 error: {e}"))?;

        let mut result = Vec::with_capacity(n);
        for i in 0..n {
            let row = &mean_data[i * self.dims..(i + 1) * self.dims];
            result.push(l2_normalize(row));
        }

        Ok(result)
    }

    /// Dimensionality of the produced embeddings (384).
    pub fn dimensions(&self) -> usize {
        self.dims
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Mean-pool `output` (n, seq_len, hidden) over the token dimension, excluding
/// padding positions where `attention_mask` (n, seq_len) is 0. Returns (n, hidden).
fn mean_pool(
    output: &candle_core::Tensor,
    attention_mask: &candle_core::Tensor,
) -> Result<candle_core::Tensor, String> {
    // Mean-pool over token dimension (dim 1), excluding padding tokens.
    let mask_f32 = attention_mask
        .to_dtype(candle_core::DType::F32)
        .map_err(|e| format!("dtype error: {e}"))?;
    // mask_f32: (n, seq_len), output: (n, seq_len, hidden)
    let mask_expanded = mask_f32
        .unsqueeze(2)
        .map_err(|e| format!("unsqueeze error: {e}"))?;
    let masked = output
        .broadcast_mul(&mask_expanded)
        .map_err(|e| format!("mul error: {e}"))?;
    let summed = masked.sum(1).map_err(|e| format!("sum error: {e}"))?;
    let counts = mask_f32
        .sum(1)
        .map_err(|e| format!("sum mask error: {e}"))?
        .unsqueeze(1)
        .map_err(|e| format!("unsqueeze error: {e}"))?
        // Same clamp as sentence-transformers: an all-padding row pools to 0, not 0/0 = NaN.
        .maximum(1e-9)
        .map_err(|e| format!("clamp error: {e}"))?;
    let mean = summed
        .broadcast_div(&counts)
        .map_err(|e| format!("div error: {e}"))?;

    Ok(mean)
}

/// Resolve the home directory from environment variables.
///
/// Checks `HOME` (Unix) then `USERPROFILE` (Windows). Returns an error when
/// neither is set.
pub(crate) fn resolve_home_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| "cannot find home directory: neither HOME nor USERPROFILE set".to_string())
}

fn model_cache_dir() -> Result<PathBuf, String> {
    let home = resolve_home_dir()?;
    let dir = home.join(".cxpak").join("models").join(CACHE_SUBDIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create dirs error: {e}"))?;
    Ok(dir)
}

/// Look up the pinned SHA256 for a model file by name.
///
/// Returns `Err` for any name not in `MODEL_FILE_CHECKSUMS` — an unpinned
/// file is never downloaded, let alone trusted.
fn expected_sha256(name: &str) -> Result<&'static str, String> {
    MODEL_FILE_CHECKSUMS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, hash)| *hash)
        .ok_or_else(|| format!("no pinned checksum configured for model file '{name}'"))
}

/// Reject `bytes` unless its SHA256 matches `expected` exactly.
///
/// Reused idiom from `commands::plugin`'s manifest checksum
/// (`format!("{:x}", Sha256::digest(..))`) — this is the "checksum-mismatch
/// refuses the bytes" contract cxpak#38 asks for on the HF download path.
fn verify_sha256(bytes: &[u8], expected: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        return Err(format!(
            "checksum mismatch: expected {expected}, got {actual} ({} bytes)",
            bytes.len()
        ));
    }
    Ok(())
}

fn ensure_model_files(dir: &Path) -> Result<(), String> {
    let files = ["model.safetensors", "config.json", "tokenizer.json"];

    for name in files {
        let dest = dir.join(name);
        let expected = expected_sha256(name)?;

        if dest.exists() {
            // A file left over from an older, unpinned fetch (or corrupted/
            // tampered on disk) must not be trusted silently just because it
            // exists. Re-validate; a mismatch falls through to a fresh,
            // verified download instead of erroring outright.
            let existing = std::fs::read(&dest).map_err(|e| format!("read error: {e}"))?;
            if verify_sha256(&existing, expected).is_ok() {
                continue;
            }
        }

        let url = format!("{HF_BASE}/{name}");
        download_file_atomic(&url, &dest, expected)?;
    }
    Ok(())
}

/// Download `url` to `dest` atomically via a temporary file + rename, after
/// verifying the downloaded bytes against `expected_sha256`.
///
/// The file is written to `<dest>.tmp.<pid>` and then renamed to `dest`.
/// On Unix, `rename(2)` is atomic: if two processes race, one wins and the
/// other's rename simply fails with EEXIST (or silently overwrites on Linux),
/// so both see a complete, consistent file. If the rename fails because
/// another process already placed the final file, we remove the temp file and
/// accept the already-existing copy.
///
/// Bounded by `DOWNLOAD_TIMEOUT`; a checksum mismatch or timeout returns
/// `Err` here, which every caller up the chain (`LocalEmbeddingProvider::new`
/// → `create_provider` → `build_embedding_index`) already turns into "no
/// embedding index" rather than a hard failure — embeddings become
/// unavailable, the rest of the command proceeds.
fn download_file_atomic(url: &str, dest: &Path, expected_sha256: &str) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| format!("http client build error: {e}"))?;

    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("download error for {url}: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {} downloading {url}", response.status()));
    }

    let bytes = response
        .bytes()
        .map_err(|e| format!("read bytes error: {e}"))?;

    verify_sha256(&bytes, expected_sha256)
        .map_err(|e| format!("integrity check failed for {url}: {e}"))?;

    let tmp_path = dest.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp_path, &bytes).map_err(|e| format!("write error: {e}"))?;

    if let Err(e) = std::fs::rename(&tmp_path, dest) {
        // Another process already created the destination — clean up the temp
        // file and verify the existing file is readable.
        let _ = std::fs::remove_file(&tmp_path);
        if !dest.exists() {
            return Err(format!("rename failed and destination missing: {e}"));
        }
    }
    Ok(())
}

fn l2_normalize(v: &[f32]) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        return v.to_vec();
    }
    v.iter().map(|x| x / norm).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // An all-padding row has a zero token count; it must pool to zeros, never 0/0 = NaN,
    // which l2_normalize would pass straight into the index.
    #[test]
    fn test_mean_pool_all_padding_row_is_finite() {
        use candle_core::{Device, Tensor};
        let dev = Device::Cpu;
        let output = Tensor::from_vec(vec![1.0f32, 2.0, 3.0, 4.0], (2, 1, 2), &dev).unwrap();
        let mask = Tensor::from_vec(vec![1i64, 0], (2, 1), &dev).unwrap();

        let rows: Vec<Vec<f32>> = mean_pool(&output, &mask).unwrap().to_vec2().unwrap();

        assert_eq!(rows, vec![vec![1.0, 2.0], vec![0.0, 0.0]]);
    }

    // #137: pooling must broadcast the (n, seq, 1) mask over the hidden dimension,
    // exclude padding positions, and divide each row by its own token count.
    #[test]
    fn test_mean_pool_excludes_padding_and_broadcasts_over_hidden() {
        use candle_core::{Device, Tensor};
        let dev = Device::Cpu;
        // n=2, seq_len=3, hidden=2. Row 0 has 3 real tokens; row 1 has 1 real + 2 padding.
        let output = Tensor::from_vec(
            vec![
                1.0f32, 10.0, 2.0, 20.0, 3.0, 30.0, //
                4.0, 40.0, 99.0, 990.0, 99.0, 990.0,
            ],
            (2, 3, 2),
            &dev,
        )
        .unwrap();
        let mask = Tensor::from_vec(vec![1i64, 1, 1, 1, 0, 0], (2, 3), &dev).unwrap();

        let pooled = mean_pool(&output, &mask).expect("pooling must succeed");

        assert_eq!(pooled.dims(), &[2, 2]);
        let rows: Vec<Vec<f32>> = pooled.to_vec2().unwrap();
        assert_eq!(rows, vec![vec![2.0, 20.0], vec![4.0, 40.0]]);
    }

    #[test]
    #[ignore = "requires network to download model"]
    fn test_local_provider_single_embed() {
        let provider = LocalEmbeddingProvider::new().expect("should construct");
        let vec = provider
            .embed("fn hello() { println!(\"hello\"); }")
            .unwrap();
        assert_eq!(vec.len(), 384);
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "norm={norm}");
    }

    #[test]
    #[ignore = "requires network to download model"]
    fn test_local_provider_batch_embed() {
        let provider = LocalEmbeddingProvider::new().expect("should construct");
        let texts = vec!["fn foo() {}", "struct Bar {}"];
        let vecs = provider.embed_batch(&texts).unwrap();
        assert_eq!(vecs.len(), 2);
        assert_eq!(vecs[0].len(), 384);
        assert_eq!(vecs[1].len(), 384);
    }

    #[test]
    #[ignore = "requires network to download model"]
    fn test_local_provider_dimensions() {
        let provider = LocalEmbeddingProvider::new().expect("should construct");
        assert_eq!(provider.dimensions(), 384);
    }

    #[test]
    fn test_resolve_home_dir_uses_home_env() {
        // Verify that resolve_home_dir() reads from HOME, not deprecated
        // std::env::home_dir(). We set HOME to a known tempdir and confirm
        // the function returns a path under that directory.
        let dir = tempfile::tempdir().expect("tempdir");
        let dir_path = dir.path().to_path_buf();

        // Safety: single-threaded test; we restore or note that other tests
        // in this module are #[ignore] and don't rely on HOME.
        let original = std::env::var_os("HOME");
        std::env::set_var("HOME", &dir_path);

        let result = super::resolve_home_dir();

        // Restore before asserting to avoid leaking env state.
        match original {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        let resolved = result.expect("resolve_home_dir should succeed when HOME is set");
        assert_eq!(
            resolved, dir_path,
            "resolve_home_dir must return the HOME env var value"
        );
    }

    #[test]
    fn test_l2_normalize_unit_vector() {
        let v = vec![3.0f32, 4.0, 0.0];
        let n = l2_normalize(&v);
        let norm: f32 = n.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "norm={norm}");
        assert!((n[0] - 0.6).abs() < 1e-6);
        assert!((n[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn test_l2_normalize_zero_vector() {
        let v = vec![0.0f32, 0.0, 0.0];
        let n = l2_normalize(&v);
        assert_eq!(n, vec![0.0, 0.0, 0.0]);
    }

    // -----------------------------------------------------------------
    // cxpak#38: HF download integrity — checksum verification refuses
    // bytes that don't match the pinned revision.
    // -----------------------------------------------------------------

    #[test]
    fn verify_sha256_rejects_wrong_checksum() {
        let bytes = b"hello world";
        // Deliberately wrong SHA256 (64 hex chars, not the real digest).
        let wrong = "0000000000000000000000000000000000000000000000000000000000000000";
        let result = verify_sha256(bytes, wrong);
        assert!(
            result.is_err(),
            "a checksum mismatch must refuse the bytes, not accept them"
        );
    }

    #[test]
    fn verify_sha256_accepts_correct_checksum() {
        use sha2::{Digest, Sha256};
        let bytes = b"hello world";
        let correct = format!("{:x}", Sha256::digest(bytes));
        assert!(
            verify_sha256(bytes, &correct).is_ok(),
            "a matching checksum must be accepted"
        );
    }

    #[test]
    fn verify_sha256_rejects_tampered_bytes_with_correct_looking_hash() {
        use sha2::{Digest, Sha256};
        let original = b"model weights v1";
        let tampered = b"model weights v2 (tampered)";
        // The checksum was computed over `original`; `tampered` must fail
        // against it even though both are plausible byte strings.
        let expected = format!("{:x}", Sha256::digest(original));
        assert!(verify_sha256(tampered, &expected).is_err());
    }

    #[test]
    fn expected_sha256_known_files_match_pinned_table() {
        // Guards against the table and HF_BASE drifting independently: every
        // file cxpak actually fetches must have a pinned checksum.
        for name in ["model.safetensors", "config.json", "tokenizer.json"] {
            assert!(
                expected_sha256(name).is_ok(),
                "model file '{name}' has no pinned checksum"
            );
        }
    }

    #[test]
    fn expected_sha256_rejects_unknown_file() {
        assert!(
            expected_sha256("not-a-real-model-file.bin").is_err(),
            "an unpinned file name must not resolve to a checksum"
        );
    }

    #[test]
    fn hf_base_is_pinned_to_a_commit_not_main() {
        assert!(
            !HF_BASE.ends_with("/main"),
            "HF_BASE must pin a commit SHA, not the mutable `main` ref: {HF_BASE}"
        );
        assert!(
            HF_BASE.contains("1110a243fdf4706b3f48f1d95db1a4f5529b4d41"),
            "HF_BASE must pin the commit MODEL_FILE_CHECKSUMS was computed against"
        );
    }
}
