//! The official Rust client for a cxpak MCP server.
//!
//! `trait CxpakClient` abstracts the transport. `RmcpCxpakClient` speaks to a real server
//! over a child-process stdio transport; `RecordedCxpakClient` replays committed recordings
//! and is what consumers' own test suites construct.
//!
//! **`call` returns `Option<Value>`, and `None` means the tool was unavailable, errored or
//! degraded — never "the server looked and found nothing".** Callers map `None` to a skipped
//! observation rather than to a verdict. Collapsing that into an empty-but-present bundle is
//! the silent false negative this seam exists to prevent.
//!
//! Behind the non-default `client` feature: it pulls `rmcp`, `async-trait` and a wider tokio
//! than the rest of cxpak needs, and `cargo install cxpak` must stay an indexer (see #32).
//!
//! Typed DTOs carry `#[serde(default)]` throughout, so an absent or null field deserializes
//! to its zero value instead of needing manual fallback at every call site.

use async_trait::async_trait;
use rmcp::{
    model::CallToolRequestParams,
    transport::{ConfigureCommandExt, TokioChildProcess},
    ServiceExt,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Mutex as AsyncMutex;

/// One MCP tool call. `None` = tool unavailable / errored / degraded — callers
/// map `None` to `Observation::Skipped` (spec §5.6). Never panics on the caller.
#[async_trait]
pub trait CxpakClient: Send + Sync {
    async fn call(&self, tool: &str, args: Value) -> Option<Value>;
}

/// Test double: replays a name→response map. Async but never awaits real I/O.
pub struct RecordedCxpakClient {
    recordings: HashMap<String, Value>,
}

impl RecordedCxpakClient {
    pub fn new(recordings: HashMap<String, Value>) -> Self {
        Self { recordings }
    }
    /// Load every `conformance/recordings/cxpak/<tool>.json` into a client.
    /// A missing directory is a hard error (misconfiguration). Per-entry and
    /// per-file failures (bad permissions, malformed JSON) are skipped so that
    /// one bad recording does not abort the whole client construction.
    pub fn from_dir(dir: &std::path::Path) -> std::io::Result<Self> {
        let mut recordings = HashMap::new();
        for entry in std::fs::read_dir(dir)? {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                let Ok(raw) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                    recordings.insert(stem.to_string(), v);
                }
            }
        }
        Ok(Self { recordings })
    }
}

#[async_trait]
impl CxpakClient for RecordedCxpakClient {
    async fn call(&self, tool: &str, _args: Value) -> Option<Value> {
        self.recordings.get(tool).cloned()
    }
}

