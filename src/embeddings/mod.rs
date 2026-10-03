pub mod config;
pub mod index;
pub mod local;
pub mod remote;

pub use config::EmbeddingConfig;
pub use index::EmbeddingIndex;

/// Core trait that every embedding backend must satisfy.
pub trait EmbeddingProvider: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String>;
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String>;
    fn dimensions(&self) -> usize;
}

// ---------------------------------------------------------------------------
// Adapters — wrap concrete types so they implement EmbeddingProvider
// ---------------------------------------------------------------------------

impl EmbeddingProvider for local::LocalEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        self.embed(text)
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        self.embed_batch(texts)
    }

    fn dimensions(&self) -> usize {
        self.dimensions()
    }
}

impl EmbeddingProvider for remote::RemoteEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        self.embed(text)
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        self.embed_batch(texts)
    }

    fn dimensions(&self) -> usize {
        self.dimensions()
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Create the appropriate provider based on `config.provider`.
///
/// Returns `Err` if the provider cannot be initialized (e.g., missing API key).
pub fn create_provider(config: EmbeddingConfig) -> Result<Box<dyn EmbeddingProvider>, String> {
    match config.provider.as_str() {
        "local" => {
            let p = local::LocalEmbeddingProvider::new()?;
            Ok(Box::new(p))
        }
        _ => {
            let p = remote::RemoteEmbeddingProvider::new(config)?;
            Ok(Box::new(p))
        }
    }
}
