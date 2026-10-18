//! Wire protocol and transport for the shared per-repo index server (ADR-0214).
//!
//! Frames are a `u32` big-endian length followed by that many bytes of UTF-8 JSON. The first client
//! frame is a [`Handshake`]; the server answers a [`HandshakeReply`]. After that each client frame
//! is a complete JSON-RPC `tools/call` request and each server frame the complete JSON-RPC
//! response, so the client can write the reply to stdout unchanged.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

/// Bump when the frame or handshake layout changes. Mixed into [`wire_schema_hash`].
pub const WIRE_SCHEMA: &str = "2";

/// Largest accepted frame. A longer claimed length is rejected before any allocation.
pub const MAX_FRAME_BYTES: u64 = 64 * 1024 * 1024;

/// Environment variables that change index or tool output. Their values are hashed into the
/// handshake so a server built under a different environment is never silently reused.
///
/// `HOME`/`USERPROFILE` locate the embedding model cache. The variable named by a repo's configured
/// `api_key_env` is not listed here: its name comes from `.cxpak.json`, see [`env_fingerprint_for`].
pub const OUTPUT_ENV_VARS: &[&str] = &[
    "CXPAK_MAX_FILE_BYTES",
    "CXPAK_MYSQL_DSN",
    "CXPAK_PG_DSN",
    "HOME",
    "USERPROFILE",
];

/// Failure of the shared-server transport. Every variant means "fall back to the in-process index".
#[derive(Debug)]
pub enum IpcError {
    Io(std::io::Error),
    /// A bounded connect, handshake, or request wait elapsed.
    Timeout,
    /// The peer closed the connection cleanly before a frame started.
    Closed,
    FrameTooLarge(u64),
    Protocol(String),
    /// The server answered the handshake with `ok: false`.
    Rejected(String),
    /// No shared server can be used here (opt-out, unsafe run dir, unsupported platform).
    Unavailable(String),
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcError::Io(e) => write!(f, "io error: {e}"),
            IpcError::Timeout => f.write_str("timed out"),
            IpcError::Closed => f.write_str("connection closed"),
            IpcError::FrameTooLarge(n) => write!(f, "frame of {n} bytes exceeds the limit"),
            IpcError::Protocol(m) => write!(f, "protocol error: {m}"),
            IpcError::Rejected(m) => write!(f, "server rejected the handshake: {m}"),
            IpcError::Unavailable(m) => write!(f, "unavailable: {m}"),
        }
    }
}

impl std::error::Error for IpcError {}

impl From<std::io::Error> for IpcError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => IpcError::Timeout,
            _ => IpcError::Io(e),
        }
    }
}

impl IpcError {
    /// The server vanished mid-exchange (idle shutdown between probe and accept). One retry of
    /// server discovery is worthwhile; any other error is not.
    pub fn is_disconnect(&self) -> bool {
        use std::io::ErrorKind::{BrokenPipe, ConnectionReset, UnexpectedEof};
        match self {
            IpcError::Closed => true,
            IpcError::Io(e) => matches!(e.kind(), UnexpectedEof | ConnectionReset | BrokenPipe),
            _ => false,
        }
    }
}

/// Write one frame.
pub fn write_frame(w: &mut impl Write, payload: &[u8]) -> Result<(), IpcError> {
    let len = payload.len() as u64;
    if len > MAX_FRAME_BYTES {
        return Err(IpcError::FrameTooLarge(len));
    }
    w.write_all(&(len as u32).to_be_bytes())?;
    w.write_all(payload)?;
    w.flush()?;
    Ok(())
}

/// Read one frame. A clean EOF before the length prefix is [`IpcError::Closed`].
pub fn read_frame(r: &mut impl Read) -> Result<Vec<u8>, IpcError> {
    let mut prefix = [0u8; 4];
    let mut got = 0;
    while got < prefix.len() {
        match r.read(&mut prefix[got..]) {
            Ok(0) if got == 0 => return Err(IpcError::Closed),
            Ok(0) => return Err(IpcError::Io(std::io::ErrorKind::UnexpectedEof.into())),
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    let len = u64::from(u32::from_be_bytes(prefix));
    if len > MAX_FRAME_BYTES {
        return Err(IpcError::FrameTooLarge(len));
    }
    let mut body = Vec::new();
    r.take(len).read_to_end(&mut body)?;
    if body.len() as u64 != len {
        return Err(IpcError::Io(std::io::ErrorKind::UnexpectedEof.into()));
    }
    Ok(body)
}

fn hex16(digest: &[u8]) -> String {
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// First 16 hex chars of sha256 over `parts` joined by NUL.
pub fn short_hash(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            h.update([0u8]);
        }
        h.update(p.as_bytes());
    }
    hex16(&h.finalize())
}

/// Content hash of the executable at `path` (streamed). A rebuilt binary changes it even when the
/// version string does not.
pub fn build_id_of(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        match f.read(&mut buf)? {
            0 => break,
            n => h.update(&buf[..n]),
        }
    }
    Ok(hex16(&h.finalize()))
}

