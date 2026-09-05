pub const BUILTIN_IGNORES: &[&str] = &[
    "node_modules",
    "__pycache__",
    ".venv",
    "venv",
    "vendor",
    "target",
    "dist",
    "build",
    "out",
    ".next",
    ".cxpak",
    ".gradle",
    ".DS_Store",
    ".idea",
    ".vscode",
    "*.swp",
    "*.swo",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
    "poetry.lock",
    "Gemfile.lock",
    "go.sum",
    "*.png",
    "*.jpg",
    "*.jpeg",
    "*.gif",
    "*.ico",
    "*.svg",
    "*.woff",
    "*.woff2",
    "*.ttf",
    "*.eot",
    "*.mp3",
    "*.mp4",
    "*.zip",
    "*.tar.gz",
    "*.jar",
    "*.war",
    "*.class",
    "*.o",
    "*.so",
    "*.dylib",
    "*.dll",
    "*.exe",
    "*.wasm",
    "*.pyc",
    ".git",
    ".hg",
    ".svn",
    // ── Tool caches (cxpak#39 half 1) ─────────────────────────────────────────
    //
    // Reachable only since `hidden(false)`: while dotfiles were skipped wholesale
    // these cost nothing, and unhiding them without this list trades one silent
    // defect for another — a budget spent on `.mypy_cache` instead of on source.
    //
    // Deliberately NOT here: `.cache` and `.yarn`. Both are broad enough to hold
    // real source (Yarn PnP keeps `.yarn/patches` and `.yarn/releases`), and
    // excluding real source silently is the defect this ticket's half 1 is about.
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".tox",
    ".nox",
    ".terraform",
    ".turbo",
    ".parcel-cache",
    ".svelte-kit",
    ".nuxt",
    ".nyc_output",
    ".ipynb_checkpoints",
    ".sass-cache",
    ".dart_tool",
    ".stack-work",
    ".eslintcache",
    ".pnpm-store",
    "*.min.js",
    "*.min.css",
    "*.map",
    // ── Credential material (cxpak#39) ────────────────────────────────────────
    //
    // Measured on the published 3.1.4 image: a repo containing `id_rsa`,
    // `server.key` and `credentials.json` indexed all three, and `credentials.json`
    // was packed VERBATIM into the `overview` bundle. The only thing standing
    // between a committed private key and the model was whether the user's
    // `.gitignore` happened to cover it — `git_global(true)`'s comment already
    // banked on that ("often excludes .env, *.pem"), which is a hope, not a control.
    //
    // These are exact names and key-material extensions, deliberately NOT the
    // `*secret*` / `*credentials*` globs the ticket proposed. A glob that wide
    // silently drops `secrets_manager.rs`, `credentials_test.go` and
    // `SecretScanner.java` out of the index — real source vanishing from context
    // with no diagnostic, which is the same defect class this list is closing, in
    // the other direction.
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "*.jks",
    "*.keystore",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    "credentials.json",
    "credentials.yml",
    "credentials.yaml",
    "secrets.json",
    "secrets.yml",
    "secrets.yaml",
    ".netrc",
    ".npmrc",
    ".pypirc",
];
