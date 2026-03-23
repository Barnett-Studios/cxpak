use serde::{Deserialize, Serialize};
use std::path::Path;

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
    /// Build defaults for a given provider name.
    pub fn defaults_for(provider: &str) -> Self {
        match provider {
            "openai" => Self {
                provider: "openai".to_string(),
                model: "text-embedding-3-small".to_string(),
                api_key_env: "OPENAI_API_KEY".to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
                dimensions: 1536,
                batch_size: 128,
            },
            "voyageai" => Self {
                provider: "voyageai".to_string(),
                model: "voyage-code-3".to_string(),
                api_key_env: "VOYAGE_API_KEY".to_string(),
                base_url: "https://api.voyageai.com/v1".to_string(),
                dimensions: 1024,
                batch_size: 128,
            },
            "cohere" => Self {
                provider: "cohere".to_string(),
                model: "embed-english-v3.0".to_string(),
                api_key_env: "COHERE_API_KEY".to_string(),
                base_url: "https://api.cohere.com/v2".to_string(),
                dimensions: 1024,
                batch_size: 96,
            },
            _ => Self::local_default(),
        }
    }

    /// Returns the default local provider config.
    pub fn local_default() -> Self {
        Self {
            provider: "local".to_string(),
            model: "all-MiniLM-L6-v2".to_string(),
            api_key_env: String::new(),
            base_url: String::new(),
            dimensions: 384,
            batch_size: 64,
        }
    }

    /// Load from `.cxpak.json` in `path` directory. Falls back to local defaults
    /// if the file is missing or the `embeddings` section is absent.
    pub fn from_repo_root(path: &Path) -> Self {
        let config_path = path.join(".cxpak.json");
        let Ok(content) = std::fs::read_to_string(&config_path) else {
            return Self::local_default();
        };
        Self::from_json(&content)
    }

    /// Parse a JSON string. Expects `{"embeddings": {...}}` at the top level.
    /// Falls back to local defaults if the `embeddings` key is absent.
    pub fn from_json(json: &str) -> Self {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
            return Self::local_default();
        };

        let Some(emb) = value.get("embeddings") else {
            return Self::local_default();
        };

        // Determine provider first so we can fill in defaults for missing fields.
        let provider = emb
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("local")
            .to_string();

        let base = Self::defaults_for(&provider);

        Self {
            provider,
            model: emb
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or(&base.model)
                .to_string(),
            api_key_env: emb
                .get("api_key_env")
                .and_then(|v| v.as_str())
                .unwrap_or(&base.api_key_env)
                .to_string(),
            base_url: emb
                .get("base_url")
                .and_then(|v| v.as_str())
                .unwrap_or(&base.base_url)
                .to_string(),
            dimensions: emb
                .get("dimensions")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(base.dimensions),
            batch_size: emb
                .get("batch_size")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(base.batch_size),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_minimal() {
        let json = r#"{"embeddings": {"provider": "openai"}}"#;
        let cfg = EmbeddingConfig::from_json(json);
        assert_eq!(cfg.provider, "openai");
        assert_eq!(cfg.model, "text-embedding-3-small");
        assert_eq!(cfg.dimensions, 1536);
        assert_eq!(cfg.batch_size, 128);
        assert_eq!(cfg.api_key_env, "OPENAI_API_KEY");
        assert_eq!(cfg.base_url, "https://api.openai.com/v1");
    }

    #[test]
    fn parse_full() {
        let json = r#"{
            "embeddings": {
                "provider": "openai",
                "model": "text-embedding-ada-002",
                "api_key_env": "MY_KEY",
                "base_url": "https://custom.example.com/v1",
                "dimensions": 768,
                "batch_size": 32
            }
        }"#;
        let cfg = EmbeddingConfig::from_json(json);
        assert_eq!(cfg.provider, "openai");
        assert_eq!(cfg.model, "text-embedding-ada-002");
        assert_eq!(cfg.api_key_env, "MY_KEY");
        assert_eq!(cfg.base_url, "https://custom.example.com/v1");
        assert_eq!(cfg.dimensions, 768);
        assert_eq!(cfg.batch_size, 32);
    }

    #[test]
    fn parse_local_default() {
        let json = r#"{"embeddings": {"provider": "local"}}"#;
        let cfg = EmbeddingConfig::from_json(json);
        assert_eq!(cfg.provider, "local");
        assert_eq!(cfg.model, "all-MiniLM-L6-v2");
        assert_eq!(cfg.dimensions, 384);
        assert_eq!(cfg.batch_size, 64);
    }

    #[test]
    fn parse_no_embeddings_section() {
        let json = r#"{"other": "data"}"#;
        let cfg = EmbeddingConfig::from_json(json);
        assert_eq!(cfg.provider, "local");
        assert_eq!(cfg.model, "all-MiniLM-L6-v2");
        assert_eq!(cfg.dimensions, 384);
    }

    #[test]
    fn parse_from_file_not_found() {
        let dir = TempDir::new().unwrap();
        // No .cxpak.json in dir
        let cfg = EmbeddingConfig::from_repo_root(dir.path());
        assert_eq!(cfg.provider, "local");
    }

    #[test]
    fn provider_defaults_voyageai() {
        let cfg = EmbeddingConfig::defaults_for("voyageai");
        assert_eq!(cfg.provider, "voyageai");
        assert_eq!(cfg.model, "voyage-code-3");
        assert_eq!(cfg.api_key_env, "VOYAGE_API_KEY");
        assert_eq!(cfg.base_url, "https://api.voyageai.com/v1");
        assert_eq!(cfg.dimensions, 1024);
        assert_eq!(cfg.batch_size, 128);
    }

    #[test]
    fn provider_defaults_cohere() {
        let cfg = EmbeddingConfig::defaults_for("cohere");
        assert_eq!(cfg.provider, "cohere");
        assert_eq!(cfg.model, "embed-english-v3.0");
        assert_eq!(cfg.api_key_env, "COHERE_API_KEY");
        assert_eq!(cfg.base_url, "https://api.cohere.com/v2");
        assert_eq!(cfg.dimensions, 1024);
        assert_eq!(cfg.batch_size, 96);
    }
}
