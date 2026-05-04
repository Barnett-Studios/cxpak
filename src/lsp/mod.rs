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

        let serve_fut = Server::new(stdin, stdout, socket).serve(service);
        tokio::select! {
            _ = serve_fut => {
                // Normal in-protocol exit: client closed stdin.  Returning
                // from the async block lets `block_on` unwind and the
                // process exits via `main`'s normal return path.
            }
            _ = shutdown_rx => {
                // Signal-driven shutdown.  `tokio::io::stdin()` internally
                // spawns a blocking thread (libc `read()`) that cannot be
                // cancelled — even after we drop `serve_fut`, that thread
                // keeps the tokio runtime alive, so `block_on` would never
                // return and the process would hang forever (verified
                // empirically: select! resolves and the banner prints, but
                // the runtime doesn't unwind).  Force-exit via
                // `std::process::exit` is the right call here: the
                // graceful banner has already printed and there is no
                // protocol-level cleanup tower-lsp asks of us.  The
                // alternative — `axum::with_graceful_shutdown`-style
                // cooperative drain — isn't available on tower-lsp's
                // Server::serve.
                eprintln!("cxpak lsp: shutting down gracefully...");
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
