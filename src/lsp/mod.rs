pub mod backend;
pub mod methods;

pub use backend::CxpakLspBackend;

/// Entry point for `cxpak lsp` — runs the LSP server over stdio until stdin closes.
pub fn run_stdio(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use tower_lsp::{LspService, Server};
    let index = crate::commands::serve::build_index(path)?;
    let shared = std::sync::Arc::new(std::sync::RwLock::new(index));
    let shared_path = std::sync::Arc::new(path.to_path_buf());
    let (service, socket) = LspService::new(|client| {
        CxpakLspBackend::new(
            client,
            std::sync::Arc::clone(&shared),
            std::sync::Arc::clone(&shared_path),
        )
    });
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        Server::new(stdin, stdout, socket).serve(service).await;
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
