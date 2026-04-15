use std::path::{Path, PathBuf};

/// Local inference provider using candle + all-MiniLM-L6-v2 (SafeTensors).
///
/// Model files are cached in `~/.cxpak/models/all-MiniLM-L6-v2/`.
pub struct LocalEmbeddingProvider {
    model: candle_transformers::models::bert::BertModel,
    tokenizer: tokenizers::Tokenizer,
    device: candle_core::Device,
    dims: usize,
}

const HF_BASE: &str = "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main";
const MODEL_DIMS: usize = 384;
const CACHE_SUBDIR: &str = "all-MiniLM-L6-v2";

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

        // Mean-pool over token dimension (dim 1), excluding padding tokens.
        let mask_f32 = attention_mask
            .to_dtype(candle_core::DType::F32)
            .map_err(|e| format!("dtype error: {e}"))?;
        // mask_f32: (n, seq_len), output: (n, seq_len, hidden)
        let mask_expanded = mask_f32
            .unsqueeze(2)
            .map_err(|e| format!("unsqueeze error: {e}"))?;
        let masked = (output * mask_expanded).map_err(|e| format!("mul error: {e}"))?;
        let summed = masked.sum(1).map_err(|e| format!("sum error: {e}"))?;
        let counts = mask_f32
            .sum(1)
            .map_err(|e| format!("sum mask error: {e}"))?
            .unsqueeze(1)
            .map_err(|e| format!("unsqueeze error: {e}"))?;
        let mean = (summed / counts).map_err(|e| format!("div error: {e}"))?;

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

fn model_cache_dir() -> Result<PathBuf, String> {
    #[allow(deprecated)]
    let home = std::env::home_dir().ok_or_else(|| "cannot find home directory".to_string())?;
    let dir = home.join(".cxpak").join("models").join(CACHE_SUBDIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create dirs error: {e}"))?;
    Ok(dir)
}

fn ensure_model_files(dir: &Path) -> Result<(), String> {
    let files = ["model.safetensors", "config.json", "tokenizer.json"];

    for name in files {
        let dest = dir.join(name);
        if dest.exists() {
            continue;
        }
        let url = format!("{HF_BASE}/{name}");
        download_file_atomic(&url, &dest)?;
    }
    Ok(())
}

/// Download `url` to `dest` atomically via a temporary file + rename.
///
/// The file is written to `<dest>.tmp.<pid>` and then renamed to `dest`.
/// On Unix, `rename(2)` is atomic: if two processes race, one wins and the
/// other's rename simply fails with EEXIST (or silently overwrites on Linux),
/// so both see a complete, consistent file. If the rename fails because
/// another process already placed the final file, we remove the temp file and
/// accept the already-existing copy.
fn download_file_atomic(url: &str, dest: &Path) -> Result<(), String> {
    let response =
        reqwest::blocking::get(url).map_err(|e| format!("download error for {url}: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {} downloading {url}", response.status()));
    }

    let bytes = response
        .bytes()
        .map_err(|e| format!("read bytes error: {e}"))?;

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
}
