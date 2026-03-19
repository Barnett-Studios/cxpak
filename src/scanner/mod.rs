pub mod defaults;

use std::fmt;
use std::path::{Path, PathBuf};

use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;

use defaults::BUILTIN_IGNORES;

/// A single file discovered by the scanner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    /// Path relative to the scanned root, using forward slashes.
    pub relative_path: String,
    /// Absolute path on disk.
    pub absolute_path: PathBuf,
    /// Detected programming language, if any.
    pub language: Option<String>,
    /// File size in bytes.
    pub size_bytes: u64,
}

/// Errors that can occur during scanning.
#[derive(Debug)]
pub enum ScanError {
    /// The given root path does not contain a `.git` directory, so it is not
    /// recognised as a repository root.
    NotARepository(PathBuf),
    /// An I/O or `ignore`-crate error occurred during directory walking.
    Walk(String),
    /// Failed to build the override rule set.
    Override(String),
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScanError::NotARepository(p) => {
                write!(
                    f,
                    "not a git repository (no .git directory found in {})",
                    p.display()
                )
            }
            ScanError::Walk(msg) => write!(f, "directory walk error: {msg}"),
            ScanError::Override(msg) => write!(f, "override builder error: {msg}"),
        }
    }
}

impl std::error::Error for ScanError {}

/// Scans a repository root, honouring `.gitignore`, an optional `.cxpakignore`,
/// and a set of built-in default patterns.
pub struct Scanner {
    root: PathBuf,
}

impl Scanner {
    /// Create a new `Scanner` rooted at `root`.
    ///
    /// Returns `ScanError::NotARepository` when `root/.git` does not exist.
    pub fn new(root: &Path) -> Result<Self, ScanError> {
        let git_dir = root.join(".git");
        if !git_dir.exists() {
            return Err(ScanError::NotARepository(root.to_path_buf()));
        }
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    /// Walk the repository and return all matching files, sorted by relative path.
    pub fn scan(&self) -> Result<Vec<ScannedFile>, ScanError> {
        // Build override rules that exclude built-in patterns.
        // The `ignore` crate treats overrides as *include* rules; prefixing with `!`
        // makes them negative (i.e. "exclude these").
        let mut override_builder = OverrideBuilder::new(&self.root);
        for pattern in BUILTIN_IGNORES {
            // A `!` prefix means "this path should NOT be included".
            let negated = format!("!{pattern}");
            override_builder
                .add(&negated)
                .map_err(|e| ScanError::Override(e.to_string()))?;
        }
        let overrides = override_builder
            .build()
            .map_err(|e| ScanError::Override(e.to_string()))?;

        // Build the walker.
        let mut builder = WalkBuilder::new(&self.root);
        builder
            .git_ignore(true) // respect .gitignore
            .git_global(false) // skip global gitignore for determinism
            .git_exclude(false) // skip .git/info/exclude for determinism
            .hidden(true) // visit hidden files (we handle .git via overrides)
            .overrides(overrides);

        // Load .cxpakignore if present.
        let cxpak_ignore = self.root.join(".cxpakignore");
        if cxpak_ignore.is_file() {
            builder.add_ignore(&cxpak_ignore);
        }

        let mut files: Vec<ScannedFile> = Vec::new();

        for result in builder.build() {
            let entry = result.map_err(|e| ScanError::Walk(e.to_string()))?;

            // Skip directories themselves; we only want files.
            if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                continue;
            }

            let absolute_path = entry.path().to_path_buf();

            // Compute relative path from root, normalised to forward slashes.
            let relative_path = absolute_path
                .strip_prefix(&self.root)
                .unwrap_or(&absolute_path)
                .to_string_lossy()
                .replace('\\', "/");

            let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);

            let language = detect_language(&absolute_path);

            files.push(ScannedFile {
                relative_path,
                absolute_path,
                language,
                size_bytes,
            });
        }

        files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

        Ok(files)
    }
}

