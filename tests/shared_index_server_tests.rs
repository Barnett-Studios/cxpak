//! Shared per-repo index server (ADR-0214) driven through the real `cxpak` binary.
//!
//! Every test runs in a sandbox whose `XDG_RUNTIME_DIR`, `TMPDIR` and `HOME` are private to the
//! test, and a drop guard SIGKILLs every server whose pid file appears there, even on panic.
#![cfg(all(unix, feature = "daemon"))]

use cxpak::daemon::index_server::{
    ensure_server, paths_for, private_run_dir, run_index_server, ClientConfig, ServerConfig,
    ServerRole,
};
use cxpak::daemon::ipc::{build_id_of, read_frame, write_frame, IpcError, Timeouts};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_cxpak");

/// Server spawns have a bounded startup wait; running these tests one at a time keeps a loaded
/// machine from turning that bound into flakes.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Sandbox {
    dir: tempfile::TempDir,
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl Sandbox {
    fn new() -> Self {
        let serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        // Short path: the socket path must stay under the 104-byte `sun_path` limit.
        let dir = tempfile::Builder::new()
            .prefix("cxs")
            .tempdir_in("/tmp")
            .unwrap();
        for sub in ["run", "home", "repo"] {
            std::fs::create_dir(dir.path().join(sub)).unwrap();
        }
        let sb = Sandbox {
            dir,
            _serial: serial,
        };
        sb.init_repo();
        sb
    }

    fn run(&self) -> PathBuf {
        self.dir.path().join("run")
    }

    fn repo(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    fn run_dir(&self) -> PathBuf {
        private_run_dir(&self.run()).unwrap()
    }

    fn init_repo(&self) {
        let repo = self.repo();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/a.rs"), "pub fn helper() {}\n").unwrap();
        std::fs::write(
            repo.join("src/b.rs"),
            "use crate::a::helper;\npub fn go() { helper(); }\n",
        )
        .unwrap();
        let git = git2::Repository::init(&repo).unwrap();
        let mut index = git.index().unwrap();
        for rel in ["src/a.rs", "src/b.rs"] {
            index.add_path(Path::new(rel)).unwrap();
        }
        index.write().unwrap();
        let tree = git.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("t", "t@t").unwrap();
        git.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
    }

    fn command(&self, switch: &str) -> Command {
        let mut cmd = Command::new(BIN);
        cmd.args(["serve", "--mcp"])
            .arg(self.repo())
            .env("XDG_RUNTIME_DIR", self.run())
            .env("TMPDIR", self.dir.path())
            .env("HOME", self.dir.path().join("home"))
            .env("CXPAK_INDEX_SERVER", switch)
            .env("CXPAK_INDEX_SERVER_IDLE_SECS", "60");
        cmd
    }

    fn server_pids(&self) -> Vec<i32> {
        let Ok(rd) = std::fs::read_dir(self.run().join(format!("cxpak-{}", uid()))) else {
            return Vec::new();
        };
        rd.flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "pid"))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok()?.trim().parse().ok())
            .collect()
    }

    fn log_text(&self) -> String {
        let Ok(rd) = std::fs::read_dir(self.run().join(format!("cxpak-{}", uid()))) else {
            return String::new();
        };
        rd.flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "log"))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .collect()
    }

    fn servers_started(&self) -> usize {
        self.log_text().matches("index-server: started").count()
    }

    fn client_config(&self, idle_secs: &str) -> ClientConfig {
        let mut cfg = ClientConfig {
            exe: PathBuf::from(BIN),
            build_id: build_id_of(Path::new(BIN)).unwrap(),
            run_dir: self.run_dir(),
            timeouts: Timeouts::default(),
            lock_wait: Duration::from_secs(10),
            spawn_wait: Duration::from_secs(15),
            child_env: Vec::new(),
        };
        cfg.child_env
            .push(("CXPAK_INDEX_SERVER_IDLE_SECS".into(), idle_secs.into()));
        cfg
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        for pid in self.server_pids() {
            // SAFETY: `kill` with a pid read from our own sandbox's pid file; no memory is touched.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

fn uid() -> u32 {
    // SAFETY: `geteuid` takes no arguments and cannot fail.
    unsafe { libc::geteuid() }
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only probes for existence.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// A real `serve --mcp` stdio session.
struct Session {
    child: Child,
    stdin: std::process::ChildStdin,
    rx: mpsc::Receiver<Value>,
    stderr_path: PathBuf,
    next_id: u64,
}

impl Session {
    fn start(mut cmd: Command, sb: &Sandbox) -> Session {
        let stderr_path = sb.dir.path().join(format!(
            "stderr-{}.txt",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(&stderr_path).unwrap())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if tx.send(v).is_err() {
                        break;
                    }
                }
            }
        });
        Session {
            child,
            stdin,
            rx,
            stderr_path,
            next_id: 1,
        }
    }

    fn send(&mut self, request: &Value) -> Option<Value> {
        let mut line = request.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).unwrap();
        self.stdin.flush().unwrap();
        self.rx.recv_timeout(Duration::from_secs(30)).ok()
    }

    fn call_node(&mut self) -> Value {
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0", "id": self.next_id, "method": "tools/call",
            "params": {"name": "cxpak_graph",
                       "arguments": {"op": "graph", "graph_op": "node", "id": "src/a.rs"}}
        });
        self.send(&request).expect("tools/call must answer in time")
    }

    /// Poll until the call is answered by a ready index; returns that response.
    fn ready_response(&mut self) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            let resp = self.call_node();
            if resp["result"]["structuredContent"]["status"].is_null() {
                return resp;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("index never became ready; stderr:\n{}", self.stderr());
    }

    fn stderr(&self) -> String {
        std::fs::read_to_string(&self.stderr_path).unwrap_or_default()
    }

    /// The same response with the request id normalised, for cross-session comparison.
    fn text_of(resp: &Value) -> String {
        resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn baseline(sb: &Sandbox) -> String {
    let mut s = Session::start(sb.command("off"), sb);
    Session::text_of(&s.ready_response())
}

#[test]
fn shared_response_is_byte_identical_to_in_process() {
    let sb = Sandbox::new();
    let expected = baseline(&sb);
    assert!(sb.server_pids().is_empty(), "opt-out must not spawn");

    let mut shared = Session::start(sb.command("on"), &sb);
    let got = Session::text_of(&shared.ready_response());
    assert_eq!(got, expected);
    assert!(shared
        .stderr()
        .contains("attached to the shared index server"));
    assert_eq!(sb.servers_started(), 1);
}

#[test]
fn opt_out_creates_no_socket_lock_or_child() {
    let sb = Sandbox::new();
    let mut s = Session::start(sb.command("off"), &sb);
    s.ready_response();
    let created = std::fs::read_dir(sb.run())
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with("cxpak-"));
    assert!(
        !created,
        "CXPAK_INDEX_SERVER=off must not touch the run dir"
    );
}

