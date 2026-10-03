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
/// for its duration. `exit`'s own dispatch (see `call`) waits, bounded,
/// for `in_flight` to settle before delegating to `inner` — see the field
/// doc below for why. `run_stdio`'s `exit` handler does NOT use
/// `in_flight` to decide whether to skip its drain grace: an earlier
/// version did (skipping it entirely once `in_flight` read zero), but
/// zero only means a request's future RESOLVED, not that its response has
/// finished travelling through tower-lsp's own forwarding stream to the
/// actual stdout write — a real but narrower race than the one `in_flight`
/// was introduced to close. The grace is unconditional now; `in_flight`'s
/// only remaining job is the bounded wait below. `exit` itself is
/// excluded from the count — its own resolution is near-instant and
/// irrelevant to either use.
struct ExitWatch<S> {
    // `Option` so `exit`'s handling (see `call`) can `take()` it: tower-lsp's
    // own `ExitService::call` — reached by delegating to `inner` at all,
    // for the `exit` method — synchronously cancels every pending request
    // (`Pending::cancel_all`) the instant it runs. A `shutdown` pipelined
    // immediately before `exit` (sent without waiting for its response,
    // which the LSP spec does not require a conforming client to do before
    // sending `exit`... though it recommends it) can still be QUEUED,
    // not yet resolved, at that instant — and cancellation would turn its
    // well-formed `result: null` into a `-32800 "Canceled"` error, which
    // then fails the "did shutdown actually succeed" check below. Taking
    // `inner` out and delegating inside a future that first waits (bounded)
    // for `in_flight` to settle defers that cancellation until any
    // just-dispatched request has had a real chance to finish.
    inner: Option<S>,
    shutdown_received: std::sync::Arc<std::sync::atomic::AtomicBool>,
    exit_tx: std::sync::Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    in_flight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

/// RAII decrement for `ExitWatch::in_flight`, held across the wrapped
/// request's `.await`.
///
/// A plain `fetch_sub` placed AFTER `fut.await` only runs on the happy
/// path: if the future is dropped before completing (the client closes
/// stdin mid-request, `$/cancelRequest` fires, or a handler panics and
/// the panic unwinds through the `.await` point), that statement is
/// simply never reached and the counter leaks upward forever — every
/// later `exit` would then wrongly believe something is still in flight
/// and pay the full drain grace for the rest of the process's life.
/// `Drop` runs on every exit path (normal return, panic unwind, AND the
/// future being dropped while suspended), so holding the decrement here
/// instead closes all three.
struct InFlightGuard(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl<S> tower::Service<tower_lsp::jsonrpc::Request> for ExitWatch<S>
where
    S: tower::Service<
            tower_lsp::jsonrpc::Request,
            Response = Option<tower_lsp::jsonrpc::Response>,
            Error = tower_lsp::ExitedError,
        > + Send
        + 'static,
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
        match &mut self.inner {
            Some(inner) => inner.poll_ready(cx),
            // `inner` was taken for `exit`'s deferred dispatch (see `call`).
            // Any further message is a client protocol violation (nothing
            // should follow `exit`) — stay `Pending` rather than panic;
            // `run_stdio`'s own `exit_rx`/grace path exits the process
            // shortly regardless, bounding how long this can matter.
            None => std::task::Poll::Pending,
        }
    }

