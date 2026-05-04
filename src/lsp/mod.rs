pub mod backend;
pub mod methods;

pub use backend::CxpakLspBackend;

/// Entry point for `cxpak lsp` — runs the LSP server over stdio until stdin closes.
pub fn run_stdio(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use tower_lsp::{LspService, Server};
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
    .custom_method("cxpak/predict", CxpakLspBackend::custom_predict)
    .custom_method("cxpak/drift", CxpakLspBackend::custom_drift)
    .custom_method(
        "cxpak/securitySurface",
        CxpakLspBackend::custom_security_surface,
    )
    .custom_method("cxpak/dataFlow", CxpakLspBackend::custom_data_flow)
    .finish();
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
                // Normal in-protocol exit: client closed stdin.  Returning
                // from the async block lets `block_on` unwind and the
                // process exits via `main`'s normal return path.
                let _ = res;
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
