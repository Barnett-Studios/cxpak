use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Flat-matrix vector index. Vectors are stored contiguously:
/// `vectors[i * dims .. (i + 1) * dims]` is the embedding for `paths[i]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingIndex {
    paths: Vec<String>,
    path_index: HashMap<String, usize>,
    vectors: Vec<f32>,
    dims: usize,
}

impl EmbeddingIndex {
    /// Create a new empty index with the given dimensionality.
    pub fn new(dims: usize) -> Self {
        Self {
            paths: Vec::new(),
            path_index: HashMap::new(),
            vectors: Vec::new(),
            dims,
        }
    }

    /// Add or update the vector for a file path.
    ///
    /// If the path already exists its embedding is replaced in-place.
    /// Panics if `vector.len() != self.dims`.
    pub fn add(&mut self, path: impl Into<String>, vector: Vec<f32>) {
        assert_eq!(
            vector.len(),
            self.dims,
            "vector length {} does not match index dims {}",
            vector.len(),
            self.dims
        );

        let path = path.into();

        if let Some(&idx) = self.path_index.get(&path) {
            // Update existing entry in-place.
            let start = idx * self.dims;
            self.vectors[start..start + self.dims].copy_from_slice(&vector);
        } else {
            let idx = self.paths.len();
            self.path_index.insert(path.clone(), idx);
            self.paths.push(path);
            self.vectors.extend_from_slice(&vector);
        }
    }

