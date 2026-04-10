use std::sync::{Arc, RwLock};
use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{async_trait, Client, LanguageServer};

type SharedIndex = Arc<RwLock<crate::index::CodebaseIndex>>;
type SharedPath = Arc<std::path::PathBuf>;

pub struct CxpakLspBackend {
    pub client: Client,
    pub index: SharedIndex,
    pub path: SharedPath,
}

impl CxpakLspBackend {
    pub fn new(client: Client, index: SharedIndex, path: SharedPath) -> Self {
        Self {
            client,
            index,
            path,
        }
    }
}

#[async_trait]
impl LanguageServer for CxpakLspBackend {
    async fn initialize(&self, _params: InitializeParams) -> LspResult<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(false),
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                diagnostic_provider: Some(DiagnosticServerCapabilities::Options(
                    DiagnosticOptions {
                        inter_file_dependencies: true,
                        workspace_diagnostics: true,
                        ..Default::default()
                    },
                )),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(
                MessageType::INFO,
                format!("cxpak LSP initialized for {}", self.path.display()),
            )
            .await;

        let path = Arc::clone(&self.path);
        let index = Arc::clone(&self.index);
        let client = self.client.clone();
        tokio::spawn(async move {
            let watcher_or_err: Result<_, String> = crate::daemon::watcher::FileWatcher::new(&path)
                .map_err(|e| format!("cxpak: watcher failed: {e}"));
            let watcher = match watcher_or_err {
                Ok(w) => w,
                Err(msg) => {
                    client.log_message(MessageType::WARNING, msg).await;
                    return;
                }
            };
            loop {
                if let Some(first) = watcher.recv_timeout(std::time::Duration::from_secs(1)) {
                    let mut changes = vec![first];
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    changes.extend(watcher.drain());
                    crate::commands::serve::process_watcher_changes(&changes, &path, &index);
                    client
                        .log_message(
                            MessageType::INFO,
                            format!("cxpak: re-indexed {} changed files", changes.len()),
                        )
                        .await;
                }
            }
        });
    }

    async fn shutdown(&self) -> LspResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_struct_fields_are_accessible() {
        // CxpakLspBackend requires a real tower_lsp::Client which cannot be
        // constructed outside of LspService::new. This test verifies the struct
        // and type aliases compile correctly.
        fn _assert_types() {
            let _: fn(Client, SharedIndex, SharedPath) -> CxpakLspBackend = CxpakLspBackend::new;
        }
    }
}
