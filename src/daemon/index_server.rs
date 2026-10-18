//! Shared per-repo index server: discovery, spawn, and the server loop (ADR-0214).
//!
//! The server is `cxpak serve --mcp <repo>` re-exec'd under internal env vars (no new CLI
//! surface). One server exists per (canonical repo root, version, build id); a client that cannot
//! reach it falls back to its own in-process index.

#[cfg(unix)]
use super::ipc::{
    build_id, serve_requests, server_handshake, Handshake, SharedIndexTransport, UnixTransport,
};
use super::ipc::{short_hash, IpcError, SharedIndexConnection, Timeouts};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const ENV_SWITCH: &str = "CXPAK_INDEX_SERVER";
pub const ENV_IDLE_SECS: &str = "CXPAK_INDEX_SERVER_IDLE_SECS";
pub const ENV_SPAWN_WAIT_SECS: &str = "CXPAK_INDEX_SERVER_SPAWN_WAIT_SECS";
pub const ENV_LOCK_WAIT_SECS: &str = "CXPAK_INDEX_SERVER_LOCK_WAIT_SECS";
pub const ENV_REQUEST_TIMEOUT_SECS: &str = "CXPAK_INDEX_SERVER_REQ_TIMEOUT_SECS";
pub const ENV_ROLE: &str = "CXPAK_INDEX_SERVER_ROLE";
pub const ENV_SOCKET: &str = "CXPAK_INDEX_SERVER_SOCKET";
pub const ENV_BUILD_ID: &str = "CXPAK_INDEX_SERVER_BUILD_ID";

/// macOS `sun_path` is 104 bytes including the NUL; keep headroom.
pub const MAX_SOCKET_PATH: usize = 100;
const DEFAULT_IDLE_SECS: u64 = 600;
/// Bound on waiting for the per-server lock; a first call can wait this plus connect and spawn.
const DEFAULT_LOCK_WAIT_SECS: u64 = 3;
const LOG_ROTATE_BYTES: u64 = 256 * 1024;

/// Whether the shared server is enabled. `off`, `0`, `false` and `no` (any case) disable it.
pub fn enabled_with(get: impl Fn(&str) -> Option<String>) -> bool {
    get(ENV_SWITCH).is_none_or(|v| {
        !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "false" | "no"
        )
    })
}

/// Files of one server, all inside the private run directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub sock: PathBuf,
    pub lock: PathBuf,
    pub pid: PathBuf,
    pub log: PathBuf,
}

impl Paths {
    /// The sibling files of a socket path.
    pub fn from_sock(sock: &Path) -> Paths {
        Paths {
            sock: sock.to_path_buf(),
            lock: sock.with_extension("lock"),
            pid: sock.with_extension("pid"),
            log: sock.with_extension("log"),
        }
    }
}

/// Socket and sibling paths for (root, version, build id) under `run_dir`.
pub fn paths_for(run_dir: &Path, canonical_root: &Path, build_id: &str) -> Result<Paths, IpcError> {
    let h = short_hash(&[
        &canonical_root.to_string_lossy(),
        env!("CARGO_PKG_VERSION"),
        build_id,
    ]);
    let sock = run_dir.join(format!("cxpak-{h}.sock"));
    if sock.as_os_str().len() > MAX_SOCKET_PATH {
        return Err(IpcError::Unavailable(format!(
            "socket path exceeds {MAX_SOCKET_PATH} bytes"
        )));
    }
    Ok(Paths::from_sock(&sock))
}

/// Where the per-user run directory lives: `$XDG_RUNTIME_DIR`, else the per-user temp dir on
/// macOS, else `<cache dir>/cxpak`.
pub fn run_base(get: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    let abs = |name: &str| {
        get(name)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute() && !p.as_os_str().is_empty())
    };
    if let Some(p) = abs("XDG_RUNTIME_DIR") {
        return Some(p);
    }
    if cfg!(target_os = "macos") {
        return Some(std::env::temp_dir());
    }
    abs("XDG_CACHE_HOME")
        .or_else(|| abs("HOME").map(|h| h.join(".cache")))
        .map(|c| c.join("cxpak"))
}

