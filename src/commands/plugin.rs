//! CLI handlers for `cxpak plugin list|add`.

use crate::plugin::manifest::{load_manifest, PluginEntry, PluginsManifest};
use std::path::Path;

pub fn run_list(repo_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let manifest = load_manifest(repo_path)?;
    if manifest.plugins.is_empty() {
        println!("No plugins registered. Add one with: cxpak plugin add <path-to.wasm>");
        return Ok(());
    }
    println!(
        "Plugins registered in {}/.cxpak/plugins.json\n",
        repo_path.display()
    );
    for plugin in &manifest.plugins {
        println!("  {}", plugin.name);
        println!("    path:     {}", plugin.path);
        println!("    checksum: {}", plugin.checksum);
        println!("    patterns: {}", plugin.file_patterns.join(", "));
        println!(
            "    content:  {}",
            if plugin.needs_content {
                "YES (plugin will see raw file contents)"
            } else {
                "no"
            }
        );
        println!();
    }
    println!(
        "Total: {} plugin{}",
        manifest.plugins.len(),
        if manifest.plugins.len() == 1 { "" } else { "s" }
    );
    Ok(())
}

pub fn run_add(
    repo_path: &Path,
    wasm_path: &Path,
    name_override: Option<&str>,
    patterns: &[String],
    needs_content: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use sha2::{Digest, Sha256};

    // 1. Verify wasm file exists
    if !wasm_path.exists() {
        return Err(format!("wasm file not found: {}", wasm_path.display()).into());
    }
    if !wasm_path.is_file() {
        return Err(format!("not a regular file: {}", wasm_path.display()).into());
    }

    // 2. Derive name
    let name = match name_override {
        Some(n) => n.to_string(),
        None => wasm_path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or("cannot derive name from path; use --name")?
            .to_string(),
    };

    // 3. Patterns required
    if patterns.is_empty() {
        return Err("--patterns is required (e.g. --patterns '**/*.py')".into());
    }

    // 4. Relative path logic
    let repo_canonical = repo_path
        .canonicalize()
        .map_err(|e| format!("cannot canonicalize repo path: {e}"))?;
    let wasm_canonical = wasm_path
        .canonicalize()
        .map_err(|e| format!("cannot canonicalize wasm path: {e}"))?;
    let relative = wasm_canonical
        .strip_prefix(&repo_canonical)
        .map_err(|_| {
            format!(
                "wasm path must be inside the repo root ({})",
                repo_path.display()
            )
        })?
        .to_string_lossy()
        .to_string();

    // 5. Compute checksum
    let bytes = std::fs::read(wasm_path)?;
    let checksum = format!("{:x}", Sha256::digest(&bytes));

    // 6. Load or create manifest
    let mut manifest = load_manifest(repo_path).unwrap_or(PluginsManifest { plugins: vec![] });

    // 7. Check for duplicate name
    if manifest.plugins.iter().any(|p| p.name == name) {
        return Err(format!(
            "plugin '{name}' already registered; remove it first or use a different name"
        )
        .into());
    }

    // 8. Content access warning
    if needs_content {
        eprintln!(
            "WARNING: Plugin '{name}' will have access to raw file contents.\nEnsure you trust this plugin before proceeding.\n"
        );
    }

    // 9. Append and save
    let entry = PluginEntry {
        name: name.clone(),
        path: relative.clone(),
        checksum: checksum.clone(),
        file_patterns: patterns.to_vec(),
        needs_content,
    };
    manifest.plugins.push(entry);

    let manifest_dir = repo_path.join(".cxpak");
    std::fs::create_dir_all(&manifest_dir)?;
    let manifest_path = manifest_dir.join("plugins.json");
    let json = serde_json::to_string_pretty(&manifest)?;
    std::fs::write(&manifest_path, json)?;

    println!("Added plugin '{name}'");
    println!("  path:     {relative}");
    println!("  checksum: {checksum}");
    println!("  patterns: {}", patterns.join(", "));
    println!("  content:  {}", if needs_content { "yes" } else { "no" });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_list_empty_when_no_manifest() {
        let dir = TempDir::new().unwrap();
        assert!(run_list(dir.path()).is_ok());
    }

    #[test]
    fn test_add_creates_manifest() {
        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("plugins").join("test.wasm");
        std::fs::create_dir_all(wasm_path.parent().unwrap()).unwrap();
        std::fs::write(&wasm_path, b"fake wasm bytes").unwrap();

        let result = run_add(
            dir.path(),
            &wasm_path,
            None,
            &["**/*.py".to_string()],
            false,
        );
        assert!(result.is_ok(), "add should succeed: {result:?}");

        let manifest = load_manifest(dir.path()).unwrap();
        assert_eq!(manifest.plugins.len(), 1);
        assert_eq!(manifest.plugins[0].name, "test");
        assert_eq!(
            manifest.plugins[0].file_patterns,
            vec!["**/*.py".to_string()]
        );
        assert!(!manifest.plugins[0].needs_content);
    }

    #[test]
    fn test_add_rejects_duplicate_name() {
        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("test.wasm");
        std::fs::write(&wasm_path, b"fake").unwrap();

        run_add(
            dir.path(),
            &wasm_path,
            Some("dup"),
            &["*".to_string()],
            false,
        )
        .unwrap();
        let err = run_add(
            dir.path(),
            &wasm_path,
            Some("dup"),
            &["*".to_string()],
            false,
        );
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("already registered"));
    }

    #[test]
    fn test_add_rejects_missing_file() {
        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("missing.wasm");
        let result = run_add(dir.path(), &wasm_path, None, &["*".to_string()], false);
        assert!(result.is_err());
    }

    #[test]
    fn test_add_requires_patterns() {
        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("test.wasm");
        std::fs::write(&wasm_path, b"fake").unwrap();
        let result = run_add(dir.path(), &wasm_path, None, &[], false);
        assert!(result.is_err());
    }

    /// A wasm file inside the repo must be stored with a relative path (not absolute).
    #[test]
    fn test_add_stores_relative_path_for_file_inside_repo() {
        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("plugins").join("my.wasm");
        std::fs::create_dir_all(wasm_path.parent().unwrap()).unwrap();
        std::fs::write(&wasm_path, b"fake wasm").unwrap();

        run_add(
            dir.path(),
            &wasm_path,
            None,
            &["**/*.rs".to_string()],
            false,
        )
        .unwrap();

        let manifest = load_manifest(dir.path()).unwrap();
        assert_eq!(manifest.plugins.len(), 1);
        let stored_path = &manifest.plugins[0].path;
        assert!(
            !stored_path.starts_with('/'),
            "stored path should be relative, got: {stored_path}"
        );
        assert!(
            stored_path.contains("my.wasm"),
            "stored path should contain filename, got: {stored_path}"
        );
    }

    /// A wasm file outside the repo root must produce an Err.
    #[test]
    fn test_add_rejects_path_outside_repo() {
        let repo_dir = TempDir::new().unwrap();
        let outside_dir = TempDir::new().unwrap();
        let wasm_path = outside_dir.path().join("external.wasm");
        std::fs::write(&wasm_path, b"fake").unwrap();

        let result = run_add(
            repo_dir.path(),
            &wasm_path,
            None,
            &["**/*.py".to_string()],
            false,
        );
        assert!(result.is_err(), "wasm outside repo root must return Err");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("inside the repo root"),
            "error should mention 'inside the repo root', got: {msg}"
        );
    }

    /// The SHA-256 checksum stored in the manifest must be exactly 64 hex characters.
    #[test]
    fn test_add_checksum_is_64_hex_chars() {
        let dir = TempDir::new().unwrap();
        let wasm_path = dir.path().join("chk.wasm");
        std::fs::write(&wasm_path, b"some wasm content").unwrap();

        run_add(
            dir.path(),
            &wasm_path,
            None,
            &["**/*.ts".to_string()],
            false,
        )
        .unwrap();

        let manifest = load_manifest(dir.path()).unwrap();
        let checksum = &manifest.plugins[0].checksum;
        assert_eq!(
            checksum.len(),
            64,
            "SHA-256 hex checksum must be exactly 64 characters, got {}: {}",
            checksum.len(),
            checksum
        );
        assert!(
            checksum.chars().all(|c| c.is_ascii_hexdigit()),
            "checksum must contain only hex digits, got: {checksum}"
        );
    }
}