#[test]
fn concurrent_clients_converge_on_exactly_one_server() {
    let sb = Sandbox::new();
    let cfg = sb.client_config("60");
    let repo = sb.repo();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let cfg = cfg.clone();
            let repo = repo.clone();
            std::thread::spawn(move || ensure_server(&repo, &cfg).map(|_| ()))
        })
        .collect();
    for h in handles {
        h.join().unwrap().unwrap();
    }
    assert_eq!(sb.servers_started(), 1, "log:\n{}", sb.log_text());
    assert_eq!(sb.server_pids().len(), 1);
}

#[test]
fn concurrent_sessions_share_one_server() {
    let sb = Sandbox::new();
    let expected = baseline(&sb);
    let mut sessions: Vec<_> = (0..4)
        .map(|_| Session::start(sb.command("on"), &sb))
        .collect();
    for s in &mut sessions {
        assert_eq!(Session::text_of(&s.ready_response()), expected);
    }
    assert_eq!(sb.servers_started(), 1, "log:\n{}", sb.log_text());
}

#[test]
fn stale_socket_is_replaced() {
    let sb = Sandbox::new();
    let cfg = sb.client_config("60");
    let root = sb.repo().canonicalize().unwrap();
    let paths = paths_for(&cfg.run_dir, &root, &cfg.build_id).unwrap();
    drop(std::os::unix::net::UnixListener::bind(&paths.sock).unwrap());
    assert!(paths.sock.exists(), "fixture: a dead socket file");
    ensure_server(&sb.repo(), &cfg).unwrap();
    assert_eq!(sb.servers_started(), 1);
}

#[test]
fn server_outlives_the_session_that_spawned_it() {
    let sb = Sandbox::new();
    {
        let mut s = Session::start(sb.command("on"), &sb);
        s.ready_response();
    }
    let pid = *sb.server_pids().first().expect("server pid file");
    std::thread::sleep(Duration::from_millis(500));
    assert!(alive(pid), "server must survive its spawner");
}

#[test]
fn killing_the_server_mid_session_falls_back_without_hanging() {
    let sb = Sandbox::new();
    let mut s = Session::start(sb.command("on"), &sb);
    let before = Session::text_of(&s.ready_response());
    let pid = *sb
        .server_pids()
        .first()
        .unwrap_or_else(|| panic!("stderr:\n{}\nlog:\n{}", s.stderr(), sb.log_text()));
    // SAFETY: SIGKILL to the server pid read from this sandbox's pid file.
    unsafe { libc::kill(pid, libc::SIGKILL) };
    let start = Instant::now();
    let first_after = s.call_node();
    assert!(start.elapsed() < Duration::from_secs(10), "client hung");
    assert_eq!(
        first_after["result"]["structuredContent"]["status"],
        "indexing"
    );
    assert_eq!(Session::text_of(&s.ready_response()), before);
    assert!(s.stderr().contains("indexing in-process"), "{}", s.stderr());
}