/// Build id of the running executable, computed once per process.
pub fn build_id() -> Result<String, IpcError> {
    static ID: OnceLock<Result<String, String>> = OnceLock::new();
    ID.get_or_init(|| {
        std::env::current_exe()
            .and_then(|p| build_id_of(&p))
            .map_err(|e| e.to_string())
    })
    .clone()
    .map_err(|e| IpcError::Unavailable(format!("cannot hash the running executable: {e}")))
}

/// Hash of the wire revision plus the advertised tool schemas: any tool-surface change flips it.
pub fn wire_schema_hash() -> String {
    let schemas =
        serde_json::to_string(&crate::capability::adapter::mcp_tool_schemas()).unwrap_or_default();
    short_hash(&[WIRE_SCHEMA, &schemas])
}

/// Fingerprint of the output-affecting environment, from an injectable lookup. Values are hashed so
/// DSN secrets never cross the socket.
pub fn env_fingerprint_with(get: impl Fn(&str) -> Option<String>) -> String {
    fingerprint_of(&get, None)
}

fn fingerprint_of(get: &dyn Fn(&str) -> Option<String>, api_key_env: Option<&str>) -> String {
    let mut pairs: Vec<String> = OUTPUT_ENV_VARS
        .iter()
        .copied()
        .chain(api_key_env)
        .filter_map(|name| get(name).map(|v| format!("{name}={v}")))
        .collect();
    pairs.sort();
    let refs: Vec<&str> = pairs.iter().map(String::as_str).collect();
    short_hash(&refs)
}

/// [`env_fingerprint_with`] plus the variable the repo's embedding config names as its API key.
pub fn env_fingerprint_for(root: &Path, get: impl Fn(&str) -> Option<String>) -> String {
    let key_env = crate::embeddings::EmbeddingConfig::from_repo_root(root).api_key_env;
    fingerprint_of(&get, Some(key_env.as_str()).filter(|n| !n.is_empty()))
}

/// Hash of the repo's `.cxpak.json` bytes (empty when absent): config changes alter output but
/// the file watcher never rebuilds for them.
pub fn config_hash(root: &Path) -> String {
    match std::fs::read(root.join(".cxpak.json")) {
        Ok(bytes) => short_hash(&[&String::from_utf8_lossy(&bytes)]),
        Err(_) => String::new(),
    }
}

/// Identity a client and a server must agree on before any tool call is forwarded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handshake {
    pub version: String,
    pub build_id: String,
    pub wire_schema_hash: String,
    pub repo_root: String,
    pub env_fingerprint: String,
    pub config_hash: String,
}

impl Handshake {
    /// This process's identity for `canonical_root`.
    pub fn local(canonical_root: &Path) -> Result<Self, IpcError> {
        Ok(Self::local_with(canonical_root, build_id()?))
    }

    /// Identity with an explicit build id (a client addressing a server binary other than itself).
    pub fn local_with(canonical_root: &Path, build_id: String) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            build_id,
            wire_schema_hash: wire_schema_hash(),
            repo_root: canonical_root.to_string_lossy().into_owned(),
            env_fingerprint: env_fingerprint_for(canonical_root, |k| std::env::var(k).ok()),
            config_hash: config_hash(canonical_root),
        }
    }

    /// Name of the first differing field, or `None` when identical.
    pub fn mismatch(&self, other: &Handshake) -> Option<&'static str> {
        if self.version != other.version {
            Some("version")
        } else if self.build_id != other.build_id {
            Some("build_id")
        } else if self.wire_schema_hash != other.wire_schema_hash {
            Some("wire_schema_hash")
        } else if self.repo_root != other.repo_root {
            Some("repo_root")
        } else if self.env_fingerprint != other.env_fingerprint {
            Some("env_fingerprint")
        } else if self.config_hash != other.config_hash {
            Some("config_hash")
        } else {
            None
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HandshakeReply {
    pub ok: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Bounds on every wait the client performs.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub connect: Duration,
    pub handshake: Duration,
    pub request: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(2),
            handshake: Duration::from_secs(2),
            request: Duration::from_secs(120),
        }
    }
}

