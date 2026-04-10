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

    pub async fn custom_health(&self) -> tower_lsp::jsonrpc::Result<serde_json::Value> {
        let idx = self.index.read().unwrap();
        match super::methods::handle_custom_method("cxpak/health", serde_json::Value::Null, &idx) {
            Ok(Some(v)) => Ok(v),
            Ok(None) => Ok(serde_json::Value::Null),
            Err(e) => Err(tower_lsp::jsonrpc::Error {
                code: tower_lsp::jsonrpc::ErrorCode::InternalError,
                message: e.into(),
                data: None,
            }),
        }
    }

    pub async fn custom_conventions(&self) -> tower_lsp::jsonrpc::Result<serde_json::Value> {
        let idx = self.index.read().unwrap();
        match super::methods::handle_custom_method(
            "cxpak/conventions",
            serde_json::Value::Null,
            &idx,
        ) {
            Ok(Some(v)) => Ok(v),
            Ok(None) => Ok(serde_json::Value::Null),
            Err(e) => Err(tower_lsp::jsonrpc::Error {
                code: tower_lsp::jsonrpc::ErrorCode::InternalError,
                message: e.into(),
                data: None,
            }),
        }
    }

    pub async fn custom_blast_radius(&self) -> tower_lsp::jsonrpc::Result<serde_json::Value> {
        let idx = self.index.read().unwrap();
        match super::methods::handle_custom_method(
            "cxpak/blastRadius",
            serde_json::Value::Null,
            &idx,
        ) {
            Ok(Some(v)) => Ok(v),
            Ok(None) => Ok(serde_json::Value::Null),
            Err(e) => Err(tower_lsp::jsonrpc::Error {
                code: tower_lsp::jsonrpc::ErrorCode::InternalError,
                message: e.into(),
                data: None,
            }),
        }
    }

    pub async fn custom_stub(
        &self,
        params: serde_json::Value,
    ) -> tower_lsp::jsonrpc::Result<serde_json::Value> {
        let method = params
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        Ok(serde_json::json!({"status": "available", "method": method}))
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

    async fn code_lens(&self, params: CodeLensParams) -> LspResult<Option<Vec<CodeLens>>> {
        let uri = params.text_document.uri.to_string();
        let idx = self.index.read().unwrap();
        let lenses = super::methods::code_lens_for_file(&uri, &idx);
        Ok(if lenses.is_empty() {
            None
        } else {
            Some(lenses)
        })
    }

    async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        let word = params
            .text_document_position_params
            .position
            .line
            .to_string();
        // In a real implementation we'd resolve the word under cursor from
        // the document.  For now, use the URI path as a fallback and let
        // hover_for_symbol return None if not found.
        let _ = word;
        Ok(None)
    }

    async fn diagnostic(
        &self,
        params: DocumentDiagnosticParams,
    ) -> LspResult<DocumentDiagnosticReportResult> {
        let uri = params.text_document.uri.to_string();
        let idx = self.index.read().unwrap();
        let diags = super::methods::diagnostics_for_file(&uri, &idx);
        Ok(DocumentDiagnosticReportResult::Report(
            DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                related_documents: None,
                full_document_diagnostic_report: FullDocumentDiagnosticReport {
                    result_id: None,
                    items: diags,
                },
            }),
        ))
    }

    #[allow(deprecated)]
    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> LspResult<Option<Vec<SymbolInformation>>> {
        let idx = self.index.read().unwrap();
        let symbols = super::methods::workspace_symbols(&params.query, &idx);
        Ok(if symbols.is_empty() {
            None
        } else {
            Some(symbols)
        })
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