/// A listener at the path the client will probe, standing in for a misbehaving server.
fn fake_server(sb: &Sandbox, reply: impl Fn(&mut std::os::unix::net::UnixStream) + Send + 'static) {
    let root = sb.repo().canonicalize().unwrap();
    let paths = paths_for(&sb.run_dir(), &root, &build_id_of(Path::new(BIN)).unwrap()).unwrap();
    let listener = std::os::unix::net::UnixListener::bind(&paths.sock).unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            reply(&mut stream);
        }
    });
}

#[test]
fn rejecting_server_means_fallback_with_correct_output() {
    let sb = Sandbox::new();
    let expected = baseline(&sb);
    fake_server(&sb, |stream| {
        let _ = read_frame(stream);
        let _ = write_frame(stream, br#"{"ok":false,"reason":"version mismatch"}"#);
    });
    let mut s = Session::start(sb.command("on"), &sb);
    assert_eq!(Session::text_of(&s.ready_response()), expected);
    assert!(s.stderr().contains("indexing in-process"), "{}", s.stderr());
    assert_eq!(
        sb.servers_started(),
        0,
        "a rejection must not spawn another server"
    );
}

#[test]
fn hung_server_times_out_and_falls_back() {
    let sb = Sandbox::new();
    let expected = baseline(&sb);
    fake_server(&sb, |stream| {
        let _ = read_frame(stream);
        let _ = write_frame(stream, br#"{"ok":true}"#);
        let _ = read_frame(stream);
        std::thread::sleep(Duration::from_secs(30));
    });
    let mut cmd = sb.command("on");
    cmd.env("CXPAK_INDEX_SERVER_REQ_TIMEOUT_SECS", "1");
    let mut s = Session::start(cmd, &sb);
    let start = Instant::now();
    let first = s.call_node();
    assert!(start.elapsed() < Duration::from_secs(10), "client hung");
    assert_eq!(first["result"]["structuredContent"]["status"], "indexing");
    assert_eq!(Session::text_of(&s.ready_response()), expected);
}

#[test]
fn environment_mismatch_falls_back() {
    let sb = Sandbox::new();
    let mut first = sb.command("on");
    first.env("CXPAK_MAX_FILE_BYTES", "1000000");
    let mut a = Session::start(first, &sb);
    a.ready_response();

    let mut second = sb.command("on");
    second.env("CXPAK_MAX_FILE_BYTES", "2000000");
    let mut b = Session::start(second, &sb);
    b.ready_response();
    assert!(b.stderr().contains("env_fingerprint"), "{}", b.stderr());
    assert!(b.stderr().contains("indexing in-process"));
}

#[test]
fn idle_server_exits_unlinks_and_leaves_no_zombie() {
    let sb = Sandbox::new();
    let cfg = sb.client_config("1");
    let conn = ensure_server(&sb.repo(), &cfg).unwrap();
    let pid = *sb.server_pids().first().expect("pid file");
    drop(conn);
    let deadline = Instant::now() + Duration::from_secs(20);
    while alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        !alive(pid),
        "idle server must exit and be reaped; log:\n{}",
        sb.log_text()
    );
    let socks = std::fs::read_dir(sb.run_dir())
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "sock"))
        .count();
    assert_eq!(socks, 0, "socket must be unlinked");
    // A fresh client spawns a new server after the idle exit.
    ensure_server(&sb.repo(), &cfg).unwrap();
    assert_eq!(sb.servers_started(), 2);
}

#[test]
fn child_refuses_when_the_binary_changed_since_spawn() {
    let sb = Sandbox::new();
    let role = ServerRole {
        sock: sb.run_dir().join("x.sock"),
        build_id: "not-this-binary".into(),
    };
    let err = run_index_server(
        &sb.repo(),
        &role,
        &ServerConfig::with_idle(Duration::from_secs(1)),
    )
    .unwrap_err();
    assert!(matches!(err, IpcError::Rejected(_)), "{err}");
    assert!(!role.sock.exists());
}

#[test]
fn failed_index_makes_the_server_exit_and_unlink() {
    let sb = Sandbox::new();
    let file = sb.dir.path().join("not-a-repo.txt");
    std::fs::write(&file, "x").unwrap();
    let role = ServerRole {
        sock: sb.run_dir().join("f.sock"),
        build_id: cxpak::daemon::ipc::build_id().unwrap(),
    };
    let mut cfg = ServerConfig::with_idle(Duration::from_secs(600));
    cfg.failed_grace = Duration::from_millis(200);
    let sock = role.sock.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(run_index_server(&file, &role, &cfg));
    });
    let result = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("a Failed server must exit, not linger");
    result.unwrap();
    assert!(!sock.exists());
}