/// An attached session: one request frame in, one response frame out.
pub trait SharedIndexConnection: Send {
    /// Forward a JSON-RPC `tools/call` request and return the server's JSON-RPC response.
    fn call(&mut self, request: &Value) -> Result<Value, IpcError>;
}

/// A way of reaching a shared index server. The Unix socket arm is the only one built today; the
/// trait keeps the call sites free of Unix types so a Windows named-pipe arm can be added later.
pub trait SharedIndexTransport {
    fn connect(
        &self,
        hello: &Handshake,
        timeouts: &Timeouts,
    ) -> Result<Box<dyn SharedIndexConnection>, IpcError>;
}

/// Request/response over any byte stream.
pub struct StreamConnection<S: Read + Write + Send> {
    stream: S,
}

impl<S: Read + Write + Send> StreamConnection<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }
}

impl<S: Read + Write + Send> SharedIndexConnection for StreamConnection<S> {
    fn call(&mut self, request: &Value) -> Result<Value, IpcError> {
        let body = serde_json::to_vec(request).map_err(|e| IpcError::Protocol(e.to_string()))?;
        write_frame(&mut self.stream, &body)?;
        let reply = read_frame(&mut self.stream)?;
        serde_json::from_slice(&reply).map_err(|e| IpcError::Protocol(e.to_string()))
    }
}

/// Client half of the handshake over an already connected stream.
pub fn client_handshake(
    stream: &mut (impl Read + Write),
    hello: &Handshake,
) -> Result<(), IpcError> {
    let body = serde_json::to_vec(hello).map_err(|e| IpcError::Protocol(e.to_string()))?;
    write_frame(stream, &body)?;
    let reply: HandshakeReply = serde_json::from_slice(&read_frame(stream)?)
        .map_err(|e| IpcError::Protocol(format!("bad handshake reply: {e}")))?;
    if reply.ok {
        Ok(())
    } else {
        Err(IpcError::Rejected(reply.reason.unwrap_or_default()))
    }
}

/// Server half: read the client handshake, check it against `expected` and `admit`, reply.
pub fn server_handshake(
    stream: &mut (impl Read + Write),
    expected: &Handshake,
    admit: Result<(), &str>,
) -> Result<(), IpcError> {
    let theirs: Result<Handshake, _> = serde_json::from_slice(&read_frame(stream)?);
    let verdict: Result<(), String> = match (theirs, admit) {
        (Err(e), _) => Err(format!("unparseable handshake: {e}")),
        (Ok(_), Err(why)) => Err(why.to_string()),
        (Ok(t), Ok(())) => match expected.mismatch(&t) {
            Some(field) => Err(format!("{field} mismatch")),
            None => Ok(()),
        },
    };
    let reply = HandshakeReply {
        ok: verdict.is_ok(),
        reason: verdict.as_ref().err().cloned(),
    };
    let body = serde_json::to_vec(&reply).map_err(|e| IpcError::Protocol(e.to_string()))?;
    write_frame(stream, &body)?;
    verdict.map_err(IpcError::Rejected)
}

/// Serve one admitted connection: every request frame is answered by `respond`. Returns `Ok(())`
/// when the client closes cleanly.
pub fn serve_requests(
    stream: &mut (impl Read + Write),
    respond: &mut dyn FnMut(&Value) -> Value,
) -> Result<(), IpcError> {
    loop {
        let frame = match read_frame(stream) {
            Ok(f) => f,
            Err(IpcError::Closed) => return Ok(()),
            Err(e) => return Err(e),
        };
        let request: Value = serde_json::from_slice(&frame)
            .map_err(|e| IpcError::Protocol(format!("bad request frame: {e}")))?;
        let body = serde_json::to_vec(&respond(&request))
            .map_err(|e| IpcError::Protocol(e.to_string()))?;
        write_frame(stream, &body)?;
    }
}

/// Unix domain socket arm.
#[cfg(unix)]
pub struct UnixTransport {
    pub path: std::path::PathBuf,
}

