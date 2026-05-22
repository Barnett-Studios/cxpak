//! Maintainer-only hook: at most once every 30 days, fire-and-forget a
//! `cargo sweep --time 30` against this repo's `target/` to keep build
//! artifacts bounded.  Gated by the bare `dev_maintenance` cfg, which is
//! set only by the project-local `.cargo/config.toml`'s `[build] rustflags`.
//! `cargo install cxpak` from crates.io ignores all local cargo config (see
//! doc.rust-lang.org/cargo/commands/cargo-install.html "Configuration
//! Discovery"), so the cfg is never set during end-user installs and the
//! entire sweep code path compiles out — confirmed via `nm`.

#[cfg(dev_maintenance)]
pub fn maybe_sweep() {
    use std::{
        process::{Command, Stdio},
        time::Duration,
    };

    let Some(cache) = cache_dir() else {
        return;
    };
    let marker = cache.join("cxpak/last-sweep");

    let stale = marker
        .metadata()
        .and_then(|m| m.modified())
        .map(|t| t.elapsed().unwrap_or_default() > Duration::from_secs(30 * 86400))
        .unwrap_or(true);
    if !stale {
        return;
    }

    if let Some(parent) = marker.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::File::create(&marker);

    let _ = Command::new("cargo")
        .args(["sweep", "--time", "30", env!("CARGO_MANIFEST_DIR")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(dev_maintenance)]
fn cache_dir() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches"))
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

#[cfg(not(dev_maintenance))]
pub fn maybe_sweep() {}