    /// Search for the `top_k` most similar paths to `query_vector`.
    ///
    /// Returns `(path, cosine_similarity)` pairs sorted descending by similarity.
    /// Returns an empty Vec if `query.len() != self.dims`.
    pub fn search(&self, query: &[f32], top_k: usize) -> Vec<(String, f64)> {
        if query.len() != self.dims {
            return vec![];
        }
        if self.paths.is_empty() || top_k == 0 {
            return vec![];
        }

        let query_norm = l2_norm(query);
        if query_norm == 0.0 {
            return vec![];
        }

        let mut scores: Vec<(usize, f64)> = self
            .paths
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let v = &self.vectors[i * self.dims..(i + 1) * self.dims];
                let sim = cosine_sim_unnormed(query, v, query_norm);
                (i, sim)
            })
            .collect();

        scores.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
        scores.truncate(top_k);

        scores
            .into_iter()
            .map(|(i, sim)| (self.paths[i].clone(), sim))
            .collect()
    }

    /// Number of entries in the index.
    pub fn len(&self) -> usize {
        self.paths.len()
    }

    /// Returns true if the index contains no entries.
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Serialize to `path` using bincode 1.x.
    ///
    /// Uses a write-to-temp + rename pattern so a concurrent reader never
    /// observes a partially-written file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let bytes =
            bincode::serialize(self).map_err(|e| format!("bincode serialize error: {e}"))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &bytes).map_err(|e| format!("write error: {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("rename error: {e}"))
    }

    /// Deserialize from `path` using bincode 1.x.
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("read error: {e}"))?;
        bincode::deserialize(&bytes).map_err(|e| format!("bincode deserialize error: {e}"))
    }

    /// Cosine similarity of the stored vector for `path` against `query`.
    ///
    /// Returns `None` if the path is not in the index.
    /// Returns `Some(0.0)` if `query.len() != self.dims`.
    pub fn cosine_similarity(&self, path: &str, query: &[f32]) -> Option<f64> {
        if query.len() != self.dims {
            return Some(0.0);
        }
        let &idx = self.path_index.get(path)?;
        let v = &self.vectors[idx * self.dims..(idx + 1) * self.dims];
        let qn = l2_norm(query);
        if qn == 0.0 {
            return Some(0.0);
        }
        Some(cosine_sim_unnormed(query, v, qn))
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn l2_norm(v: &[f32]) -> f64 {
    let sum: f32 = v.iter().map(|x| x * x).sum();
    (sum as f64).sqrt()
}

/// Cosine similarity where caller supplies the pre-computed query norm.
fn cosine_sim_unnormed(a: &[f32], b: &[f32], a_norm: f64) -> f64 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let b_norm = l2_norm(b);
    if b_norm == 0.0 || a_norm == 0.0 {
        return 0.0;
    }
    (dot as f64) / (a_norm * b_norm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn unit_vec(dims: usize, val: f32) -> Vec<f32> {
        let mut v = vec![0.0f32; dims];
        v[0] = val;
        v
    }

    #[test]
    fn add_and_search() {
        let mut idx = EmbeddingIndex::new(4);
        idx.add("a.rs", vec![1.0, 0.0, 0.0, 0.0]);
        idx.add("b.rs", vec![0.0, 1.0, 0.0, 0.0]);
        idx.add("c.rs", vec![0.0, 0.0, 1.0, 0.0]);

        // Query exactly matches "a.rs"
        let results = idx.search(&[1.0, 0.0, 0.0, 0.0], 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "a.rs");
        assert!((results[0].1 - 1.0).abs() < 1e-6);
        // second result is one of the orthogonal ones with sim 0.0
        assert!((results[1].1).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity() {
        let mut idx = EmbeddingIndex::new(3);
        idx.add("file.rs", vec![1.0, 0.0, 0.0]);

        let sim = idx.cosine_similarity("file.rs", &[1.0, 0.0, 0.0]).unwrap();
        assert!((sim - 1.0).abs() < 1e-6, "identical vectors => sim=1.0");

        let sim2 = idx.cosine_similarity("file.rs", &[0.0, 1.0, 0.0]).unwrap();
        assert!((sim2).abs() < 1e-6, "orthogonal vectors => sim=0.0");

        assert!(idx
            .cosine_similarity("missing.rs", &[1.0, 0.0, 0.0])
            .is_none());
    }

    #[test]
    fn incremental_update() {
        let mut idx = EmbeddingIndex::new(2);
        idx.add("x.rs", vec![1.0, 0.0]);
        assert_eq!(idx.len(), 1);

        // Update same path — should replace, not add.
        idx.add("x.rs", vec![0.0, 1.0]);
        assert_eq!(idx.len(), 1, "update should not increase count");

        // The stored vector should now be [0.0, 1.0].
        let sim = idx.cosine_similarity("x.rs", &[0.0, 1.0]).unwrap();
        assert!((sim - 1.0).abs() < 1e-6, "updated vector should be used");
    }

    #[test]
    fn save_load() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("index.bin");

        let mut idx = EmbeddingIndex::new(3);
        idx.add("alpha.rs", vec![0.5, 0.5, 0.0]);
        idx.add("beta.rs", vec![0.0, 0.5, 0.5]);

        idx.save(&path).expect("save should succeed");
        let loaded = EmbeddingIndex::load(&path).expect("load should succeed");

        assert_eq!(loaded.len(), 2);
        let sim = loaded
            .cosine_similarity("alpha.rs", &[0.5, 0.5, 0.0])
            .unwrap();
        assert!(sim > 0.99, "loaded similarity: {sim}");
    }

    #[test]
    fn save_atomic_no_tmp_file_remains() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("index.bin");

        let mut idx = EmbeddingIndex::new(4);
        for i in 0..10 {
            idx.add(format!("file{i}.rs"), vec![i as f32 * 0.1, 0.0, 0.0, 0.0]);
        }

        idx.save(&path).expect("save should succeed");

        // The .tmp file must not persist after a successful save.
        let tmp = path.with_extension("tmp");
        assert!(!tmp.exists(), ".tmp file must not remain after save");

        // The destination file must exist and round-trip cleanly.
        assert!(path.exists(), "index.bin must exist after save");
        let loaded = EmbeddingIndex::load(&path).expect("load should succeed");
        assert_eq!(loaded.len(), 10, "all 10 entries must survive round-trip");

        // Verify a specific entry's vector is preserved.
        let sim = loaded
            .cosine_similarity("file3.rs", &[0.3, 0.0, 0.0, 0.0])
            .unwrap();
        assert!(
            sim > 0.99,
            "vector for file3.rs should be preserved; sim={sim}"
        );
    }

    #[test]
    fn empty_search() {
        let idx = EmbeddingIndex::new(4);
        let results = idx.search(&[1.0, 0.0, 0.0, 0.0], 5);
        assert!(results.is_empty(), "empty index should return no results");
    }

    #[test]
    fn search_top_k_respected() {
        let mut idx = EmbeddingIndex::new(2);
        for i in 0..10 {
            idx.add(format!("file{i}.rs"), unit_vec(2, i as f32 * 0.1 + 0.1));
        }
        let results = idx.search(&[1.0, 0.0], 3);
        assert_eq!(results.len(), 3);
        // Results should be sorted descending
        assert!(results[0].1 >= results[1].1);
        assert!(results[1].1 >= results[2].1);
    }

    #[test]
    fn search_returns_empty_on_dim_mismatch() {
        let mut idx = EmbeddingIndex::new(4);
        idx.add("a.rs", vec![1.0, 0.0, 0.0, 0.0]);

        // Query with wrong dimensionality must return empty vec.
        let results = idx.search(&[1.0, 0.0], 5);
        assert!(
            results.is_empty(),
            "search with wrong dims should return empty, got {results:?}"
        );
    }

    #[test]
    fn cosine_similarity_returns_zero_on_dim_mismatch() {
        let mut idx = EmbeddingIndex::new(4);
        idx.add("a.rs", vec![1.0, 0.0, 0.0, 0.0]);

        // Wrong dimensionality → Some(0.0), not None.
        let result = idx.cosine_similarity("a.rs", &[1.0, 0.0]);
        assert_eq!(
            result,
            Some(0.0),
            "dim mismatch should return Some(0.0), got {result:?}"
        );
    }
}
