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
];

/// Credential-shaped filenames and key-material extensions (cxpak#39, #67, #78).
///
/// Measured on the published 3.1.4 image: a repo containing `id_rsa`, `server.key` and
/// `credentials.json` indexed all three, and `credentials.json` was packed VERBATIM into
/// the `overview` bundle. The only thing standing between a committed private key and the
/// model was whether the user's `.gitignore` happened to cover it — `git_global(true)`'s
/// comment already banked on that ("often excludes .env, *.pem"), which is a hope, not a
/// control.
///
/// These are exact names and key-material extensions, deliberately NOT the `*secret*` /
/// `*credentials*` globs the ticket proposed. A glob that wide silently drops
/// `secrets_manager.rs`, `credentials_test.go` and `SecretScanner.java` out of the index —
/// real source vanishing from context with no diagnostic, which is the same defect class
/// this list is closing, in the other direction.
///
/// **Kept separate from [`BUILTIN_IGNORES`] deliberately (cxpak#78 round 2).** That list
/// feeds `ignore::overrides::Override`, whose gitignore-style matching applies to a
/// DIRECTORY of the same name exactly as it does to a file, pruning the whole subtree —
/// correct for `node_modules`/`target`, wrong here: a bare `credentials` or `kubeconfig`
/// pattern silently dropped every file under a `src/credentials/` or `pkg/kubeconfig/`
/// directory, real source included, with no diagnostic (the exact defect class the
/// extensionless-glob comment above already rejected, reached by a different door). These
/// patterns are instead matched against each FILE's own basename only, post-walk (see
/// `Scanner::scan`), and a basename match is further exempted when `detect_language`
/// recognises its extension — `id_rsa_*` is the suffixed-ssh-key convention and also,
/// unavoidably given gitignore glob has no "stop before a dot" syntax, every
/// `id_rsa_*.{rs,go,...}` source file; the language check is what keeps `id_rsa_helper.rs`
/// in the index while still excluding the extensionless key file the pattern exists for.
pub const CREDENTIAL_IGNORES: &[&str] = &[
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
    // The extensionless sibling of `credentials.{json,yml,yaml}` — the AWS SDK/CLI's own
    // default filename, and the common form; the three covered above are the rarer ones.
    "credentials",
    // `ssh-keygen -f id_rsa_work` / `-f id_ed25519_github` — the standard multi-key
    // convention. The four exact names above only catch someone with exactly one key.
    // Suffix, not a bare `id_rsa*` glob: that would also swallow `id_rsa_work.pub`, a
    // PUBLIC key. Over-excluding a public key from the index is a usability regression,
    // not a security hole, so the asymmetry is accepted deliberately — but kept narrow
    // (underscore-separated, the convention `ssh-keygen` itself documents) rather than
    // widened to every possible rename, which no finite list closes anyway.
    "id_rsa_*",
    "id_dsa_*",
    "id_ecdsa_*",
    "id_ed25519_*",
    // OpenSSH private key under a conventional name with no `id_` prefix and no extension.
    "deploy_key",
    "*.p8",                 // Apple auth key (e.g. `AuthKey_ABC123.p8`)
    "*.ppk",                // PuTTY private key
    "*.gpg", // PGP secret (or public — same over-exclusion trade-off as id_rsa_*, see above) keyring
    "kubeconfig", // cluster certs + auth tokens
    "service-account.json", // GCP service-account key
    "terraform.tfstate",
    "terraform.tfstate.backup", // plaintext secrets in Terraform state
    // Dotfile credential stores in the identical position as .env/.netrc/.npmrc/.pypirc
    // above: protected ONLY by `hidden(true)` today, which #39's remaining half will lift.
    // Landing these before that change is the same reasoning PR #67 gave for .env.
    ".git-credentials", // plaintext https tokens
    ".pgpass",
    ".my.cnf",
    ".htpasswd",
    ".dockercfg",
    ".s3cfg",
    ".boto",
];
