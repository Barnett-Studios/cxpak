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
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        // Race the LSP serve loop against SIGTERM/Ctrl-C so containerised
        // hosts (kubectl, systemd, docker stop) don't kill us mid-RPC.
        // tower-lsp's `Server::serve` future returns when the client closes
        // stdin (the normal termination signal from any LSP client), but a
        // SIGTERM from outside the LSP protocol stream would otherwise be
        // ignored — the kernel default disposition would terminate the
        // process and drop the in-flight JSON-RPC response.  Same shape as
        // commands::serve.rs's HTTP/MCP shutdown handler.
        let serve_fut = Server::new(stdin, stdout, socket).serve(service);
        let shutdown_fut = async {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                match signal(SignalKind::terminate()) {
                    Ok(mut term) => {
                        tokio::select! {
                            _ = tokio::signal::ctrl_c() => {},
                            _ = term.recv() => {},
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
            eprintln!("cxpak lsp: shutting down gracefully...");
        };
        tokio::select! {
            _ = serve_fut => {}
            _ = shutdown_fut => {}
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
