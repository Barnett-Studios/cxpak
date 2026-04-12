use super::manifest::PluginEntry;
use super::{Detection, Finding};

const MAX_RETURN_BYTES: usize = 1_048_576; // 1 MB

pub fn enforce_return_limit(
    findings: Vec<Finding>,
) -> Result<Vec<Finding>, Box<dyn std::error::Error>> {
    let serialized = serde_json::to_vec(&findings)?;
    if serialized.len() > MAX_RETURN_BYTES {
        return Err(format!(
            "plugin return exceeded 1 MB limit ({} bytes)",
            serialized.len()
        )
        .into());
    }
    Ok(findings)
}

pub fn enforce_detection_limit(
    detections: Vec<Detection>,
) -> Result<Vec<Detection>, Box<dyn std::error::Error>> {
    let serialized = serde_json::to_vec(&detections)?;
    if serialized.len() > MAX_RETURN_BYTES {
        return Err(format!(
            "plugin return exceeded 1 MB limit ({} bytes)",
            serialized.len()
        )
        .into());
    }
    Ok(detections)
}

pub fn warn_if_needs_content(entry: &PluginEntry) -> Option<String> {
    if entry.needs_content {
        Some(format!(
            "Plugin '{}' requests raw file content. Ensure you trust this plugin. Path: {}",
            entry.name, entry.path
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn small_finding() -> Finding {
        Finding {
            kind: "test".to_string(),
            message: "test finding".to_string(),
            path: Some("src/main.rs".to_string()),
            severity: "info".to_string(),
            metadata: HashMap::new(),
        }
    }

    fn large_findings() -> Vec<Finding> {
        // Create enough findings to exceed 1 MB
        let large_msg = "x".repeat(1024);
        (0..1100)
            .map(|i| Finding {
                kind: "bulk".to_string(),
                message: large_msg.clone(),
                path: Some(format!("src/file{i}.rs")),
                severity: "warning".to_string(),
                metadata: HashMap::new(),
            })
            .collect()
    }

    #[test]
    fn enforce_return_limit_within_limit_returns_ok() {
        let findings = vec![small_finding()];
        let result = enforce_return_limit(findings.clone());
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 1);
    }

    #[test]
    fn enforce_return_limit_exceeding_limit_returns_err() {
        let findings = large_findings();
        let result = enforce_return_limit(findings);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("exceeded 1 MB"),
            "error should mention 'exceeded 1 MB', got: {msg}"
        );
    }

    #[test]
    fn enforce_detection_limit_within_limit_returns_ok() {
        let detections = vec![Detection {
            kind: "sql_injection".to_string(),
            message: "possible injection".to_string(),
            line: Some(10),
            severity: "error".to_string(),
            metadata: HashMap::new(),
        }];
        let result = enforce_detection_limit(detections);
        assert!(result.is_ok());
    }

    #[test]
    fn enforce_detection_limit_exceeding_limit_returns_err() {
        let large_msg = "y".repeat(1024);
        let detections: Vec<Detection> = (0..1100)
            .map(|i| Detection {
                kind: "bulk".to_string(),
                message: large_msg.clone(),
                line: Some(i as u32),
                severity: "warning".to_string(),
                metadata: HashMap::new(),
            })
            .collect();
        let result = enforce_detection_limit(detections);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("exceeded 1 MB"),
            "error should mention 'exceeded 1 MB', got: {msg}"
        );
    }

    #[test]
    fn warn_if_needs_content_true_returns_some_warning() {
        let entry = PluginEntry {
            name: "risky-plugin".to_string(),
            path: "plugins/risky.wasm".to_string(),
            checksum: "abc".to_string(),
            file_patterns: vec![],
            needs_content: true,
        };
        let warning = warn_if_needs_content(&entry);
        assert!(warning.is_some());
        let msg = warning.unwrap();
        assert!(msg.contains("risky-plugin"), "got: {msg}");
        assert!(msg.contains("raw file content"), "got: {msg}");
    }

    #[test]
    fn warn_if_needs_content_false_returns_none() {
        let entry = PluginEntry {
            name: "safe-plugin".to_string(),
            path: "plugins/safe.wasm".to_string(),
            checksum: "abc".to_string(),
            file_patterns: vec![],
            needs_content: false,
        };
        assert!(warn_if_needs_content(&entry).is_none());
    }
}
