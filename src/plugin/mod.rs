use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[cfg(feature = "plugins")]
pub mod loader;
pub mod manifest;
pub mod security;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginCapability {
    Analyzer,
    Detector,
    OutputFormat(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexSnapshot {
    pub files: Vec<FileSnapshot>,
    pub pagerank: HashMap<String, f64>,
    pub total_files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSnapshot {
    pub path: String,
    pub language: Option<String>,
    pub token_count: usize,
    pub content: Option<String>,
    pub public_symbols: Vec<String>,
    pub imports: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub kind: String,
    pub message: String,
    pub path: Option<String>,
    pub severity: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Detection {
    pub kind: String,
    pub message: String,
    pub line: Option<u32>,
    pub severity: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

pub trait CxpakPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn capabilities(&self) -> Vec<PluginCapability>;

    /// Run a whole-codebase analysis pass.
    ///
    /// Callers must verify `capabilities()` includes [`PluginCapability::Analyzer`]
    /// before invoking; see [`enforce_capability`].
    fn analyze(&self, index: &IndexSnapshot) -> Vec<Finding>;

    /// Run a per-file detection pass.
    ///
    /// Callers must verify `capabilities()` includes [`PluginCapability::Detector`]
    /// before invoking; see [`enforce_capability`].
    fn detect(&self, file: &FileSnapshot) -> Vec<Detection>;
}

/// Error returned when a plugin is invoked without the required capability.
#[derive(Debug)]
pub struct CapabilityError {
    pub missing: PluginCapability,
}

impl std::fmt::Display for CapabilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "plugin is missing required capability: {:?}",
            self.missing
        )
    }
}

impl std::error::Error for CapabilityError {}

/// Guard that enforces a capability is present before a plugin method is called.
///
/// Returns `Ok(())` when the plugin advertises `required`, or
/// `Err(CapabilityError)` when it does not.  Always call this before
/// [`CxpakPlugin::analyze`] (requiring [`PluginCapability::Analyzer`]) or
/// [`CxpakPlugin::detect`] (requiring [`PluginCapability::Detector`]).
pub fn enforce_capability(
    p: &dyn CxpakPlugin,
    required: PluginCapability,
) -> Result<(), CapabilityError> {
    if p.capabilities().contains(&required) {
        Ok(())
    } else {
        Err(CapabilityError { missing: required })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finding_round_trips_serde_json() {
        let finding = Finding {
            kind: "unused_import".to_string(),
            message: "Unused import detected".to_string(),
            path: Some("src/main.rs".to_string()),
            severity: "warning".to_string(),
            metadata: {
                let mut m = HashMap::new();
                m.insert("line".to_string(), serde_json::json!(42));
                m
            },
        };
        let serialized = serde_json::to_string(&finding).expect("serialize");
        let deserialized: Finding = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized.kind, finding.kind);
        assert_eq!(deserialized.message, finding.message);
        assert_eq!(deserialized.path, finding.path);
        assert_eq!(deserialized.severity, finding.severity);
        assert_eq!(deserialized.metadata["line"], serde_json::json!(42));
    }

    #[test]
    fn detection_round_trips_serde_json() {
        let detection = Detection {
            kind: "sql_injection".to_string(),
            message: "Possible SQL injection".to_string(),
            line: Some(15),
            severity: "error".to_string(),
            metadata: HashMap::new(),
        };
        let serialized = serde_json::to_string(&detection).expect("serialize");
        let deserialized: Detection = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized.kind, detection.kind);
        assert_eq!(deserialized.line, detection.line);
        assert_eq!(deserialized.severity, detection.severity);
    }

    struct StubPlugin {
        caps: Vec<PluginCapability>,
    }
    impl CxpakPlugin for StubPlugin {
        fn name(&self) -> &str {
            "stub"
        }
        fn version(&self) -> &str {
            "0.1"
        }
        fn capabilities(&self) -> Vec<PluginCapability> {
            self.caps.clone()
        }
        fn analyze(&self, _: &IndexSnapshot) -> Vec<Finding> {
            vec![]
        }
        fn detect(&self, _: &FileSnapshot) -> Vec<Detection> {
            vec![]
        }
    }

    #[test]
    fn enforce_capability_ok_when_present() {
        let p = StubPlugin {
            caps: vec![PluginCapability::Analyzer],
        };
        assert!(enforce_capability(&p, PluginCapability::Analyzer).is_ok());
    }

    #[test]
    fn enforce_capability_err_when_missing() {
        let p = StubPlugin {
            caps: vec![PluginCapability::Analyzer],
        };
        let result = enforce_capability(&p, PluginCapability::Detector);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Detector"));
    }

    #[test]
    fn enforce_capability_err_message_contains_missing_variant() {
        let p = StubPlugin { caps: vec![] };
        let err = enforce_capability(&p, PluginCapability::Analyzer).unwrap_err();
        assert!(err.to_string().contains("Analyzer"));
    }

    #[test]
    fn plugin_capability_output_format_serializes_correctly() {
        let cap = PluginCapability::OutputFormat("sarif".to_string());
        let serialized = serde_json::to_string(&cap).expect("serialize");
        let deserialized: PluginCapability =
            serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized, cap);

        let analyzer = PluginCapability::Analyzer;
        let s = serde_json::to_string(&analyzer).expect("serialize");
        let d: PluginCapability = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(d, analyzer);
    }
}