#[cfg(unix)]
fn effective_uid() -> u32 {
    // SAFETY: `geteuid` takes no arguments, cannot fail, and touches no memory.
    unsafe { libc::geteuid() }
}

/// `dir` must be a real directory (not a symlink) owned by us with no group or world access.
#[cfg(unix)]
pub fn validate_run_dir(dir: &Path) -> Result<(), IpcError> {
    use std::os::unix::fs::MetadataExt;
    let unsafe_dir = |why: &str| IpcError::Unavailable(format!("run dir {}: {why}", dir.display()));
    let meta = std::fs::symlink_metadata(dir).map_err(IpcError::Io)?;
    if !meta.is_dir() {
        return Err(unsafe_dir("not a real directory"));
    }
    if meta.uid() != effective_uid() {
        return Err(unsafe_dir("not owned by the current user"));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(unsafe_dir("accessible by group or others"));
    }
    Ok(())
}

/// Create (0700) and validate `<base>/cxpak-<uid>`.
#[cfg(unix)]
pub fn private_run_dir(base: &Path) -> Result<PathBuf, IpcError> {
    use std::os::unix::fs::DirBuilderExt;
    let dir = base.join(format!("cxpak-{}", effective_uid()));
    match std::fs::symlink_metadata(&dir) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(base).map_err(IpcError::Io)?;
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&dir)
                .map_err(IpcError::Io)?;
        }
        Err(e) => return Err(IpcError::Io(e)),
    }
    validate_run_dir(&dir)?;
    Ok(dir)
}

/// Exclusive advisory lock on `path`, polled for at most `wait`. Released when the file drops.
#[cfg(unix)]
fn lock_exclusive(path: &Path, wait: Duration) -> Result<std::fs::File, IpcError> {
    use fs2::FileExt;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(IpcError::Io)?;
    let start = std::time::Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(file),
            Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                if start.elapsed() >= wait {
                    return Err(IpcError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(IpcError::Io(e)),
        }
    }
}

/// Append-only private log: never follows a symlink, never truncates; rotated by rename when large.
#[cfg(unix)]
fn open_log(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    if std::fs::metadata(path).is_ok_and(|m| m.len() > LOG_ROTATE_BYTES) {
        std::fs::rename(path, path.with_extension("log.1"))?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

/// `(dev, ino)` of the socket file at `path`.
#[cfg(unix)]
fn file_identity(path: &Path) -> std::io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(path)?;
    Ok((m.dev(), m.ino()))
}

/// Remove `sock` only when it is still the file this server bound. A newer server may own the path.
#[cfg(unix)]
pub fn unlink_if_ours(sock: &Path, ours: (u64, u64)) -> std::io::Result<bool> {
    match file_identity(sock) {
        Ok(id) if id == ours => std::fs::remove_file(sock).map(|()| true),
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn is_absent(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
    )
}

/// Everything a client needs to find or start a server.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub exe: PathBuf,
    /// Content hash of `exe`; the server is addressed and verified by it.
    pub build_id: String,
    pub run_dir: PathBuf,
    pub timeouts: Timeouts,
    pub lock_wait: Duration,
    pub spawn_wait: Duration,
    /// Extra environment for the spawned server (tests tune the idle timeout this way).
    pub child_env: Vec<(String, String)>,
}

#[cfg(unix)]
impl ClientConfig {
    /// Defaults plus the process environment.
    pub fn from_env() -> Result<Self, IpcError> {
        let base = run_base(&|k| std::env::var_os(k))
            .ok_or_else(|| IpcError::Unavailable("no run directory base".into()))?;
        let mut timeouts = Timeouts::default();
        if let Some(secs) = std::env::var(ENV_REQUEST_TIMEOUT_SECS)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|s| *s > 0)
        {
            timeouts.request = Duration::from_secs(secs);
        }
        let secs = |name: &str, default: u64| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|s| *s > 0)
                .unwrap_or(default)
        };
        let spawn_wait = Duration::from_secs(secs(ENV_SPAWN_WAIT_SECS, 10));
        Ok(Self {
            exe: std::env::current_exe().map_err(IpcError::Io)?,
            build_id: build_id()?,
            run_dir: private_run_dir(&base)?,
            timeouts,
            lock_wait: Duration::from_secs(secs(ENV_LOCK_WAIT_SECS, DEFAULT_LOCK_WAIT_SECS)),
            spawn_wait,
            child_env: Vec::new(),
        })
    }
}

