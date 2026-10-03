pub mod backend;
pub mod methods;

pub use backend::CxpakLspBackend;

/// The absolute workspace root the server anchors on.
///
/// `cxpak lsp` takes `[PATH] [default: .]`, and a RELATIVE root is not merely
/// untidy — it makes two separate things fail silently. `Url::from_file_path`
/// returns `Err` for a non-absolute path, so every `workspace/symbol` location
/// fell back to a `file:///unknown` placeholder (#75); and
/// `abs.strip_prefix(repo_root)` cannot match an absolute `file://` URI against
/// `.`, so every codeLens/diagnostic/hover resolution fell through to a suffix
/// match with no root bound (#76). One absolute root is the fix for the first
/// and the precondition for the second.
///
/// Canonicalising also resolves symlinks, which matters on macOS where a
/// client's `/var/folders/...` and the server's own view `/private/var/...`
/// name the same directory by different paths.
pub fn workspace_root(path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    std::fs::canonicalize(path)
}

/// Wraps the built `LspService` to observe the `shutdown`/`exit` lifecycle
/// from OUTSIDE tower-lsp's own state machine (#114).
///
/// tower-lsp's internal `exit` handling (`ExitService::call`) only stops
/// the service from accepting further requests — it does not, and cannot,
/// break the stdin read loop in `Server::serve` on its own, because that
/// loop only ends when `framed_stdin.next()` sees EOF. A client that sends
/// `exit` and then leaves stdin OPEN (perfectly spec-legal: `exit` alone is
/// "a notification to ask the server to exit its process" — closing the
/// pipe is a separate, optional step) would otherwise park the server
/// forever waiting for bytes that never arrive.
///
/// This layer watches every incoming request's method name as it passes
/// through: a `shutdown` that tower-lsp actually ACCEPTS (not one rejected
/// pre-`initialize`/post-`shutdown` with the `-32002` "not initialized"
/// error) flips `shutdown_received` once its response resolves (read by
/// the `exit` handler in `run_stdio` to pick the spec-mandated exit code);
/// `exit` fires `exit_tx` the moment the notification is seen —
/// independent of whatever the transport does with stdin afterward.
///
/// Every OTHER request's future is wrapped to hold `in_flight` above zero
/// for its duration, so `run_stdio`'s `exit` handler can tell whether
/// there is a response still being written and skip the drain grace
/// entirely when there is not (#114 follow-up: `exit` with nothing
/// outstanding was waiting out the full grace window regardless).
/// `exit` itself is excluded from this count — its own resolution is
/// near-instant and irrelevant to "is there something to drain".
struct ExitWatch<S> {
    inner: S,
    shutdown_received: std::sync::Arc<std::sync::atomic::AtomicBool>,
    exit_tx: std::sync::Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    in_flight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl<S> tower::Service<tower_lsp::jsonrpc::Request> for ExitWatch<S>
where
    S: tower::Service<
        tower_lsp::jsonrpc::Request,
        Response = Option<tower_lsp::jsonrpc::Response>,
        Error = tower_lsp::ExitedError,
    >,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future =
        std::pin::Pin<Box<dyn std::future::Future<Output = Result<S::Response, S::Error>> + Send>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: tower_lsp::jsonrpc::Request) -> Self::Future {
        if req.method() == "exit" {
            // `take()` makes this a one-shot signal even if a
            // (spec-violating) client sends `exit` twice — the second
            // send simply has nowhere to go rather than panicking on a
            // closed channel. Not counted in `in_flight`: `exit`'s own
            // resolution is near-instant and tells us nothing about
            // whether there is a response to drain.
            if let Ok(mut slot) = self.exit_tx.lock() {
                if let Some(tx) = slot.take() {
                    let _ = tx.send(());
                }
            }
            return Box::pin(self.inner.call(req));
        }

        // `shutdown` only counts toward `shutdown_received` if tower-lsp's
        // own state machine actually accepted it — a `shutdown` arriving
        // before `initialize` (or after a prior `shutdown`) is answered
        // with a `-32002` error, not `result: null`, and must not make a
        // following `exit` take the "clean" exit-code-0 path.
        let shutdown_received =
            (req.method() == "shutdown").then(|| std::sync::Arc::clone(&self.shutdown_received));

        self.in_flight
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let in_flight = std::sync::Arc::clone(&self.in_flight);
        let fut = self.inner.call(req);
        Box::pin(async move {
            let result = fut.await;
            in_flight.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            if let Some(flag) = shutdown_received {
                if let Ok(response) = &result {
                    if response.as_ref().and_then(|r| r.error()).is_none() {
                        flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            }
            result
        })
    }
}

/// Entry point for `cxpak lsp` — runs the LSP server over stdio until stdin
/// closes OR the client sends `exit` (#114: both must terminate the
/// process, not just end an in-process loop).
pub fn run_stdio(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use tower_lsp::{LspService, Server};
    // Anchor before indexing: a root that cannot be resolved is an error the
    // caller should see, not a placeholder to serve wrong answers from.
    let path = &workspace_root(path)?;
    let index = crate::commands::serve::build_index(path)?;
    // Inner Arc so the LSP dispatch can take an O(1) snapshot and run
    // long-running custom methods without holding the lock — see
    // SharedIndex docs in commands::serve.
    let shared = std::sync::Arc::new(std::sync::RwLock::new(std::sync::Arc::new(index)));
    let shared_path = std::sync::Arc::new(path.to_path_buf());
    let (service, socket) = LspService::build(|client| {
        CxpakLspBackend::new(
            client,
            std::sync::Arc::clone(&shared),
            std::sync::Arc::clone(&shared_path),
        )
    })
    .custom_method("cxpak/health", CxpakLspBackend::custom_health)
    .custom_method("cxpak/conventions", CxpakLspBackend::custom_conventions)
    .custom_method("cxpak/blastRadius", CxpakLspBackend::custom_blast_radius)
    .custom_method("cxpak/overview", CxpakLspBackend::custom_overview)
    .custom_method("cxpak/trace", CxpakLspBackend::custom_trace)
    .custom_method("cxpak/diff", CxpakLspBackend::custom_diff)
    .custom_method("cxpak/search", CxpakLspBackend::custom_search)
    .custom_method("cxpak/apiSurface", CxpakLspBackend::custom_api_surface)
    .custom_method("cxpak/deadCode", CxpakLspBackend::custom_dead_code)
    .custom_method("cxpak/callGraph", CxpakLspBackend::custom_call_graph)
    .custom_method("cxpak/graph", CxpakLspBackend::custom_graph)
    .custom_method("cxpak/retrieval", CxpakLspBackend::custom_retrieval)
    .custom_method("cxpak/predict", CxpakLspBackend::custom_predict)
    .custom_method("cxpak/drift", CxpakLspBackend::custom_drift)
    .custom_method(
        "cxpak/securitySurface",
        CxpakLspBackend::custom_security_surface,
    )
    .custom_method("cxpak/dataFlow", CxpakLspBackend::custom_data_flow)
    .finish();

    // #114: observe `shutdown`/`exit` independently of tower-lsp's own
    // stdin-driven loop — see `ExitWatch` for why that loop alone cannot
    // be trusted to end the process on `exit`.
    let shutdown_received = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let in_flight = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (exit_tx, exit_rx) = tokio::sync::oneshot::channel::<()>();
    let service = ExitWatch {
        inner: service,
        shutdown_received: std::sync::Arc::clone(&shutdown_received),
        exit_tx: std::sync::Arc::new(std::sync::Mutex::new(Some(exit_tx))),
        in_flight: std::sync::Arc::clone(&in_flight),
    };

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        // Spawn the signal listener on its OWN task so it's polled
        // independently of `Server::serve`.  Empirically tower-lsp's
        // serve loop (driven by tokio::io::stdin via a blocking helper)
        // can starve a sibling-branch `term.recv()` inside the SAME
        // select!: the signal future never gets a poll cycle and we
        // miss SIGTERM until much later — defeating the graceful
        // shutdown contract.  A separate task gets its own scheduler
        // slot, signals the main loop via a oneshot channel, and lets
        // `Server::serve` keep doing its blocking-read thing.
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                match signal(SignalKind::terminate()) {
                    Ok(mut term) => {
                        tokio::select! {
                            _ = tokio::signal::ctrl_c() => {}
                            _ = term.recv() => {}
                        }
                    }
                    Err(_) => {
                        tokio::signal::ctrl_c().await.ok();
                    }
                }
            }
            #[cfg(not(unix))]
            {
                tokio::signal::ctrl_c().await.ok();
            }
            let _ = shutdown_tx.send(());
        });

        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        // Race the LSP serve loop against the signal-listener task.
        // tower-lsp's `Server::serve` future returns when the client
        // closes stdin (the normal in-protocol exit); the spawned task
        // covers signal-driven shutdown for containerised hosts
        // (kubectl, systemd, docker stop) — same shape as
        // commands/serve.rs's HTTP/MCP shutdown handler, just split
        // across two tasks because tower-lsp's stdin-driven future
        // doesn't yield often enough for an in-place select! to poll
        // the signal branch reliably.
        // Tell anyone watching stderr that we're ready to accept LSP
        // messages and respond to signals.  Test harnesses can poll for
        // this line instead of sleeping a guessed-at duration.
        eprintln!("cxpak lsp: ready");

        // Spawn the serve loop as its own task so it keeps draining
        // stdin, dispatching, and writing responses for the duration
        // of the grace window after a signal.  If we instead awaited
        // `serve_fut` directly inside `select!`, the signal branch
        // resolving would drop the serve future — aborting tower-lsp's
        // dispatch loop entirely — and any request bytes already in
        // the pipe would never reach a handler.  Empirically that
        // yielded EOF (no response) for any request whose bytes hadn't
        // been parsed before the signal.  Spawning lets the dispatch
        // continue running in parallel; the grace sleep below decides
        // how long to let it.
        let serve_handle = tokio::spawn(Server::new(stdin, stdout, socket).serve(service));
        tokio::select! {
            res = serve_handle => {
                // Normal in-protocol exit: client closed stdin, with no
                // `exit` notification (or tower-lsp's own loop happened to
                // observe EOF before our `exit_rx` branch below fired).
                // Falling off the end of `run_stdio` and letting `rt` drop
                // normally is NOT safe here (#114): `tokio::io::stdin()`
                // parks a dedicated thread in a blocking `read()` that is
                // never told to stop, and `Runtime::drop` blocks
                // indefinitely waiting for that thread to finish — so an
                // ordinary return would hang the process forever even
                // though the LSP transport itself ended cleanly. Exit
                // explicitly instead.
                let _ = res;
                std::process::exit(0);
            }
            _ = exit_rx => {
                // #114: the client sent the `exit` notification. tower-lsp's
                // own `exit` handling only stops it from accepting further
                // requests — it does not break `Server::serve`'s stdin read
                // loop, so a client that sends `exit` without also closing
                // stdin (spec-legal: the two are independent) would
                // otherwise park this process forever. `ExitWatch` observes
                // the notification directly and fires `exit_tx` the moment
                // it is dispatched, regardless of stdin's state.
                //
                // Same drain rationale as the SIGTERM branch below: a
                // request sent just before `exit` may still be in flight on
                // the runtime, so give it the same grace window to finish
                // writing its response before cutting the process — but
                // ONLY if there is actually something in flight. The common
                // case (`shutdown` answered, then `exit`, nothing else
                // outstanding) has nothing to drain, and a client waiting
                // on this process to go away shouldn't eat a fixed ~1.5s
                // for no reason (#114 follow-up).
                eprintln!("cxpak lsp: exit notification received, shutting down...");
                if in_flight.load(std::sync::atomic::Ordering::SeqCst) > 0 {
                    let grace_ms: u64 = std::env::var("CXPAK_LSP_SHUTDOWN_GRACE_MS")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(1500);
                    tokio::time::sleep(std::time::Duration::from_millis(grace_ms)).await;
                }
                // Spec: 0 if `exit` followed a `shutdown` request, 1 (a
                // "non-zero code", left unspecified beyond that) otherwise.
                if shutdown_received.load(std::sync::atomic::Ordering::SeqCst) {
                    std::process::exit(0);
                } else {
                    std::process::exit(1);
                }
            }
            _ = shutdown_rx => {
                // Signal-driven shutdown.  `tokio::io::stdin()` internally
                // spawns a blocking thread (libc `read()`) that cannot be
                // cancelled — even after we drop `serve_fut`, that thread
                // keeps the tokio runtime alive, so `block_on` would never
                // return.  We must `process::exit` to actually leave; the
                // question is *when*.
                //
                // Empirically (race-test in tests/lsp_subprocess.rs's
                // `lsp_sigterm_drains_in_flight_response`), the in-flight
                // tower-lsp handler tasks continue running AFTER `select!`
                // resolves — they remain on the runtime, finishing their
                // work and writing JSON-RPC responses to stdout.  Without
                // a grace period, an immediate `process::exit` cuts the
                // response mid-write, the client sees EOF, and the
                // original "don't drop in-flight responses" motivation
                // for handling SIGTERM at all is defeated.
                //
                // Wait `LSP_SHUTDOWN_GRACE_MS` (default 1500ms) for
                // in-flight handlers to drain.  Methods that take longer
                // than the grace will still be cut — there is no clean
                // way to drain unboundedly without protocol-level
                // cooperation (LSP's `shutdown` request → `exit`
                // notification cycle, which only the client can drive).
                // Override via `CXPAK_LSP_SHUTDOWN_GRACE_MS=...` for
                // operators who need a longer drain window.
                eprintln!("cxpak lsp: shutting down gracefully...");
                let grace_ms: u64 = std::env::var("CXPAK_LSP_SHUTDOWN_GRACE_MS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1500);
                tokio::time::sleep(std::time::Duration::from_millis(grace_ms)).await;
                std::process::exit(0);
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn lsp_module_compiles_with_feature() {
        // Verify the module and re-export compile under the lsp feature.
        fn _check() -> fn(&std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
            super::run_stdio
        }
    }
}
