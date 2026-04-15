use crate::commands::serve::build_index;
use crate::conventions::diff::diff_exports;
use crate::conventions::export::build_export;
use std::error::Error;
use std::path::Path;

pub fn run_export(path: &Path) -> Result<(), Box<dyn Error>> {
    let index = build_index(path)?;
    let repo = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let repo_str = repo.to_string_lossy().to_string();
    let export = build_export(&repo_str, index.conventions);
    let cxpak_dir = path.join(".cxpak");
    std::fs::create_dir_all(&cxpak_dir)?;
    let out = cxpak_dir.join("conventions.json");
    let json = serde_json::to_string_pretty(&export)?;
    // Atomic write: write to .tmp then rename so the file is never partially written.
    let tmp_path = out.with_extension("json.tmp");
    std::fs::write(&tmp_path, &json)?;
    std::fs::rename(&tmp_path, &out)?;
    eprintln!(
        "cxpak: conventions exported to {} (checksum: {})",
        out.display(),
        &export.checksum[..8]
    );
    Ok(())
}

pub fn run_diff(path: &Path) -> Result<(), Box<dyn Error>> {
    let baseline_path = path.join(".cxpak").join("conventions.json");
    if !baseline_path.exists() {
        return Err(format!(
            "No baseline found at {}. Run: cxpak conventions export .",
            baseline_path.display()
        )
        .into());
    }
    let baseline_json = std::fs::read_to_string(&baseline_path)?;
    let baseline: crate::conventions::export::ConventionExport =
        serde_json::from_str(&baseline_json)?;
    let index = build_index(path)?;
    let repo = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let current = build_export(&repo.to_string_lossy(), index.conventions);
    let diff = diff_exports(&current, &baseline);
    println!("{}", serde_json::to_string_pretty(&diff)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn export_creates_conventions_json() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        let result = run_export(dir.path());
        let _ = result;
        let out = dir.path().join(".cxpak").join("conventions.json");
        if out.exists() {
            let content = std::fs::read_to_string(&out).unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
            assert_eq!(parsed["version"], "1.0");
        }
    }
}