/// Start a detached server for `root`. The returned flag is set by a reaper thread once the child
/// exits, so the spawner never leaves a zombie and can stop waiting for a server that died.
#[cfg(unix)]
fn spawn_server(
    cfg: &ClientConfig,
    root: &Path,
    paths: &Paths,
    build_id: &str,
) -> Result<std::sync::Arc<std::sync::atomic::AtomicBool>, IpcError> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    let log = open_log(&paths.log).map_err(IpcError::Io)?;
    let log_err = log.try_clone().map_err(IpcError::Io)?;
    let mut cmd = Command::new(&cfg.exe);
    cmd.args(["serve", "--mcp"])
        .arg(root)
        .current_dir(root)
        .env(ENV_ROLE, "server")
        .env(ENV_SOCKET, &paths.sock)
        .env(ENV_BUILD_ID, build_id)
        .envs(cfg.child_env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_err);
    // SAFETY: the closure runs between fork and exec and calls only `setsid`, which is
    // async-signal-safe; it allocates nothing and takes no locks.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = cmd.spawn().map_err(IpcError::Io)?;
    let exited = std::sync::Arc::new(AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&exited);
    std::thread::spawn(move || {
        let _ = child.wait();
        flag.store(true, Ordering::SeqCst);
    });
    Ok(exited)
}