// ---- Typed response DTOs (tolerant: every field has a default) ----
// Field names/shapes are confirmed against cxpak 2.3.0 recordings (Task 4.2) AND
// re-verified against a live cxpak 3.0.0 MCP server (2026-07-11). 6/7 capabilities
// are shape-compatible; `dead_code` (name→symbol, total_scanned→total) and
// `predict` (risk_score → per-file impact lists) drifted in 3.0.0 and carry
// serde aliases / additive fields below to support both versions.
// Every optional field uses `#[serde(default)]` so absent keys deserialize to the zero value.

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Health {
    #[serde(default)]
    pub conventions: f64,
    #[serde(default)]
    pub dead_code: Option<f64>,
    #[serde(default)]
    pub composite: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeadSymbol {
    #[serde(default)]
    pub file: String,
    /// cxpak 2.3.0 emits `name`; 3.0.0 renamed it to `symbol` (verified against a
    /// live 3.0.0 MCP response). Alias keeps both working. Only `file` is used by
    /// the verifier's logic, so this is evidence-only, but keep it accurate.
    #[serde(default, alias = "symbol")]
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeadCode {
    #[serde(default)]
    pub dead_symbols: Vec<DeadSymbol>,
    /// Field absent → 0 (serde default). cxpak 2.3.0 `total_scanned`; 3.0.0 `total`.
    #[serde(default, alias = "total")]
    pub total_scanned: u64,
}

/// cxpak_predict response. cxpak 3.0.0 (ADR-0174) restructured this from a single
/// `risk_score` (+ `test_predictions`) into per-file impact lists plus a
/// `confidence_summary` string — verified against a live 3.0.0 MCP response. Both
/// shapes are captured here; `confidence_summary.is_some()` discriminates 3.0.0
/// (whose `risk_score` is absent → would default to 0.0 and silently pass).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Predict {
    #[serde(default)]
    pub risk_score: f64,
    #[serde(default)]
    pub test_predictions: Vec<Value>,
    #[serde(default)]
    pub structural_impact: Vec<Value>,
    #[serde(default)]
    pub call_impact: Vec<Value>,
    #[serde(default)]
    pub historical_impact: Vec<Value>,
    #[serde(default)]
    pub test_impact: Vec<Value>,
    #[serde(default)]
    pub confidence_summary: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CallGraph {
    #[serde(default)]
    pub unresolved: Vec<Value>,
    #[serde(default)]
    pub edges: Vec<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Violation {
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub line: Option<u64>,
    #[serde(default)]
    pub rule: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Verify {
    #[serde(default)]
    pub violations: Vec<Violation>,
    /// Zero or absent → caller falls back to `changed_files.len()` for the file count.
    #[serde(default)]
    pub files_checked: u64,
}

/// Per-module architectural data from cxpak_architecture (cxpak 2.3.0 real shape).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ArchModule {
    #[serde(default)]
    pub boundary_violations: Vec<Value>,
    #[serde(default)]
    pub god_files: Vec<Value>,
}

/// cxpak_architecture response. Top-level `circular_deps` + per-module arrays.
/// The old plan had top-level boundary_violations/god_files — WRONG (Task 4.2 §9 fix).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Architecture {
    #[serde(default)]
    pub circular_deps: Vec<Value>,
    #[serde(default)]
    pub modules: Vec<ArchModule>,
}

/// cxpak_security_surface response using cxpak 2.3.0 field names.
/// Legacy names `.secrets`/`.sql_injection` are absent in real responses;
/// `secret_patterns`/`sql_injection_surface` are the correct fields (0 findings in the golden fixture either way).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SecuritySurface {
    #[serde(default)]
    pub secret_patterns: Vec<Value>,
    #[serde(default)]
    pub sql_injection_surface: Vec<Value>,
    #[serde(default)]
    pub unprotected_endpoints: Vec<Value>,
}

// ---- SpawnBackoff: 3-strike consecutive-spawn-failure backoff (spec §5.6) ----
// Increment on a child exit with `served_one == false`; reset on `served_one == true`
// or manual close. Deterministic, no I/O.

/// Tracks consecutive child-spawn failures. When `strikes() >= max`, `may_spawn()`
/// returns false and callers degrade to `Observation::Skipped` rather than retrying.
pub struct SpawnBackoff {
    max: u32,
    consecutive: u32,
}

impl SpawnBackoff {
    pub fn new(max: u32) -> Self {
        Self {
            max,
            consecutive: 0,
        }
    }

    /// Returns true if a new spawn attempt is allowed.
    pub fn may_spawn(&self) -> bool {
        self.consecutive < self.max
    }

    /// Called when a child exits. `served_one = true` means the child completed at
    /// least one successful tool call before exiting; `false` means it never served
    /// a request (crash-on-start, handshake failure, etc.).
    pub fn record_exit(&mut self, served_one: bool) {
        if served_one {
            self.consecutive = 0;
        } else {
            self.consecutive += 1;
        }
    }

    /// Current consecutive-failure count.
    pub fn strikes(&self) -> u32 {
        self.consecutive
    }

    /// Reset the failure count (e.g. on a manual close that should not be penalised).
    pub fn reset(&mut self) {
        self.consecutive = 0;
    }
}

// ---- RmcpCxpakClient: live MCP transport over stdio child process ----
// Spawns `cxpak serve --mcp <work_dir>`, performs the MCP `initialize` handshake
// via rmcp, then issues `tools/call` requests. Lazy: spawns on first `call()`.
// Per-call timeout: 10 s. Backoff: 3 consecutive spawn/handshake failures → give up.
// Drop: synchronous SIGKILL via libc (no reliance on the tokio scheduler for teardown).
//
// Concurrency: rmcp's Peer<RoleClient> is Clone (#[derive(Clone)]) and uses an
// mpsc::Sender internally — concurrent calls over one stdio connection are multiplexed
// by request-id in rmcp's background service task. The async mutex is held ONLY during
// the brief setup phase (spawn-if-None + peer clone); it is released before call_tool
// is awaited, so tokio::join! over N tools runs N calls in true parallel.

/// cxpak 3.1.0 builds its index in the background on spawn and answers calls made
/// before the index is warm with a plaintext "indexing in progress" message (not
/// JSON). The client polls past it until the index warms or this budget expires.
/// Cold index measured 10–14s on a real repo; 25s covers a slow machine with margin.
/// On expiry the call returns None → the verifier Skips (fail-open, unchanged).
const INDEX_WARM_BUDGET: std::time::Duration = std::time::Duration::from_secs(25);
/// Poll interval between "indexing in progress" retries.
const INDEX_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
/// Cap the MCP spawn+handshake so a wedged `cxpak serve` cannot hang the caller
/// unbounded (was previously unbounded — cxpak-warm-index spec §caller-timeout).
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// cxpak's still-indexing sentinel. Matched on the response text: the message is not
/// JSON, so it otherwise fails to parse and is indistinguishable from real junk.
fn is_indexing(text: &str) -> bool {
    text.contains("indexing in progress")
}

/// Active rmcp connection to a running `cxpak serve --mcp` child.
struct ActiveConn {
    /// The connected MCP client. Deref gives `Peer<RoleClient>` for tool calls.
    service: rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    /// Whether this child has successfully served at least one tool call.
    /// Shared Arc so callers can update it after releasing the conn lock.
    served_one: Arc<AtomicBool>,
    /// OS PID captured before rmcp takes ownership of the transport, used for
    /// synchronous SIGKILL (rmcp's async kill chain requires scheduler cooperation
    /// which `#[tokio::test]` does not guarantee post-fn-return).
    child_pid: u32,
}

/// Live MCP client: lazy-spawns `cxpak serve --mcp <work_dir>`, performs the MCP
/// initialize handshake via rmcp, then issues `tools/call` with a 10 s timeout.
///
/// The async mutex is released before each `call_tool` await, enabling concurrent
/// calls (e.g. from `verify_all`'s `tokio::join!`) to run in parallel over a single
/// stdio connection.
pub struct RmcpCxpakClient {
    work_dir: std::path::PathBuf,
    conn: AsyncMutex<Option<ActiveConn>>,
    backoff: std::sync::Mutex<SpawnBackoff>,
    /// Set to true before any teardown so error paths skip recording a strike.
    closed: AtomicBool,
}

impl RmcpCxpakClient {
    /// Create a lazy client for `work_dir`. No child is spawned until the first call.
    pub fn new(work_dir: std::path::PathBuf) -> Self {
        Self {
            work_dir,
            conn: AsyncMutex::new(None),
            backoff: std::sync::Mutex::new(SpawnBackoff::new(3)),
            closed: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl CxpakClient for RmcpCxpakClient {
    async fn call(&self, tool: &str, args: Value) -> Option<Value> {
        if self.closed.load(Ordering::Relaxed) {
            return None;
        }

        // Phase 1: hold the mutex only for setup — spawn if needed, then clone peer.
        // The guard is dropped at the end of this block so the 10 s call_tool await
        // in Phase 2 does not serialise concurrent callers.
        let (peer, served_one) = {
            let mut guard = self.conn.lock().await;

            if guard.is_none() {
                {
                    let backoff = self.backoff.lock().unwrap();
                    if !backoff.may_spawn() {
                        return None;
                    }
                }

                // Null the child's stderr via rmcp's BUILDER. `cxpak serve` prints a
                // startup banner ("MCP server ready ...") to stderr; if inherited it
                // violates the hook contract (§5.1: hooks never write stderr). rmcp's
                // `TokioChildProcess::new` hardcodes `stderr = Stdio::inherit()` and
                // its `.spawn()` overrides any stderr set on the Command, so the null
                // MUST be applied on the builder (not in `.configure`).
                let transport = match TokioChildProcess::builder(
                    tokio::process::Command::new("cxpak").configure(|cmd| {
                        cmd.arg("serve").arg("--mcp").arg(&self.work_dir);
                    }),
                )
                .stderr(std::process::Stdio::null())
                .spawn()
                {
                    Ok((t, _stderr)) => t,
                    Err(_) => {
                        self.backoff.lock().unwrap().record_exit(false);
                        return None;
                    }
                };

                // Capture PID before `.serve()` consumes the transport; used for
                // synchronous SIGKILL in Drop and in the error-path cleanup below.
                let child_pid = match transport.id() {
                    Some(pid) => pid,
                    None => {
                        self.backoff.lock().unwrap().record_exit(false);
                        return None;
                    }
                };

                // Time-box the handshake: a wedged `cxpak serve` must not hang the
                // caller unbounded. On timeout/failure kill the child so it cannot leak.
                match tokio::time::timeout(HANDSHAKE_TIMEOUT, ().serve(transport)).await {
                    Ok(Ok(service)) => {
                        *guard = Some(ActiveConn {
                            service,
                            served_one: Arc::new(AtomicBool::new(false)),
                            child_pid,
                        });
                    }
                    Ok(Err(_)) | Err(_) => {
                        kill_child(child_pid);
                        self.backoff.lock().unwrap().record_exit(false);
                        return None;
                    }
                }
            }

            let conn = guard
                .as_mut()
                .expect("conn invariant: spawn block sets Some or returns early");
            // Clone the Peer (cheap: clones the mpsc::Sender + Arc refs) and the
            // served_one flag. Both are valid without holding the conn lock.
            // The guard — and the lock — drop at the end of this block.
            (conn.service.peer().clone(), Arc::clone(&conn.served_one))
        };

        // Phase 2: issue the tool call WITHOUT holding the conn mutex, retrying past
        // cxpak's "indexing in progress" response until the index warms or the budget
        // expires. cxpak 3.1.0 indexes in the background on spawn; the first call after
        // spawn polls up to ~14s, later calls (warm index) return immediately.
        let tool_name = tool.to_owned();
        let deadline = tokio::time::Instant::now() + INDEX_WARM_BUDGET;
        loop {
            let params = match &args {
                Value::Object(map) => {
                    CallToolRequestParams::new(tool_name.clone()).with_arguments(map.clone())
                }
                _ => CallToolRequestParams::new(tool_name.clone()),
            };

            let timed =
                tokio::time::timeout(std::time::Duration::from_secs(10), peer.call_tool(params))
                    .await;

            match timed {
                Ok(Ok(result)) => {
                    // cxpak returns the payload as a JSON string inside content[0].text.
                    let text = result
                        .content
                        .first()
                        .and_then(|c| c.as_text())
                        .map(|t| t.text.clone())?;
                    // Still indexing: retry-after, not data. Poll until warm or budget out.
                    if is_indexing(&text) {
                        if tokio::time::Instant::now() >= deadline {
                            return None;
                        }
                        tokio::time::sleep(INDEX_POLL_INTERVAL).await;
                        continue;
                    }
                    return match serde_json::from_str::<Value>(&text) {
                        Ok(parsed) => {
                            served_one.store(true, Ordering::Relaxed);
                            Some(parsed)
                        }
                        Err(_) => None,
                    };
                }
                Ok(Err(_)) | Err(_) => {
                    // Transport error or 10 s timeout: treat the connection as broken.
                    // Re-acquire the lock to clean up. Only the FIRST error reporter for
                    // this connection generation removes it — identified by Arc identity —
                    // so a stale error from a parallel call cannot evict a new connection.
                    let mut guard = self.conn.lock().await;
                    let is_same_conn = guard
                        .as_ref()
                        .map(|c| Arc::ptr_eq(&c.served_one, &served_one))
                        .unwrap_or(false);
                    if is_same_conn {
                        let conn = guard
                            .take()
                            .expect("conn invariant: checked Some immediately above");
                        let did_serve = conn.served_one.load(Ordering::Relaxed);
                        kill_child(conn.child_pid);
                        drop(conn); // RunningService drops → rmcp cancel chain fires (child already dead)
                        if !self.closed.load(Ordering::Relaxed) {
                            self.backoff.lock().unwrap().record_exit(did_serve);
                        }
                    }
                    return None;
                }
            }
        }
    }
}

impl Drop for RmcpCxpakClient {
    fn drop(&mut self) {
        // Mark closed before the state drops so any error path racing with Drop
        // skips recording a strike, preserving correct backoff semantics.
        self.closed.store(true, Ordering::Relaxed);

        // With the lock held only during setup (not across the 10 s call_tool await),
        // try_lock reliably succeeds here even when a call is in-flight: the setup phase
        // is sub-millisecond, and Drop is called only after the owning scope ends (no
        // live references remain that could be mid-setup on another thread).
        if let Ok(mut guard) = self.conn.try_lock() {
            if let Some(conn) = guard.take() {
                kill_child(conn.child_pid);
                drop(conn); // RunningService → rmcp cancel chain (child already dead)
            }
        }
        // If try_lock fails (exceptional case: another task in setup at the exact
        // moment of Drop), the Mutex drop will drop RunningService when the setup task
        // releases it; the child will exit on its stdin EOF shortly after.
    }
}

/// Send SIGKILL to the given OS process. No-op on non-Unix.
#[cfg(unix)]
fn kill_child(pid: u32) {
    // Safety: `pid` is a child process we spawned. `kill(2)` with SIGKILL is
    // async-signal-safe and has no UB regardless of the process state.
    // Expected return values: 0 (killed), ESRCH (child already exited — benign),
    // EPERM (PID reused by an unrelated process after our child exited — we accept
    // the no-op; the OS will not let us kill a process we do not own).
    let _ = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
}
#[cfg(not(unix))]
fn kill_child(_pid: u32) {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn recorded_client_replays_and_misses() {
        let mut rec = std::collections::HashMap::new();
        rec.insert("cxpak_health".to_string(), json!({ "conventions": 8.0 }));
        let client = RecordedCxpakClient::new(rec);
        let hit = client.call("cxpak_health", json!({})).await;
        assert_eq!(hit, Some(json!({ "conventions": 8.0 })));
        assert_eq!(client.call("cxpak_missing", json!({})).await, None);
    }

    #[test]
    fn health_dto_defaults_are_tolerant() {
        let h: Health = serde_json::from_value(json!({})).unwrap();
        assert_eq!(h.conventions, 0.0);
        assert!(h.dead_code.is_none());
    }

    #[test]
    fn is_indexing_matches_cxpak_sentinel_only() {
        // The live retry loop hinges on this exact substring; cxpak 3.1.0 emits
        // "cxpak: indexing in progress — Retry this call in a few seconds".
        assert!(is_indexing(
            "cxpak: indexing in progress — Retry this call in a few seconds"
        ));
        assert!(!is_indexing("{\"edges\":[],\"unresolved\":[]}"));
        assert!(!is_indexing("indexing complete"));
    }

    #[test]
    fn backoff_three_strikes_then_gives_up() {
        let mut b = SpawnBackoff::new(3);
        assert!(b.may_spawn());
        b.record_exit(false); // strike 1
        assert!(b.may_spawn());
        b.record_exit(false); // strike 2
        b.record_exit(false); // strike 3
        assert!(!b.may_spawn(), "3 consecutive failures → give up");
        b.record_exit(true); // served → reset
        assert_eq!(b.strikes(), 0);
        assert!(b.may_spawn());
    }
}