    fn call(&mut self, req: tower_lsp::jsonrpc::Request) -> Self::Future {
        if req.method() == "exit" {
            let in_flight = std::sync::Arc::clone(&self.in_flight);
            let exit_tx = std::sync::Arc::clone(&self.exit_tx);
            let Some(mut inner) = self.inner.take() else {
                // Unreachable in practice: `poll_ready` returns `Pending`
                // once `inner` is gone, and the transport always awaits
                // `poll_ready` before calling. No `S::Error` value is
                // constructible from here (tower-lsp's `ExitedError` has no
                // public constructor) — never resolving is the honest
                // signal for a state that should not occur.
                return Box::pin(std::future::pending());
            };
            return Box::pin(async move {
                // Bounded wait for any request dispatched just before
                // `exit` (most notably a pipelined `shutdown`) to actually
                // finish before delegating — which is what triggers
                // tower-lsp's own `Pending::cancel_all()` — so it is not
                // cancelled out from under us. 500ms is generous for any
                // ordinary handler and short against the ~1.5s default
                // drain grace; it only ever costs real time when something
                // was genuinely still in flight.
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(500);
                while in_flight.load(std::sync::atomic::Ordering::SeqCst) > 0
                    && tokio::time::Instant::now() < deadline
                {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
                // `take()` makes this a one-shot signal even if a
                // (spec-violating) client sends `exit` twice — the second
                // send simply has nowhere to go rather than panicking on a
                // closed channel.
                if let Ok(mut slot) = exit_tx.lock() {
                    if let Some(tx) = slot.take() {
                        let _ = tx.send(());
                    }
                }
                inner.call(req).await
            });
        }

        // `shutdown` only counts toward `shutdown_received` if tower-lsp's
        // own state machine actually accepted it — a `shutdown` arriving
        // before `initialize` (or after a prior `shutdown`) is answered
        // with a `-32002` error, not `result: null`, and must not make a
        // following `exit` take the "clean" exit-code-0 path.
        let shutdown_received =
            (req.method() == "shutdown").then(|| std::sync::Arc::clone(&self.shutdown_received));

        // `inner` is only ever taken by the `exit` branch above, and
        // `exit` is one-shot — this should be unreachable in practice.
        // `poll_ready` already returns `Pending` once `inner` is gone, so
        // a well-behaved transport cannot reach `call` in that state; if
        // it somehow does, the honest answer (no `S::Error` is
        // constructible from here — see `poll_ready`) is a future that
        // never resolves, not a panic.
        let Some(inner) = self.inner.as_mut() else {
            return Box::pin(std::future::pending());
        };

        self.in_flight
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let guard = InFlightGuard(std::sync::Arc::clone(&self.in_flight));
        let fut = inner.call(req);
        Box::pin(async move {
            let result = fut.await;
            // Set the flag BEFORE `guard` drops (and so before `in_flight`
            // is decremented) — `guard`, declared after `shutdown_received`
            // was captured but before this point, drops at the end of this
            // block, strictly after the store below runs. That ordering is
            // what closes the race a pipelined `shutdown` immediately
            // followed by `exit` could otherwise hit: the `exit` handler
            // reads `in_flight == 0` as "nothing to drain" and
            // `shutdown_received` to pick the exit code, and SeqCst's total
            // order guarantees it can never observe the decrement without
            // also observing this store.
            if let Some(flag) = shutdown_received {
                if let Ok(response) = &result {
                    if response.as_ref().and_then(|r| r.error()).is_none() {
                        flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            }
            drop(guard);
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
        inner: Some(service),
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
            // Matching `Ok(())` specifically (not a bare `_`) matters: on
            // plain stdin EOF with no `exit` ever sent, `Server::serve`
            // drops `service` (and so `ExitWatch`'s `exit_tx`) as part of
            // returning — which resolves `exit_rx` to `Err(RecvError)` at
            // essentially the same instant `serve_handle` above resolves.
            // A bare `_ = exit_rx` would match THAT too and could win the
            // race against the `res = serve_handle` arm, wrongly taking
            // this branch's exit(1)-on-no-shutdown path for a plain EOF
            // that should be exit(0). Only a genuine `exit` notification
            // sends `Ok(())`; a dropped sender's `Err` fails to match this
            // pattern, so `select!` falls through to the branches that are
            // still live instead of firing this one.
            Ok(()) = exit_rx => {
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
                // writing its response before cutting the process.
                //
                // ALWAYS applied, unconditionally — an earlier version of
                // this skipped the sleep entirely when `in_flight` read
                // zero (the common "shutdown answered, then exit, nothing
                // else outstanding" case), to avoid the full ~1.5s when
                // there was supposedly nothing to drain. That was racy:
                // `in_flight` reading zero only means a request's future
                // RESOLVED, not that its response has finished travelling
                // through tower-lsp's own forwarding stream to the actual
                // stdout write — `process::exit` does not wait for that
                // write to land, so the fast path could truncate output.
                // A fixed short cushion on that path narrowed the window
                // without closing it. There is no reliable way to KNOW the
                // write has landed from here without tokio exposing it, so
                // the grace is now unconditional; it still keeps total
                // exit time well under 2s by default.
                eprintln!("cxpak lsp: exit notification received, shutting down...");
                let grace_ms: u64 = std::env::var("CXPAK_LSP_SHUTDOWN_GRACE_MS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1500);
                tokio::time::sleep(std::time::Duration::from_millis(grace_ms)).await;
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

    // `ExitWatch`'s `in_flight`/`shutdown_received` bookkeeping, exercised
    // directly against a fake inner `Service` with a controllable delay —
    // deterministic and fast, unlike driving the same logic through a real
    // subprocess and a real (inherently depth-bounded, hard to reliably
    // slow down) LSP method. `tests/lsp_subprocess.rs` covers the
    // end-to-end, real-binary behaviour these unit tests can't see
    // (the actual process exit code and timing); this covers the
    // bookkeeping these unit tests are better suited to pin down exactly.
    mod exit_watch {
        use super::super::*;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};
        use tower::Service;
        use tower_lsp::jsonrpc::{Request, Response};

        /// A fake inner service that resolves successfully after a fixed
        /// delay — standing in for a real LSP method handler without
        /// depending on anything's actual runtime to create one.
        #[derive(Clone)]
        struct DelayedOk {
            delay_ms: u64,
        }

        impl Service<Request> for DelayedOk {
            type Response = Option<Response>;
            type Error = tower_lsp::ExitedError;
            type Future = std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
            >;

            fn poll_ready(
                &mut self,
                _cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<(), Self::Error>> {
                std::task::Poll::Ready(Ok(()))
            }

            fn call(&mut self, req: Request) -> Self::Future {
                let delay_ms = self.delay_ms;
                let id = req.id().cloned();
                Box::pin(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    Ok(id.map(|id| Response::from_ok(id, serde_json::Value::Null)))
                })
            }
        }

        fn watch(inner: DelayedOk, in_flight: &Arc<AtomicUsize>) -> ExitWatch<DelayedOk> {
            ExitWatch {
                inner: Some(inner),
                shutdown_received: Arc::new(AtomicBool::new(false)),
                exit_tx: Arc::new(Mutex::new(None)),
                in_flight: Arc::clone(in_flight),
            }
        }

        /// The bug #114's follow-up review flagged: a plain `fetch_sub`
        /// placed after `fut.await` never runs if the future is dropped
        /// before completing — e.g. the client closes stdin mid-request,
        /// or `$/cancelRequest` fires. `InFlightGuard`'s `Drop` must run
        /// regardless.
        #[tokio::test]
        async fn in_flight_does_not_leak_when_the_future_is_dropped_before_completing() {
            let in_flight = Arc::new(AtomicUsize::new(0));
            let mut w = watch(DelayedOk { delay_ms: 5_000 }, &in_flight);

            let fut = w.call(Request::build("textDocument/hover").id(1).finish());
            assert_eq!(
                in_flight.load(Ordering::SeqCst),
                1,
                "in_flight must be incremented as soon as the request is dispatched"
            );

            drop(fut); // simulate cancellation/drop before completion — never awaited.
            assert_eq!(
                in_flight.load(Ordering::SeqCst),
                0,
                "in_flight must still be decremented when the future is dropped \
                 before completing — a leak here means every later `exit` pays \
                 the drain grace forever"
            );
        }

        /// `in_flight` must also clear on the ordinary happy path — the
        /// companion case to the leak test above, proving the counter is a
        /// true reflection of "still running", not just "never leaks up".
        #[tokio::test]
        async fn in_flight_tracks_a_request_for_its_actual_duration() {
            let in_flight = Arc::new(AtomicUsize::new(0));
            let mut w = watch(DelayedOk { delay_ms: 50 }, &in_flight);

            let fut = w.call(Request::build("textDocument/hover").id(1).finish());
            assert_eq!(in_flight.load(Ordering::SeqCst), 1);

            fut.await.ok();
            assert_eq!(
                in_flight.load(Ordering::SeqCst),
                0,
                "in_flight must be back to zero once the request has actually resolved"
            );
        }

        /// The ordering #114's follow-up review flagged: a pipelined
        /// `shutdown` immediately followed by `exit` must never let `exit`
        /// observe `in_flight == 0` without `shutdown_received` already
        /// being `true` for a shutdown that actually succeeded.
        #[tokio::test]
        async fn shutdown_received_is_set_before_in_flight_clears() {
            let in_flight = Arc::new(AtomicUsize::new(0));
            let shutdown_received = Arc::new(AtomicBool::new(false));
            let mut w = ExitWatch {
                inner: Some(DelayedOk { delay_ms: 20 }),
                shutdown_received: Arc::clone(&shutdown_received),
                exit_tx: Arc::new(Mutex::new(None)),
                in_flight: Arc::clone(&in_flight),
            };

            let fut = w.call(Request::build("shutdown").id(1).finish());
            let resp = fut.await.expect("DelayedOk never errors");
            assert!(resp.is_some(), "shutdown must get a response");
            assert_eq!(
                in_flight.load(Ordering::SeqCst),
                0,
                "in_flight must have cleared by the time the future resolves"
            );
            assert!(
                shutdown_received.load(Ordering::SeqCst),
                "shutdown_received must be set — and per the ordering inside \
                 `call`, it is set strictly before `in_flight`'s decrement runs, \
                 so any reader that observes the decrement also observes this"
            );
        }
    }
}
