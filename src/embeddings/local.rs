use std::io::{Read, Write};
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

/// Known-good SHA256 of each model file at the commit pinned in `HF_BASE`,
/// computed from the actual bytes served by Hugging Face for that commit.
/// This is also the authoritative list of file names cxpak fetches — there
/// is deliberately no separate "files to fetch" list that could drift out
/// of sync with it. A byte mismatch — a corrupted download, a tampered
/// mirror, or `HF_BASE` having drifted out of sync with this table — is
/// rejected rather than loaded.
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

/// How long to wait for the TCP connection to establish. Short — a dead or
/// unreachable host should fail fast.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to wait for the NEXT chunk of data once the transfer has
/// started. `model.safetensors` is ~90MB; a slow-but-progressing link must
/// be allowed to finish, so this bounds idle time between reads, not the
/// whole transfer — only a connection that stalls mid-stream times out.
const IDLE_READ_TIMEOUT: Duration = Duration::from_secs(30);

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

fn ensure_model_files(dir: &Path) -> Result<(), String> {
    ensure_model_files_at(dir, HF_BASE, MODEL_FILE_CHECKSUMS)
}

/// Core implementation, parameterized over the base URL and the expected
/// checksums so tests can point it at a local loopback fixture server
/// instead of the real Hugging Face endpoint, with no real network access
/// (cxpak#38).
fn ensure_model_files_at(
    dir: &Path,
    base_url: &str,
    checksums: &[(&str, &str)],
) -> Result<(), String> {
    for (name, expected) in checksums {
        let dest = dir.join(name);

        if dest.exists() {
            // A file left over from an older, unpinned fetch (or corrupted/
            // tampered on disk) must not be trusted silently just because it
            // exists. Re-validate by streaming its content through SHA256
            // (never buffer the whole file); a mismatch falls through to a
            // fresh, verified download instead of erroring outright.
            if matches!(sha256_of_file(&dest), Ok(actual) if actual == *expected) {
                continue;
            }
        }

        let url = format!("{base_url}/{name}");
        download_file_atomic(&url, &dest, expected)?;
    }
    Ok(())
}

/// Stream `path`'s content through SHA256 in bounded-size chunks.
///
/// Never buffers the whole file in memory, so this is as cheap to call on
/// the ~90MB `model.safetensors` as on `config.json`.
fn sha256_of_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| format!("read error: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("read error: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Download `url` to `dest` atomically via a temporary file + rename, using
/// the production `CONNECT_TIMEOUT`/`IDLE_READ_TIMEOUT` values.
fn download_file_atomic(url: &str, dest: &Path, expected_sha256: &str) -> Result<(), String> {
    download_file_atomic_with_timeouts(
        url,
        dest,
        expected_sha256,
        CONNECT_TIMEOUT,
        IDLE_READ_TIMEOUT,
    )
}

/// Core implementation, parameterized over both timeouts so tests can use
/// small values instead of the real production ones.
///
/// The response body is streamed directly into both the SHA256 hasher and
/// the temp file in one pass, in bounded-size chunks — it is never buffered
/// into a single in-memory `Vec`/`Bytes`, so peak memory stays bounded
/// regardless of the ~90MB `model.safetensors` size, and the bytes are
/// hashed exactly once (not read back from disk afterward to verify).
///
/// The file is written to `<dest>.tmp.<pid>` and then renamed to `dest`.
/// On Unix, `rename(2)` is atomic: if two processes race, one wins and the
/// other's rename simply fails with EEXIST (or silently overwrites on Linux),
/// so both see a complete, consistent file. If the rename fails because
/// another process already placed the final file, we remove the temp file and
/// accept the already-existing copy.
///
/// `<dest>.tmp.<pid>` is removed on EVERY error exit — connect/read/write
/// failure, idle-timeout, or checksum mismatch — never just the checksum
/// case, so a failed attempt never leaves a stray partial file behind.
///
/// `connect_timeout` bounds only establishing the connection. `idle_timeout`
/// is passed to `reqwest::blocking::ClientBuilder::timeout`, which — despite
/// the generic name — bounds each individual connect/read/write *operation*,
/// not the request's total wall-clock time (confirmed empirically: a
/// deliberately slow drip that never goes idle for longer than the timeout
/// completes even though the overall transfer exceeds it). So a
/// slow-but-progressing ~90MB transfer can still finish; only a connection
/// that never connects, or a read that goes idle mid-stream for longer than
/// `idle_timeout`, times out. A checksum mismatch or either timeout returns
/// `Err` here, which every caller up the chain (`LocalEmbeddingProvider::new`
/// → `create_provider` → `build_embedding_index`) already turns into "no
/// embedding index" rather than a hard failure — embeddings become
/// unavailable, the rest of the command proceeds.
fn download_file_atomic_with_timeouts(
    url: &str,
    dest: &Path,
    expected_sha256: &str,
    connect_timeout: Duration,
    idle_timeout: Duration,
) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(connect_timeout)
        .timeout(idle_timeout)
        .build()
        .map_err(|e| format!("http client build error: {e}"))?;

    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("download error for {url}: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {} downloading {url}", response.status()));
    }

    let tmp_path = dest.with_extension(format!("tmp.{}", std::process::id()));

    // Every error path below goes through this `result` before returning, so
    // the temp file is cleaned up uniformly regardless of which step failed.
    let result = write_verified_body(response, &tmp_path, expected_sha256, url);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    if let Err(e) = std::fs::rename(&tmp_path, dest) {
        // Another process already created the destination — clean up the temp
        // file and accept the already-existing copy.
        let _ = std::fs::remove_file(&tmp_path);
        if !dest.exists() {
            return Err(format!("rename failed and destination missing: {e}"));
        }
    }
    Ok(())
}

