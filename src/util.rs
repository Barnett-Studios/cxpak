use std::path::Path;

/// Replace dangerous bidi control characters (and other invisible
/// formatting controls) with a visible `<U+XXXX>` escape so downstream
/// renderers (HTML, terminal, IDE diagnostics) cannot be tricked into
/// displaying text whose visual order differs from its semantic order.
///
/// Mitigates "Trojan-source" style attacks (CVE-2021-42574) where an
/// identifier like `let access_level = "user\u{202E}//\u{202D}admin"`
/// renders as `let access_level = "user//admin"` while semantically
/// containing a comment.  cxpak indexes user-supplied source paths and
/// symbol names; without filtering, a malicious repo could spoof labels
/// in the SPA dashboard, LSP diagnostics, and search index.
///
/// Filtered codepoints (per Unicode bidi spec):
/// - U+202A LRE  Left-to-Right Embedding
/// - U+202B RLE  Right-to-Left Embedding
/// - U+202C PDF  Pop Directional Formatting
/// - U+202D LRO  Left-to-Right Override
/// - U+202E RLO  Right-to-Left Override
/// - U+2066 LRI  Left-to-Right Isolate
/// - U+2067 RLI  Right-to-Left Isolate
/// - U+2068 FSI  First Strong Isolate
/// - U+2069 PDI  Pop Directional Isolate
/// - U+200E LRM  Left-to-Right Mark
/// - U+200F RLM  Right-to-Left Mark
/// - U+061C ALM  Arabic Letter Mark
/// - U+200B ZWSP Zero-Width Space (homograph attack vector)
/// - U+200C ZWNJ Zero-Width Non-Joiner
/// - U+200D ZWJ  Zero-Width Joiner
///
/// Returns the input unchanged if it contains none of these — the
/// allocation cost is paid only when sanitisation is actually needed.
pub fn sanitize_bidi(s: &str) -> String {
    if !s.chars().any(is_dangerous_format_char) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if is_dangerous_format_char(c) {
            // Visible escape so the user sees attempts.
            out.push_str(&format!("<U+{:04X}>", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

#[inline]
fn is_dangerous_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{202A}'..='\u{202E}'  // LRE/RLE/PDF/LRO/RLO
            | '\u{2066}'..='\u{2069}'  // LRI/RLI/FSI/PDI
            | '\u{200B}'..='\u{200F}'  // ZWSP/ZWNJ/ZWJ/LRM/RLM
            | '\u{061C}'  // ALM
            | '\u{2060}'  // WJ Word Joiner
            | '\u{FEFF}'  // ZWNBSP/BOM
            | '\u{206A}'..='\u{206F}'  // deprecated format controls
    )
}

pub fn ensure_gitignore_entry(repo_root: &Path) -> std::io::Result<()> {
    let gitignore_path = repo_root.join(".gitignore");
    let entry = ".cxpak/";

    if gitignore_path.exists() {
        let content = std::fs::read_to_string(&gitignore_path)?;
        if content.lines().any(|line| line.trim() == entry) {
            return Ok(());
        }
        let separator = if content.ends_with('\n') { "" } else { "\n" };
        std::fs::write(&gitignore_path, format!("{content}{separator}{entry}\n"))
    } else {
        std::fs::write(&gitignore_path, format!("{entry}\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_creates_gitignore_with_cxpak() {
        let dir = TempDir::new().unwrap();
        ensure_gitignore_entry(dir.path()).unwrap();
        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains(".cxpak/"));
    }

    #[test]
    fn test_appends_to_existing_gitignore() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        ensure_gitignore_entry(dir.path()).unwrap();
        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains("target/"));
        assert!(content.contains(".cxpak/"));
    }

    #[test]
    fn test_idempotent_if_already_present() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n.cxpak/\n").unwrap();
        ensure_gitignore_entry(dir.path()).unwrap();
        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(content.matches(".cxpak/").count(), 1);
    }

    #[test]
    fn test_sanitize_word_joiner() {
        assert!(sanitize_bidi("a\u{2060}b").contains("<U+2060>"));
    }

    #[test]
    fn test_sanitize_bom() {
        assert!(sanitize_bidi("a\u{FEFF}b").contains("<U+FEFF>"));
    }

    #[test]
    fn test_sanitize_deprecated_format() {
        assert!(sanitize_bidi("a\u{206A}b").contains("<U+206A>"));
        assert!(sanitize_bidi("a\u{206F}b").contains("<U+206F>"));
    }
}