/// Find the server for `repo`, spawning it when absent. Every decision to probe, spawn or unlink
/// happens under the per-server lock, so exactly one server exists.
#[cfg(unix)]
pub fn ensure_server(
    repo: &Path,
    cfg: &ClientConfig,
) -> Result<Box<dyn SharedIndexConnection>, IpcError> {
    use std::sync::atomic::Ordering;
    let root = repo.canonicalize().map_err(IpcError::Io)?;
    let hello = Handshake::local_with(&root, cfg.build_id.clone());
    let paths = paths_for(&cfg.run_dir, &root, &hello.build_id)?;
    let transport = UnixTransport {
        path: paths.sock.clone(),
    };
    let _lock = lock_exclusive(&paths.lock, cfg.lock_wait)?;
    match transport.connect(&hello, &cfg.timeouts) {
        Ok(conn) => return Ok(conn),
        Err(IpcError::Io(e)) if is_absent(&e) => {}
        Err(e) => return Err(e),
    }
    match std::fs::remove_file(&paths.sock) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(IpcError::Io(e)),
    }
    let exited = spawn_server(cfg, &root, &paths, &hello.build_id)?;
    let deadline = std::time::Instant::now() + cfg.spawn_wait;
    loop {
        match transport.connect(&hello, &cfg.timeouts) {
            Ok(conn) => return Ok(conn),
            Err(IpcError::Io(e)) if is_absent(&e) => {
                if exited.load(Ordering::SeqCst) || std::time::Instant::now() >= deadline {
                    return Err(IpcError::Unavailable(
                        "spawned server did not come up".into(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Connection to the shared server for a new MCP session, or `None` to stay in-process (opt-out,
/// unsupported platform, or any failure). One retry covers a server that exited between probe and
/// accept.
pub fn connect_for_session(repo: &Path) -> Option<Box<dyn SharedIndexConnection>> {
    if !enabled_with(|k| std::env::var(k).ok()) {
        return None;
    }
    connect_for_session_enabled(repo)
}

#[cfg(unix)]
fn connect_for_session_enabled(repo: &Path) -> Option<Box<dyn SharedIndexConnection>> {
    let cfg = match ClientConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cxpak: shared index server unavailable ({e}); indexing in-process");
            return None;
        }
    };
    for attempt in 0..2 {
        match ensure_server(repo, &cfg) {
            Ok(conn) => {
                eprintln!("cxpak: attached to the shared index server");
                return Some(conn);
            }
            Err(e) if attempt == 0 && e.is_disconnect() => {}
            Err(e) => {
                eprintln!("cxpak: shared index server unavailable ({e}); indexing in-process");
                return None;
            }
        }
    }
    None
}

#[cfg(not(unix))]
fn connect_for_session_enabled(_repo: &Path) -> Option<Box<dyn SharedIndexConnection>> {
    None
}

/// The server role requested through the internal env vars, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRole {
    pub sock: PathBuf,
    pub build_id: String,
}

pub fn server_role_with(get: impl Fn(&str) -> Option<String>) -> Option<ServerRole> {
    (get(ENV_ROLE).as_deref() == Some("server")).then_some(())?;
    Some(ServerRole {
        sock: PathBuf::from(get(ENV_SOCKET)?),
        build_id: get(ENV_BUILD_ID)?,
    })
}

pub fn server_role_from_env() -> Option<ServerRole> {
    server_role_with(|k| std::env::var(k).ok())
}

/// Tunables of the server loop.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub idle: Duration,
    pub max_clients: usize,
    pub failed_grace: Duration,
    pub poll: Duration,
}

impl ServerConfig {
    pub fn with_idle(idle: Duration) -> Self {
        Self {
            idle,
            max_clients: 64,
            failed_grace: Duration::from_secs(2),
            poll: Duration::from_millis(20),
        }
    }

    pub fn from_env() -> Self {
        let secs = std::env::var(ENV_IDLE_SECS)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|s| *s > 0)
            .unwrap_or(DEFAULT_IDLE_SECS);
        Self::with_idle(Duration::from_secs(secs))
    }
}

/// Run the server until idle or failed. `Ok(())` also covers losing the bind race.
#[cfg(unix)]
pub fn run_index_server(
    repo: &Path,
    role: &ServerRole,
    cfg: &ServerConfig,
) -> Result<(), IpcError> {
    use crate::commands::serve::{
        dispatch_tools_call, spawn_mcp_index_build, spawn_mcp_watcher, IndexReadiness,
        SharedReadiness, SharedSnapshot,
    };
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, RwLock};
    use std::time::Instant;

    let own_build = build_id()?;
    if own_build != role.build_id {
        return Err(IpcError::Rejected(format!(
            "binary changed since spawn (expected build {}, running {own_build})",
            role.build_id
        )));
    }
    let root = repo.canonicalize().map_err(IpcError::Io)?;
    let paths = Paths::from_sock(&role.sock);
    let run_dir = role
        .sock
        .parent()
        .ok_or_else(|| IpcError::Unavailable("socket path has no parent".into()))?;
    validate_run_dir(run_dir)?;
    // SAFETY: `umask` takes and returns a plain mode, cannot fail, and touches no memory. The
    // server is still single-threaded here, so no other thread creates files under the mask.
    let previous_mask = unsafe { libc::umask(0o077) };
    let bound = UnixListener::bind(&role.sock);
    // SAFETY: as above; restores the mask saved just before.
    unsafe { libc::umask(previous_mask) };
    let listener = match bound {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => return Ok(()),
        Err(e) => return Err(IpcError::Io(e)),
    };
    std::fs::set_permissions(&role.sock, std::fs::Permissions::from_mode(0o600))
        .map_err(IpcError::Io)?;
    let ours = file_identity(&role.sock).map_err(IpcError::Io)?;
    listener.set_nonblocking(true).map_err(IpcError::Io)?;
    std::fs::write(&paths.pid, std::process::id().to_string()).map_err(IpcError::Io)?;
    eprintln!(
        "cxpak index-server: started pid={} sock={}",
        std::process::id(),
        role.sock.display()
    );

    let expected = Handshake::local(&root)?;
    let readiness: SharedReadiness = Arc::new(RwLock::new(IndexReadiness::Building));
    let shutdown = Arc::new(AtomicBool::new(false));
    let _build = spawn_mcp_index_build(&root, Arc::clone(&readiness));
    let watcher = spawn_mcp_watcher(&root, Arc::clone(&readiness), Arc::clone(&shutdown));

    let clients = Arc::new(AtomicUsize::new(0));
    let idle_since = Arc::new(Mutex::new(Instant::now()));
    let mut failed_since: Option<Instant> = None;

    struct ClientGuard {
        clients: Arc<AtomicUsize>,
        idle_since: Arc<Mutex<Instant>>,
    }
    impl Drop for ClientGuard {
        fn drop(&mut self) {
            self.clients.fetch_sub(1, Ordering::SeqCst);
            *self.idle_since.lock().unwrap_or_else(|p| p.into_inner()) = Instant::now();
        }
    }

    // Over-cap connections are dropped here, with no thread. Returns whether one was accepted.
    let accept_one = || -> std::io::Result<bool> {
        let (stream, _) = match listener.accept() {
            Ok(accepted) => accepted,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) => return Err(e),
        };
        if clients.fetch_add(1, Ordering::SeqCst) >= cfg.max_clients {
            clients.fetch_sub(1, Ordering::SeqCst);
            return Ok(true);
        }
        let guard = ClientGuard {
            clients: Arc::clone(&clients),
            idle_since: Arc::clone(&idle_since),
        };
        let expected = expected.clone();
        let readiness = Arc::clone(&readiness);
        let root = root.clone();
        let mut stream = stream;
        let spawned = std::thread::Builder::new()
            .name("cxpak-index-conn".into())
            .spawn(move || {
                let _guard = guard;
                let timeout = Some(Duration::from_secs(2));
                let ready = stream.set_nonblocking(false).is_ok()
                    && stream.set_read_timeout(timeout).is_ok()
                    && stream
                        .set_write_timeout(Some(Duration::from_secs(120)))
                        .is_ok();
                if !ready {
                    return;
                }
                if server_handshake(&mut stream, &expected, Ok(())).is_err() {
                    return;
                }
                if stream.set_read_timeout(None).is_err() {
                    return;
                }
                let snapshot: SharedSnapshot = Arc::new(RwLock::new(None));
                let result = serve_requests(&mut stream, &mut |request| {
                    dispatch_tools_call(request, &readiness, &root, &snapshot)
                });
                if let Err(e) = result {
                    eprintln!("cxpak index-server: connection ended: {e}");
                }
            });
        if let Err(e) = spawned {
            // The closure, and with it the guard, was dropped: the count is already restored.
            eprintln!("cxpak index-server: cannot start a connection thread: {e}");
        }
        Ok(true)
    };

    loop {
        match accept_one() {
            Ok(true) => {}
            Ok(false) => std::thread::sleep(cfg.poll),
            Err(e) => {
                eprintln!("cxpak index-server: accept failed: {e}");
                std::thread::sleep(cfg.poll);
            }
        }

        let is_failed = matches!(
            &*readiness.read().unwrap_or_else(|p| p.into_inner()),
            IndexReadiness::Failed(_)
        );
        if is_failed && failed_since.is_none() {
            failed_since = Some(Instant::now());
        }
        let failed_due = failed_since.is_some_and(|t| t.elapsed() >= cfg.failed_grace);
        let idle_due = clients.load(Ordering::SeqCst) == 0
            && idle_since
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .elapsed()
                >= cfg.idle;
        if !(failed_due || idle_due) {
            continue;
        }
        // The decision is re-checked under the lock: a client that connected after the idle
        // decision keeps the server alive, and clients cannot probe while we unlink. A probing
        // client holds the lock while it waits to be accepted, so contention must never block the
        // loop: keep accepting and retry on the next pass.
        let _lock = match lock_exclusive(&paths.lock, Duration::ZERO) {
            Ok(lock) => lock,
            Err(IpcError::Timeout) => continue,
            Err(e) => {
                eprintln!("cxpak index-server: cannot take the lock: {e}");
                std::thread::sleep(cfg.poll);
                continue;
            }
        };
        // Clients that connected before we took the lock are in the backlog; admit them first.
        while matches!(accept_one(), Ok(true)) {}
        if !failed_due && clients.load(Ordering::SeqCst) != 0 {
            continue;
        }
        if let Err(e) = unlink_if_ours(&role.sock, ours) {
            eprintln!("cxpak index-server: cannot remove socket: {e}");
        }
        if std::fs::read_to_string(&paths.pid)
            .is_ok_and(|p| p.trim() == std::process::id().to_string())
        {
            let _ = std::fs::remove_file(&paths.pid);
        }
        break;
    }
    shutdown.store(true, Ordering::Relaxed);
    let _ = watcher.join();
    eprintln!("cxpak index-server: exiting pid={}", std::process::id());
    Ok(())
}

/// Servers need a Unix socket; elsewhere the role is refused and sessions stay in-process.
#[cfg(not(unix))]
pub fn run_index_server(
    _repo: &Path,
    _role: &ServerRole,
    _cfg: &ServerConfig,
) -> Result<(), IpcError> {
    Err(IpcError::Unavailable(
        "no shared index transport on this platform".into(),
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn get_none(_: &str) -> Option<std::ffi::OsString> {
        None
    }

    #[test]
    fn switch_values() {
        assert!(enabled_with(|_| None));
        for off in ["off", "OFF", "0", "false", "No"] {
            assert!(!enabled_with(|_| Some(off.to_string())), "{off}");
        }
        assert!(enabled_with(|_| Some("on".to_string())));
    }

    #[test]
    fn paths_depend_on_root_and_build_id_and_fit_sun_path() {
        let dir = tempfile::tempdir().unwrap();
        let a = paths_for(dir.path(), Path::new("/r/a"), "b1").unwrap();
        assert_eq!(a, paths_for(dir.path(), Path::new("/r/a"), "b1").unwrap());
        assert_ne!(
            a.sock,
            paths_for(dir.path(), Path::new("/r/b"), "b1").unwrap().sock
        );
        assert_ne!(
            a.sock,
            paths_for(dir.path(), Path::new("/r/a"), "b2").unwrap().sock
        );
        assert_eq!(a.lock.extension().unwrap(), "lock");
        assert_eq!(Paths::from_sock(&a.sock), a);
        let long = dir.path().join("x".repeat(120));
        assert!(matches!(
            paths_for(&long, Path::new("/r/a"), "b1"),
            Err(IpcError::Unavailable(_))
        ));
    }

    #[test]
    fn run_base_prefers_xdg_then_platform_default() {
        let xdg = |k: &str| (k == "XDG_RUNTIME_DIR").then(|| "/run/u".into());
        assert_eq!(run_base(&xdg), Some(PathBuf::from("/run/u")));
        let rel = |k: &str| (k == "XDG_RUNTIME_DIR").then(|| "relative".into());
        assert_ne!(run_base(&rel), Some(PathBuf::from("relative")));
        let none = run_base(&get_none);
        if cfg!(target_os = "macos") {
            assert_eq!(none, Some(std::env::temp_dir()));
        } else {
            assert_eq!(none, None);
        }
    }

    #[test]
    fn run_dir_is_created_private_and_revalidated() {
        let base = tempfile::tempdir().unwrap();
        let dir = private_run_dir(base.path()).unwrap();
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(private_run_dir(base.path()).unwrap(), dir);
    }

    #[test]
    fn run_dir_rejects_writable_and_symlink() {
        let base = tempfile::tempdir().unwrap();
        let dir = private_run_dir(base.path()).unwrap();
        for bad in [0o770, 0o707, 0o777] {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(bad)).unwrap();
            assert!(
                matches!(validate_run_dir(&dir), Err(IpcError::Unavailable(_))),
                "{bad:o}"
            );
        }
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir(&dir).unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), &dir).unwrap();
        assert!(matches!(
            private_run_dir(base.path()),
            Err(IpcError::Unavailable(_))
        ));
    }

    #[test]
    fn run_dir_rejects_group_and_other_readable() {
        let base = tempfile::tempdir().unwrap();
        let dir = private_run_dir(base.path()).unwrap();
        for bad in [0o750, 0o740, 0o705, 0o701, 0o755] {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(bad)).unwrap();
            assert!(
                matches!(validate_run_dir(&dir), Err(IpcError::Unavailable(_))),
                "{bad:o}"
            );
        }
    }

    #[test]
    fn log_never_follows_a_symlink_and_appends() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim");
        std::fs::write(&victim, "precious").unwrap();
        let log = dir.path().join("s.log");
        std::os::unix::fs::symlink(&victim, &log).unwrap();
        assert!(open_log(&log).is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");

        std::fs::remove_file(&log).unwrap();
        std::fs::write(&log, "old\n").unwrap();
        {
            use std::io::Write;
            writeln!(open_log(&log).unwrap(), "new").unwrap();
        }
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "old\nnew\n");
    }

    #[test]
    fn log_rotates_by_rename_when_large() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("s.log");
        std::fs::write(&log, vec![b'x'; (LOG_ROTATE_BYTES + 1) as usize]).unwrap();
        drop(open_log(&log).unwrap());
        assert_eq!(std::fs::metadata(&log).unwrap().len(), 0);
        assert!(dir.path().join("s.log.1").exists());
    }

    #[test]
    fn unlink_only_when_identity_matches() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("s.sock");
        std::fs::write(&sock, "old server").unwrap();
        let old = file_identity(&sock).unwrap();
        // A newer server replaces the path while the old one is still exiting.
        std::fs::remove_file(&sock).unwrap();
        std::fs::write(&sock, "new server").unwrap();
        assert!(!unlink_if_ours(&sock, old).unwrap());
        assert!(sock.exists());
        let new = file_identity(&sock).unwrap();
        assert!(unlink_if_ours(&sock, new).unwrap());
        assert!(!sock.exists());
        assert!(!unlink_if_ours(&sock, new).unwrap());
    }

    #[test]
    fn lock_is_exclusive_and_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("l.lock");
        let held = lock_exclusive(&lock, Duration::from_millis(200)).unwrap();
        let start = std::time::Instant::now();
        assert!(matches!(
            lock_exclusive(&lock, Duration::from_millis(150)),
            Err(IpcError::Timeout)
        ));
        assert!(start.elapsed() < Duration::from_secs(2));
        drop(held);
        lock_exclusive(&lock, Duration::from_millis(200)).unwrap();
    }

    #[test]
    fn server_role_needs_all_three_vars() {
        let all = |k: &str| match k {
            ENV_ROLE => Some("server".to_string()),
            ENV_SOCKET => Some("/s.sock".to_string()),
            ENV_BUILD_ID => Some("abc".to_string()),
            _ => None,
        };
        assert_eq!(
            server_role_with(all),
            Some(ServerRole {
                sock: PathBuf::from("/s.sock"),
                build_id: "abc".into()
            })
        );
        assert_eq!(
            server_role_with(|k| (k != ENV_BUILD_ID).then(|| all(k)).flatten()),
            None
        );
        assert_eq!(
            server_role_with(|k| (k != ENV_ROLE).then(|| all(k)).flatten()),
            None
        );
    }
}