/// Detect a programming language from a file's name or extension.
pub fn detect_language(path: &Path) -> Option<String> {
    // First: check by filename (for extensionless files like Dockerfile, Makefile)
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        let lang = match name {
            "Dockerfile" | "Makefile" | "GNUmakefile" => match name {
                "Dockerfile" => Some("dockerfile"),
                _ => Some("makefile"),
            },
            _ if name.starts_with("Dockerfile.") => Some("dockerfile"),
            _ => None,
        };
        if let Some(l) = lang {
            return Some(l.to_string());
        }
    }

    // Case-sensitive check first (only .R needs this)
    let raw_ext = path.extension()?.to_string_lossy();
    if raw_ext.as_ref() == "R" {
        return Some("r".to_string());
    }

    // Then: check by extension (lowercased)
    let ext = raw_ext.to_lowercase();
    let lang = match ext.as_str() {
        // Existing languages
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "java" => "java",
        "py" => "python",
        "go" => "go",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" | "hh" | "cxx" => "cpp",
        "rb" => "ruby",
        "cs" => "csharp",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        // New Tier 1
        "sh" | "bash" => "bash",
        "php" => "php",
        "dart" => "dart",
        "scala" | "sc" => "scala",
        "lua" => "lua",
        "ex" | "exs" => "elixir",
        "zig" => "zig",
        "hs" => "haskell",
        "groovy" | "gradle" => "groovy",
        "m" | "mm" => "objc",
        "r" => "r",
        "jl" => "julia",
        "ml" => "ocaml",
        "mli" => "ocaml_interface",
        // New Tier 2
        "css" => "css",
        "scss" => "scss",
        "md" | "mdx" => "markdown",
        "json" => "json",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        "hcl" | "tf" | "tfvars" => "hcl",
        "proto" => "proto",
        "svelte" => "svelte",
        "mk" => "makefile",
        "html" | "htm" => "html",
        "graphql" | "gql" => "graphql",
        "xml" | "xsd" | "xsl" | "svg" => "xml",
        _ => return None,
    };
    Some(lang.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_git_repo(dir: &Path) {
        fs::create_dir_all(dir.join(".git")).unwrap();
    }

    #[test]
    fn test_scanner_not_a_repository() {
        let tmp = tempfile::tempdir().unwrap();
        match Scanner::new(tmp.path()) {
            Err(err) => assert!(
                format!("{err}").contains("not a git repository"),
                "unexpected error: {err}"
            ),
            Ok(_) => panic!("expected NotARepository error"),
        }
    }

    #[test]
    fn test_scanner_basic_scan() {
        let tmp = tempfile::tempdir().unwrap();
        setup_git_repo(tmp.path());
        fs::write(tmp.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(tmp.path().join("readme.txt"), "hello").unwrap();

        let scanner = Scanner::new(tmp.path()).unwrap();
        let files = scanner.scan().unwrap();
        assert!(files.len() >= 2);

        let rs_file = files.iter().find(|f| f.relative_path == "main.rs");
        assert!(rs_file.is_some());
        assert_eq!(rs_file.unwrap().language.as_deref(), Some("rust"));

        let txt_file = files.iter().find(|f| f.relative_path == "readme.txt");
        assert!(txt_file.is_some());
        assert_eq!(txt_file.unwrap().language, None);
    }

    #[test]
    fn test_scanner_cxpakignore() {
        let tmp = tempfile::tempdir().unwrap();
        setup_git_repo(tmp.path());
        fs::write(tmp.path().join("keep.rs"), "fn keep() {}").unwrap();
        fs::write(tmp.path().join("skip.rs"), "fn skip() {}").unwrap();
        fs::write(tmp.path().join(".cxpakignore"), "skip.rs\n").unwrap();

        let scanner = Scanner::new(tmp.path()).unwrap();
        let files = scanner.scan().unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
        assert!(paths.contains(&"keep.rs"), "keep.rs should be present");
        assert!(
            !paths.contains(&"skip.rs"),
            "skip.rs should be excluded by .cxpakignore"
        );
    }

    #[test]
    fn test_detect_language_existing_extensions() {
        let cases = vec![
            ("foo.rs", Some("rust")),
            ("foo.ts", Some("typescript")),
            ("foo.tsx", Some("typescript")),
            ("foo.js", Some("javascript")),
            ("foo.jsx", Some("javascript")),
            ("foo.mjs", Some("javascript")),
            ("foo.cjs", Some("javascript")),
            ("foo.java", Some("java")),
            ("foo.py", Some("python")),
            ("foo.go", Some("go")),
            ("foo.c", Some("c")),
            ("foo.h", Some("c")),
            ("foo.cpp", Some("cpp")),
            ("foo.hpp", Some("cpp")),
            ("foo.cc", Some("cpp")),
            ("foo.hh", Some("cpp")),
            ("foo.cxx", Some("cpp")),
            ("foo.rb", Some("ruby")),
            ("foo.cs", Some("csharp")),
            ("foo.swift", Some("swift")),
            ("foo.kt", Some("kotlin")),
            ("foo.kts", Some("kotlin")),
            ("foo.txt", None),
        ];
        for (filename, expected) in cases {
            let result = detect_language(Path::new(filename));
            assert_eq!(
                result.as_deref(),
                expected,
                "detect_language({filename}) = {:?}, expected {:?}",
                result,
                expected
            );
        }
    }

    #[test]
    fn test_detect_dockerfile() {
        assert_eq!(
            detect_language(Path::new("Dockerfile")),
            Some("dockerfile".to_string())
        );
        assert_eq!(
            detect_language(Path::new("Dockerfile.prod")),
            Some("dockerfile".to_string())
        );
        assert_eq!(
            detect_language(Path::new("src/Dockerfile")),
            Some("dockerfile".to_string())
        );
    }

    #[test]
    fn test_detect_makefile() {
        assert_eq!(
            detect_language(Path::new("Makefile")),
            Some("makefile".to_string())
        );
        assert_eq!(
            detect_language(Path::new("GNUmakefile")),
            Some("makefile".to_string())
        );
        assert_eq!(
            detect_language(Path::new("build/Makefile")),
            Some("makefile".to_string())
        );
        assert_eq!(
            detect_language(Path::new("rules.mk")),
            Some("makefile".to_string())
        );
    }

    #[test]
    fn test_detect_new_tier1_extensions() {
        let cases = vec![
            ("script.sh", "bash"),
            ("script.bash", "bash"),
            ("index.php", "php"),
            ("main.dart", "dart"),
            ("App.scala", "scala"),
            ("build.sc", "scala"),
            ("init.lua", "lua"),
            ("mix.ex", "elixir"),
            ("test.exs", "elixir"),
            ("main.zig", "zig"),
            ("Main.hs", "haskell"),
            ("build.groovy", "groovy"),
            ("build.gradle", "groovy"),
            ("ViewController.m", "objc"),
            ("ViewController.mm", "objc"),
            ("analysis.r", "r"),
            ("analysis.R", "r"),
            ("solver.jl", "julia"),
            ("parser.ml", "ocaml"),
            ("parser.mli", "ocaml_interface"),
        ];
        for (filename, expected) in cases {
            let result = detect_language(Path::new(filename));
            assert_eq!(
                result.as_deref(),
                Some(expected),
                "detect_language({filename}) = {:?}, expected Some({expected:?})",
                result,
            );
        }
    }

    #[test]
    fn test_detect_new_tier2_extensions() {
        let cases = vec![
            ("style.css", "css"),
            ("style.scss", "scss"),
            ("README.md", "markdown"),
            ("page.mdx", "markdown"),
            ("config.json", "json"),
            ("config.yml", "yaml"),
            ("config.yaml", "yaml"),
            ("Cargo.toml", "toml"),
            ("main.tf", "hcl"),
            ("vars.tfvars", "hcl"),
            ("config.hcl", "hcl"),
            ("service.proto", "proto"),
            ("App.svelte", "svelte"),
            ("index.html", "html"),
            ("index.htm", "html"),
            ("schema.graphql", "graphql"),
            ("schema.gql", "graphql"),
            ("config.xml", "xml"),
            ("schema.xsd", "xml"),
            ("transform.xsl", "xml"),
            ("icon.svg", "xml"),
        ];
        for (filename, expected) in cases {
            let result = detect_language(Path::new(filename));
            assert_eq!(
                result.as_deref(),
                Some(expected),
                "detect_language({filename}) = {:?}, expected Some({expected:?})",
                result,
            );
        }
    }

    #[test]
    fn test_detect_matlab_extension_maps_to_objc() {
        // .m defaults to objc, not matlab (known ambiguity — objc wins unconditionally)
        assert_eq!(
            detect_language(Path::new("script.m")),
            Some("objc".to_string())
        );
    }

    #[test]
    fn test_detect_unknown_returns_none() {
        assert_eq!(detect_language(Path::new("foo.txt")), None);
        assert_eq!(detect_language(Path::new("foo.unknown")), None);
        assert_eq!(detect_language(Path::new("foo")), None);
    }

    #[test]
    fn test_scan_error_display() {
        let not_repo = ScanError::NotARepository(PathBuf::from("/tmp/fake"));
        assert!(format!("{not_repo}").contains("not a git repository"));

        let walk_err = ScanError::Walk("bad entry".to_string());
        assert!(format!("{walk_err}").contains("directory walk error"));

        let override_err = ScanError::Override("bad pattern".to_string());
        assert!(format!("{override_err}").contains("override builder error"));
    }
}