/// Stream `response`'s body into `tmp_path` in bounded 64KB chunks, hashing
/// as it goes, and verify the final digest against `expected_sha256`.
///
/// Each `response.read()` call is itself bounded by the client's configured
/// idle timeout (see `download_file_atomic_with_timeouts`), so a stalled
/// read surfaces here as an `io::Error` without any extra thread/channel
/// plumbing.
fn write_verified_body(
    mut response: reqwest::blocking::Response,
    tmp_path: &Path,
    expected_sha256: &str,
    url: &str,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};

    let mut tmp_file =
        std::fs::File::create(tmp_path).map_err(|e| format!("create temp file error: {e}"))?;

    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = response
            .read(&mut buf)
            .map_err(|e| format!("read bytes error: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        tmp_file
            .write_all(&buf[..n])
            .map_err(|e| format!("write error: {e}"))?;
    }
    tmp_file.flush().map_err(|e| format!("flush error: {e}"))?;

    let actual = format!("{:x}", hasher.finalize());
    if actual != expected_sha256 {
        return Err(format!(
            "integrity check failed for {url}: checksum mismatch: expected {expected_sha256}, got {actual}"
        ));
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
    fn hf_base_is_pinned_to_a_commit_not_main() {
        assert!(
            !HF_BASE.ends_with("/main"),
            "HF_BASE must pin a commit SHA, not the mutable `main` ref: {HF_BASE}"
        );
    }

    #[test]
    fn sha256_of_file_streams_a_correct_digest() {
        use sha2::{Digest, Sha256};

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("f.bin");
        let content = b"some file content, hashed two independent ways";
        std::fs::write(&path, content).expect("write fixture file");

        let expected = format!("{:x}", Sha256::digest(content));
        let actual = sha256_of_file(&path).expect("streaming hash should succeed");
        assert_eq!(
            actual, expected,
            "chunked streaming hash must match a direct digest of the same bytes"
        );
    }

    /// A single-shot HTTP/1.1 server on loopback that answers exactly one
    /// GET with a fixed body, then exits. No real network access — this
    /// stays entirely on 127.0.0.1, so it is safe and fast in CI, while
    /// still exercising the real `reqwest::blocking` download path.
    fn spawn_single_response_server(body: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
        let addr = listener.local_addr().expect("local_addr");
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // Drain (don't bother parsing) the request line/headers.
                let mut drain = [0u8; 4096];
                let _ = stream.read(&mut drain);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        (format!("http://{addr}"), handle)
    }

    /// A loopback server that advertises `advertised_len` via
    /// `Content-Length` but sends only `actual_body` (shorter) before
    /// closing the connection — a truncated transfer, which the client must
    /// surface as a read error rather than a clean EOF.
    fn spawn_truncated_response_server(
        advertised_len: usize,
        actual_body: Vec<u8>,
    ) -> (String, std::thread::JoinHandle<()>) {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
        let addr = listener.local_addr().expect("local_addr");
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut drain = [0u8; 4096];
                let _ = stream.read(&mut drain);
                let header =
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {advertised_len}\r\nConnection: close\r\n\r\n");
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&actual_body);
                let _ = stream.flush();
                // Drop the connection here — fewer bytes than advertised.
            }
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn download_file_atomic_cleans_up_after_truncated_response_read_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("weights.bin");

        // Advertise far more bytes than are actually sent, then close the
        // connection — reqwest/hyper must treat this as a read error, not a
        // clean end-of-stream.
        let (base_url, server) =
            spawn_truncated_response_server(10_000, b"only-a-few-bytes".to_vec());
        let url = format!("{base_url}/weights.bin");
        let wrong_checksum = "0".repeat(64);

        let result = download_file_atomic(&url, &dest, &wrong_checksum);

        assert!(
            result.is_err(),
            "a connection that closes before delivering the advertised body must surface as an error"
        );
        assert!(
            !dest.exists(),
            "a read error must not leave a file at the destination"
        );
        let leftover: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(|e| e.ok())
            .collect();
        assert!(
            leftover.is_empty(),
            "a read error must not leave a temp file behind either"
        );

        server.join().expect("server thread should exit cleanly");
    }

    #[test]
    fn download_file_atomic_cleans_up_when_temp_file_cannot_be_created() {
        let dir = tempfile::tempdir().expect("tempdir");
        // `dest`'s parent directory doesn't exist, so the temp file write
        // step fails before any bytes are written.
        let dest = dir.path().join("missing-subdir").join("weights.bin");

        let (base_url, server) = spawn_single_response_server(b"irrelevant-body".to_vec());
        let url = format!("{base_url}/weights.bin");

        let result = download_file_atomic(&url, &dest, &"0".repeat(64));

        assert!(
            result.is_err(),
            "a temp-file-creation failure must surface as an error, not succeed silently"
        );
        assert!(
            !dest.exists(),
            "nothing should be written to a missing directory"
        );
        // Strengthened per review: the whole tempdir root, not just `dest`,
        // must end up empty — no stray `.tmp.<pid>` file anywhere.
        let leftover: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(|e| e.ok())
            .collect();
        assert!(
            leftover.is_empty(),
            "a temp-file-creation failure must not leave anything behind in the tempdir: {leftover:?}"
        );

        server.join().expect("server thread should exit cleanly");
    }

    /// A loopback server that sends `initial_chunk` and then goes silent
    /// (without closing the connection) for `stall_for` before finally
    /// closing — simulates a connection that stalls mid-body rather than
    /// one that closes early.
    fn spawn_stall_mid_body_server(
        advertised_len: usize,
        initial_chunk: Vec<u8>,
        stall_for: Duration,
    ) -> (String, std::thread::JoinHandle<()>) {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
        let addr = listener.local_addr().expect("local_addr");
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut drain = [0u8; 4096];
                let _ = stream.read(&mut drain);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {advertised_len}\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&initial_chunk);
                let _ = stream.flush();
                // Go idle without sending more data or closing. Bounded so
                // this thread still exits; the client must time out well
                // before this elapses.
                std::thread::sleep(stall_for);
            }
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn download_file_atomic_times_out_on_a_mid_body_stall_near_the_idle_timeout() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("weights.bin");

        let idle_timeout = Duration::from_millis(500);
        // Stall for far longer than the idle timeout, but still bounded so
        // the server thread itself exits once joined.
        let (base_url, server) = spawn_stall_mid_body_server(
            10_000,
            b"first-bytes-then-silence".to_vec(),
            Duration::from_secs(2),
        );
        let url = format!("{base_url}/weights.bin");

        let start = std::time::Instant::now();
        let result = download_file_atomic_with_timeouts(
            &url,
            &dest,
            &"0".repeat(64),
            Duration::from_secs(2),
            idle_timeout,
        );
        let elapsed = start.elapsed();

        assert!(result.is_err(), "a mid-body stall must surface as an error");
        assert!(
            elapsed < idle_timeout * 4,
            "must time out near the configured idle timeout ({idle_timeout:?}), not reqwest's \
             30s default or the server's full stall duration; took {elapsed:?}"
        );
        assert!(!dest.exists());

        server.join().expect("server thread should exit cleanly");
    }

    /// A loopback server that sends `chunks` one at a time, sleeping
    /// `delay_between_chunks` before each — a slow-but-progressing transfer.
    fn spawn_drip_server(
        chunks: Vec<Vec<u8>>,
        delay_between_chunks: Duration,
    ) -> (String, std::thread::JoinHandle<()>) {
        use std::net::TcpListener;

        let total_len: usize = chunks.iter().map(|c| c.len()).sum();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
        let addr = listener.local_addr().expect("local_addr");
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut drain = [0u8; 4096];
                let _ = stream.read(&mut drain);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {total_len}\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.flush();
                for chunk in chunks {
                    std::thread::sleep(delay_between_chunks);
                    let _ = stream.write_all(&chunk);
                    let _ = stream.flush();
                }
            }
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn download_file_atomic_succeeds_on_a_slow_drip_whose_total_time_exceeds_the_idle_timeout() {
        use sha2::{Digest, Sha256};

        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("weights.bin");

        let chunks: Vec<Vec<u8>> = (0..4).map(|i| format!("chunk-{i}-").into_bytes()).collect();
        let full_body: Vec<u8> = chunks.concat();
        let expected = format!("{:x}", Sha256::digest(&full_body));

        let idle_timeout = Duration::from_millis(600);
        // 4 gaps of 300ms each: no single gap exceeds idle_timeout, but the
        // total (~1.2s) does — proving this is a per-read/idle bound, not a
        // cap on the whole transfer's wall-clock time.
        let (base_url, server) = spawn_drip_server(chunks, Duration::from_millis(300));
        let url = format!("{base_url}/weights.bin");

        let result = download_file_atomic_with_timeouts(
            &url,
            &dest,
            &expected,
            Duration::from_secs(2),
            idle_timeout,
        );

        assert!(
            result.is_ok(),
            "a slow-but-progressing transfer must not be killed by the idle timeout: {result:?}"
        );
        assert_eq!(std::fs::read(&dest).expect("dest should exist"), full_body);

        server.join().expect("server thread should exit cleanly");
    }

    #[test]
    fn download_file_atomic_rejects_wrong_checksum_and_leaves_no_files_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("weights.bin");

        let (base_url, server) =
            spawn_single_response_server(b"wrong-bytes-from-a-bad-mirror".to_vec());
        let url = format!("{base_url}/weights.bin");

        // Deliberately wrong SHA256 relative to the bytes the server serves.
        let wrong_checksum = "0".repeat(64);
        let result = download_file_atomic(&url, &dest, &wrong_checksum);

        assert!(
            result.is_err(),
            "a checksum mismatch must refuse the downloaded bytes, not accept them"
        );
        assert!(
            !dest.exists(),
            "a failed integrity check must not leave a file at the destination"
        );
        let leftover_tmp: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(|e| e.ok())
            .collect();
        assert!(
            leftover_tmp.is_empty(),
            "a failed download must not leave a temp file behind either"
        );

        server.join().expect("server thread should exit cleanly");
    }

    #[test]
    fn ensure_model_files_at_rejects_tampered_cache_and_redownloads_clean_copy() {
        use sha2::{Digest, Sha256};

        let dir = tempfile::tempdir().expect("tempdir");
        let good_content = b"the-real-model-bytes-served-by-the-fixture";
        let good_hash = format!("{:x}", Sha256::digest(good_content));

        // Plant a tampered file at the destination the loader checks first —
        // simulating an older unpinned cache entry or on-disk corruption.
        let dest = dir.path().join("weights.bin");
        std::fs::write(&dest, b"tampered-bytes-not-the-real-model").expect("plant tampered file");

        let (base_url, server) = spawn_single_response_server(good_content.to_vec());
        let checksums: &[(&str, &str)] = &[("weights.bin", good_hash.as_str())];

        ensure_model_files_at(dir.path(), &base_url, checksums)
            .expect("a tampered cache entry should be detected and transparently redownloaded");

        let final_bytes = std::fs::read(&dest).expect("dest should exist after redownload");
        assert_eq!(
            final_bytes, good_content,
            "the tampered cache file must be replaced with the verified download, not left in place"
        );

        server.join().expect("server thread should exit cleanly");
    }

    #[test]
    fn ensure_model_files_at_rejects_a_corrupted_upstream_response() {
        let dir = tempfile::tempdir().expect("tempdir");

        // The server serves bytes that do NOT match the pinned checksum —
        // simulating a corrupted transfer or a tampered mirror.
        let (base_url, server) =
            spawn_single_response_server(b"wrong-bytes-from-a-bad-mirror".to_vec());
        let wrong_checksum = "0".repeat(64);
        let checksums: &[(&str, &str)] = &[("weights.bin", wrong_checksum.as_str())];

        let result = ensure_model_files_at(dir.path(), &base_url, checksums);

        assert!(
            result.is_err(),
            "a response that fails checksum verification must not be accepted"
        );
        assert!(
            !dir.path().join("weights.bin").exists(),
            "a failed download must not leave a partial/incorrect file at the destination"
        );

        server.join().expect("server thread should exit cleanly");
    }
}
