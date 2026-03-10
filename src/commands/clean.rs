use std::path::Path;

pub fn run(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let cxpak_dir = path.join(".cxpak");
    if cxpak_dir.exists() {
        std::fs::remove_dir_all(&cxpak_dir)?;
        eprintln!("cxpak: removed {}", cxpak_dir.display());
    } else {
        eprintln!("cxpak: nothing to clean");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_existing_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let cxpak = dir.path().join(".cxpak");
        std::fs::create_dir_all(cxpak.join("cache")).unwrap();
        std::fs::write(cxpak.join("tree.md"), "data").unwrap();

        run(dir.path()).unwrap();
        assert!(!cxpak.exists());
    }

    #[test]
    fn test_clean_nonexistent_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        // Should not error
        run(dir.path()).unwrap();
    }
}
