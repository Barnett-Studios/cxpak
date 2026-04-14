#[cfg(feature = "plugins")]
use super::{CxpakPlugin, Detection, FileSnapshot, Finding, IndexSnapshot, PluginCapability};
#[cfg(feature = "plugins")]
use std::path::Path;

#[cfg(feature = "plugins")]
const MAX_PLUGIN_BYTES: u64 = 10 * 1024 * 1024;

#[cfg(feature = "plugins")]
pub struct PluginLoader {
    engine: wasmtime::Engine,
}

#[cfg(feature = "plugins")]
impl PluginLoader {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let engine = wasmtime::Engine::default();
        Ok(Self { engine })
    }

    pub fn load(&self, path: &Path) -> Result<Box<dyn CxpakPlugin>, Box<dyn std::error::Error>> {
        let meta = std::fs::metadata(path)
            .map_err(|e| format!("cannot read plugin at {}: {e}", path.display()))?;
        if meta.len() > MAX_PLUGIN_BYTES {
            return Err(format!(
                "plugin too large: {} bytes (max {MAX_PLUGIN_BYTES})",
                meta.len()
            )
            .into());
        }
        let module = wasmtime::Module::from_file(&self.engine, path)
            .map_err(|e| format!("failed to compile WASM module {}: {e}", path.display()))?;
        let mut store = wasmtime::Store::new(&self.engine, ());
        let _instance = wasmtime::Instance::new(&mut store, &module, &[])
            .map_err(|e| format!("failed to instantiate WASM module {}: {e}", path.display()))?;
        // Guest function binding is the v2.0.0 skeleton — real binding will be implemented
        // when WIT interface generation is available
        Err(format!(
            "WASM plugin loaded ({} bytes) but guest function binding not yet implemented",
            meta.len()
        )
        .into())
    }
}

#[cfg(feature = "plugins")]
#[allow(dead_code)]
struct WasmPlugin {
    name: String,
    version: String,
    capabilities: Vec<PluginCapability>,
}

#[cfg(feature = "plugins")]
impl CxpakPlugin for WasmPlugin {
    fn name(&self) -> &str {
        &self.name
    }
    fn version(&self) -> &str {
        &self.version
    }
    fn capabilities(&self) -> Vec<PluginCapability> {
        self.capabilities.clone()
    }
    fn analyze(&self, _index: &IndexSnapshot) -> Vec<Finding> {
        vec![]
    }
    fn detect(&self, _file: &FileSnapshot) -> Vec<Detection> {
        vec![]
    }
}

#[cfg(all(test, feature = "plugins"))]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn load_plugin_non_existent_path_returns_err_with_path() {
        let loader = PluginLoader::new().expect("engine init");
        let path = std::path::Path::new("/nonexistent/plugin.wasm");
        let result = loader.load(path);
        assert!(result.is_err());
        let msg = result.err().expect("is err").to_string();
        assert!(
            msg.contains("/nonexistent/plugin.wasm"),
            "error message should contain the path, got: {msg}"
        );
    }

    #[test]
    fn load_plugin_too_large_returns_err() {
        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        // Write slightly more than 10 MB
        let chunk = vec![0u8; 1024 * 1024];
        for _ in 0..11 {
            tmp.write_all(&chunk).expect("write");
        }
        tmp.flush().expect("flush");

        let loader = PluginLoader::new().expect("engine init");
        let result = loader.load(tmp.path());
        assert!(result.is_err());
        let msg = result.err().expect("is err").to_string();
        assert!(
            msg.contains("plugin too large"),
            "error should mention 'plugin too large', got: {msg}"
        );
    }

    /// PluginLoader::new() must succeed — the wasmtime engine initialises without error.
    #[test]
    fn plugin_loader_new_succeeds() {
        let result = PluginLoader::new();
        assert!(result.is_ok(), "PluginLoader::new() must return Ok");
    }

    /// Loading a valid (but trivial) WASM module from disk should fail with an error
    /// that mentions "guest function binding", confirming that module compilation
    /// succeeds but the v2.0.0 stub rejects the call.
    #[test]
    fn load_valid_wasm_returns_err_containing_guest_function_binding() {
        // Minimal valid WASM module: magic header + version (8 bytes).
        let minimal_wasm: &[u8] = &[
            0x00, 0x61, 0x73, 0x6d, // magic: \0asm
            0x01, 0x00, 0x00, 0x00, // version: 1
        ];
        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        tmp.write_all(minimal_wasm).expect("write wasm");
        tmp.flush().expect("flush");

        let loader = PluginLoader::new().expect("engine init");
        let result = loader.load(tmp.path());
        assert!(
            result.is_err(),
            "loading a wasm plugin must return Err (stub not implemented)"
        );
        let msg = result.err().expect("is err").to_string();
        assert!(
            msg.contains("guest function binding"),
            "error should mention 'guest function binding', got: {msg}"
        );
    }
}
