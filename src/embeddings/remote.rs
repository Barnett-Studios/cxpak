use crate::embeddings::config::EmbeddingConfig;
use serde::Deserialize;

/// Remote HTTP embedding provider. Supports OpenAI-compatible APIs (OpenAI, VoyageAI, Ollama)
/// and Cohere's distinct format.
pub struct RemoteEmbeddingProvider {
    client: reqwest::blocking::Client,
    config: EmbeddingConfig,
    api_key: String,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    data: Vec<OpenAiEmbedding>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbedding {
    embedding: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct CohereResponse {
    embeddings: Vec<Vec<f32>>,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl RemoteEmbeddingProvider {
    /// Construct a provider from config. Reads the API key from the environment variable
    /// specified by `config.api_key_env`. Returns an error if the variable is not set.
    pub fn new(config: EmbeddingConfig) -> Result<Self, String> {
        let api_key = if config.api_key_env.is_empty() {
            String::new()
        } else {
            std::env::var(&config.api_key_env)
                .map_err(|_| format!("environment variable '{}' is not set", config.api_key_env))?
        };

        let client = reqwest::blocking::Client::new();
        Ok(Self {
            client,
            config,
            api_key,
        })
    }

    /// Embed a single text.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let mut results = self.embed_batch(&[text])?;
        results
            .pop()
            .ok_or_else(|| "empty embedding response".to_string())
    }

    /// Embed a batch of texts.
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        match self.config.provider.as_str() {
            "cohere" => self.embed_cohere(texts),
            _ => self.embed_openai_compat(texts),
        }
    }

    /// Dimensionality of the produced embeddings.
    pub fn dimensions(&self) -> usize {
        self.config.dimensions
    }

    // ---------------------------------------------------------------------------
    // Private helpers
    // ---------------------------------------------------------------------------

    fn embed_openai_compat(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        let url = format!("{}/embeddings", self.config.base_url);
        let body = serde_json::json!({
            "model": self.config.model,
            "input": texts,
        });

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .map_err(|e| format!("HTTP request error: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            return Err(format!("API error {status}: {text}"));
        }

        let parsed: OpenAiResponse = response
            .json()
            .map_err(|e| format!("JSON parse error: {e}"))?;

        Ok(parsed.data.into_iter().map(|e| e.embedding).collect())
    }

    fn embed_cohere(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        let url = format!("{}/embed", self.config.base_url);
        let body = serde_json::json!({
            "model": self.config.model,
            "texts": texts,
            "input_type": "search_document",
            "embedding_types": ["float"],
        });

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .map_err(|e| format!("HTTP request error: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            return Err(format!("Cohere API error {status}: {text}"));
        }

        let parsed: CohereResponse = response
            .json()
            .map_err(|e| format!("JSON parse error: {e}"))?;

        Ok(parsed.embeddings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn openai_config() -> EmbeddingConfig {
        EmbeddingConfig {
            provider: "openai".to_string(),
            model: "text-embedding-3-small".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            dimensions: 1536,
            batch_size: 128,
        }
    }

    fn cohere_config() -> EmbeddingConfig {
        EmbeddingConfig {
            provider: "cohere".to_string(),
            model: "embed-english-v3.0".to_string(),
            api_key_env: "COHERE_API_KEY".to_string(),
            base_url: "https://api.cohere.com/v2".to_string(),
            dimensions: 1024,
            batch_size: 96,
        }
    }

    #[test]
    fn missing_api_key_returns_error() {
        // Use a unique env var name that is guaranteed not to be set.
        let mut config = openai_config();
        config.api_key_env = "CXPAK_TEST_NONEXISTENT_KEY_XYZ".to_string();

        // Make sure it's not set in the environment.
        std::env::remove_var("CXPAK_TEST_NONEXISTENT_KEY_XYZ");

        let result = RemoteEmbeddingProvider::new(config);
        assert!(result.is_err(), "should fail when env var is not set");
        let msg = result.err().expect("expected error");
        assert!(
            msg.contains("CXPAK_TEST_NONEXISTENT_KEY_XYZ"),
            "error should mention the env var: {msg}"
        );
    }

    #[test]
    fn custom_base_url_used_correctly() {
        // Set a dummy API key so construction succeeds.
        std::env::set_var("CXPAK_TEST_KEY_FOR_URL_CHECK", "dummy-key");

        let mut config = openai_config();
        config.api_key_env = "CXPAK_TEST_KEY_FOR_URL_CHECK".to_string();
        config.base_url = "https://custom.example.com/v1".to_string();

        let provider = RemoteEmbeddingProvider::new(config).expect("construction should succeed");

        // The correct URL for OpenAI-compat would be base_url + /embeddings.
        // We can't make a real HTTP call, but we can verify the provider was created
        // and holds the correct config.
        assert_eq!(provider.config.base_url, "https://custom.example.com/v1");
        assert_eq!(provider.api_key, "dummy-key");

        std::env::remove_var("CXPAK_TEST_KEY_FOR_URL_CHECK");
    }

    #[test]
    fn cohere_provider_constructed_correctly() {
        std::env::set_var("COHERE_API_KEY", "test-cohere-key");

        let provider =
            RemoteEmbeddingProvider::new(cohere_config()).expect("construction should succeed");
        assert_eq!(provider.config.provider, "cohere");
        assert_eq!(provider.dimensions(), 1024);

        std::env::remove_var("COHERE_API_KEY");
    }

    #[test]
    fn empty_api_key_env_skips_env_lookup() {
        let config = EmbeddingConfig {
            provider: "local".to_string(),
            model: "all-MiniLM-L6-v2".to_string(),
            api_key_env: String::new(), // empty -> no env lookup
            base_url: String::new(),
            dimensions: 384,
            batch_size: 64,
        };
        let result = RemoteEmbeddingProvider::new(config);
        assert!(result.is_ok(), "empty api_key_env should succeed");
    }
}