#[cfg(unix)]
impl SharedIndexTransport for UnixTransport {
    fn connect(
        &self,
        hello: &Handshake,
        timeouts: &Timeouts,
    ) -> Result<Box<dyn SharedIndexConnection>, IpcError> {
        use std::os::unix::net::UnixStream;
        // `connect` on a full accept backlog blocks with no timeout API, so it runs on a helper
        // thread the caller abandons on expiry.
        let (tx, rx) = std::sync::mpsc::channel();
        let path = self.path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(UnixStream::connect(path));
        });
        let mut stream = match rx.recv_timeout(timeouts.connect) {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(IpcError::Io(e)),
            Err(_) => return Err(IpcError::Timeout),
        };
        stream.set_read_timeout(Some(timeouts.handshake))?;
        stream.set_write_timeout(Some(timeouts.handshake))?;
        client_handshake(&mut stream, hello)?;
        stream.set_read_timeout(Some(timeouts.request))?;
        stream.set_write_timeout(Some(timeouts.request))?;
        Ok(Box::new(StreamConnection::new(stream)))
    }
}

/// Placeholder arm for platforms without a transport: the caller always stays in-process.
#[cfg(not(unix))]
pub struct UnavailableTransport;

#[cfg(not(unix))]
impl SharedIndexTransport for UnavailableTransport {
    fn connect(
        &self,
        _hello: &Handshake,
        _timeouts: &Timeouts,
    ) -> Result<Box<dyn SharedIndexConnection>, IpcError> {
        Err(IpcError::Unavailable(
            "no shared index transport on this platform".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn hello() -> Handshake {
        Handshake {
            version: "1.2.3".into(),
            build_id: "aaaa".into(),
            wire_schema_hash: "bbbb".into(),
            repo_root: "/r".into(),
            env_fingerprint: "cccc".into(),
            config_hash: "dddd".into(),
        }
    }

    #[test]
    fn frame_round_trips() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"{\"a\":1}").unwrap();
        assert_eq!(&buf[..4], &[0, 0, 0, 7]);
        assert_eq!(read_frame(&mut Cursor::new(buf)).unwrap(), b"{\"a\":1}");
    }

    #[test]
    fn empty_stream_is_closed_not_error() {
        assert!(matches!(
            read_frame(&mut Cursor::new(Vec::new())),
            Err(IpcError::Closed)
        ));
    }

    #[test]
    fn truncated_prefix_and_body_are_unexpected_eof() {
        let mut short_prefix = Cursor::new(vec![0u8, 0]);
        let e = read_frame(&mut short_prefix).unwrap_err();
        assert!(e.is_disconnect(), "{e}");
        let mut short_body = Cursor::new(vec![0u8, 0, 0, 9, b'x']);
        let e = read_frame(&mut short_body).unwrap_err();
        assert!(e.is_disconnect(), "{e}");
    }

    #[test]
    fn oversized_length_rejected_before_allocation() {
        let mut c = Cursor::new(vec![0xff, 0xff, 0xff, 0xff]);
        assert!(matches!(
            read_frame(&mut c),
            Err(IpcError::FrameTooLarge(n)) if n == u64::from(u32::MAX)
        ));
    }

    #[test]
    fn handshake_mismatch_names_each_field() {
        let a = hello();
        assert_eq!(a.mismatch(&a), None);
        type Mutation = fn(&mut Handshake);
        let cases: [(Mutation, &str); 6] = [
            (|h| h.version = "9".into(), "version"),
            (|h| h.build_id = "x".into(), "build_id"),
            (|h| h.wire_schema_hash = "x".into(), "wire_schema_hash"),
            (|h| h.repo_root = "/other".into(), "repo_root"),
            (|h| h.env_fingerprint = "x".into(), "env_fingerprint"),
            (|h| h.config_hash = "x".into(), "config_hash"),
        ];
        for (mutate, field) in cases {
            let mut b = hello();
            mutate(&mut b);
            assert_eq!(a.mismatch(&b), Some(field));
        }
    }

    #[test]
    fn env_fingerprint_tracks_output_env_only() {
        let base = env_fingerprint_with(|_| None);
        let with_cap =
            env_fingerprint_with(|k| (k == "CXPAK_MAX_FILE_BYTES").then(|| "10".to_string()));
        assert_ne!(base, with_cap);
        let other_cap =
            env_fingerprint_with(|k| (k == "CXPAK_MAX_FILE_BYTES").then(|| "11".to_string()));
        assert_ne!(with_cap, other_cap);
        let unrelated = env_fingerprint_with(|k| (k == "CXPAK_UNRELATED").then(|| "x".to_string()));
        assert_eq!(base, unrelated);
    }

    #[test]
    fn env_fingerprint_tracks_home() {
        let base = env_fingerprint_with(|_| None);
        let home = env_fingerprint_with(|k| (k == "HOME").then(|| "/h/a".to_string()));
        assert_ne!(base, home);
    }

    fn repo_with_config(config: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".cxpak.json"), config).unwrap();
        dir
    }

    #[test]
    fn handshake_tracks_the_value_of_the_configured_api_key_variable() {
        let var = "CXPAK_TEST_IPC_EMBED_KEY";
        let repo = repo_with_config(&format!(
            r#"{{"embeddings":{{"provider":"openai","api_key_env":"{var}"}}}}"#
        ));
        std::env::set_var(var, "secret-a");
        let a = Handshake::local_with(repo.path(), "b".into());
        std::env::set_var(var, "secret-b");
        let b = Handshake::local_with(repo.path(), "b".into());
        std::env::remove_var(var);
        assert_ne!(a.env_fingerprint, b.env_fingerprint);
        assert!(!a.env_fingerprint.contains("secret"));
    }

    #[test]
    fn handshake_tracks_the_config_file_content() {
        let repo = repo_with_config(r#"{"embeddings":{"provider":"local"}}"#);
        let before = Handshake::local_with(repo.path(), "b".into());
        std::fs::write(
            repo.path().join(".cxpak.json"),
            r#"{"embeddings":{"provider":"local","model":"other"}}"#,
        )
        .unwrap();
        let after = Handshake::local_with(repo.path(), "b".into());
        assert!(before.mismatch(&after).is_some(), "config differs");
    }

    #[test]
    fn wire_schema_hash_is_stable_and_nonempty() {
        assert_eq!(wire_schema_hash(), wire_schema_hash());
        assert_eq!(wire_schema_hash().len(), 16);
    }

    #[test]
    fn build_id_of_follows_content() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("bin");
        std::fs::write(&f, b"one").unwrap();
        let a = build_id_of(&f).unwrap();
        std::fs::write(&f, b"two").unwrap();
        assert_ne!(a, build_id_of(&f).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn handshake_and_requests_over_a_socket_pair() {
        use std::os::unix::net::UnixStream;
        let (mut c, mut s) = UnixStream::pair().unwrap();
        let expected = hello();
        let srv = std::thread::spawn(move || {
            server_handshake(&mut s, &hello(), Ok(())).unwrap();
            serve_requests(&mut s, &mut |req| serde_json::json!({"echo": req})).unwrap();
        });
        client_handshake(&mut c, &expected).unwrap();
        let mut conn = StreamConnection::new(c);
        let reply = conn.call(&serde_json::json!({"id": 1})).unwrap();
        assert_eq!(reply, serde_json::json!({"echo": {"id": 1}}));
        drop(conn);
        srv.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn mismatched_handshake_is_rejected_with_the_field() {
        use std::os::unix::net::UnixStream;
        let (mut c, mut s) = UnixStream::pair().unwrap();
        let srv = std::thread::spawn(move || server_handshake(&mut s, &hello(), Ok(())));
        let mut other = hello();
        other.build_id = "different".into();
        let err = client_handshake(&mut c, &other).unwrap_err();
        assert!(
            matches!(&err, IpcError::Rejected(r) if r.contains("build_id")),
            "{err}"
        );
        assert!(matches!(srv.join().unwrap(), Err(IpcError::Rejected(_))));
    }

    #[cfg(unix)]
    #[test]
    fn full_server_rejects_even_a_matching_handshake() {
        use std::os::unix::net::UnixStream;
        let (mut c, mut s) = UnixStream::pair().unwrap();
        let srv = std::thread::spawn(move || server_handshake(&mut s, &hello(), Err("busy")));
        let err = client_handshake(&mut c, &hello()).unwrap_err();
        assert!(
            matches!(&err, IpcError::Rejected(r) if r == "busy"),
            "{err}"
        );
        srv.join().unwrap().unwrap_err();
    }
}
