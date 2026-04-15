#[cfg(feature = "plugins")]
use super::{CxpakPlugin, Detection, FileSnapshot, Finding, IndexSnapshot, PluginCapability};
#[cfg(feature = "plugins")]
use std::path::Path;

#[cfg(feature = "plugins")]
const MAX_PLUGIN_BYTES: u64 = 10 * 1024 * 1024;

/// Maximum WASM linear memory a plugin may grow to (64 MiB).
#[cfg(feature = "plugins")]
const MAX_PLUGIN_MEMORY: usize = 64 * 1024 * 1024;

/// Epoch deadline in ticks (100 ticks × 100 ms/tick = 10 s wall clock).
#[cfg(feature = "plugins")]
const EPOCH_DEADLINE: u64 = 100;

#[cfg(feature = "plugins")]
struct PluginResourceLimiter {
    max_memory: usize,
}

#[cfg(feature = "plugins")]
impl wasmtime::ResourceLimiter for PluginResourceLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        Ok(desired <= self.max_memory)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        Ok(true)
    }
}

#[cfg(feature = "plugins")]
pub struct PluginLoader {
    engine: wasmtime::Engine,
}

#[cfg(feature = "plugins")]
impl PluginLoader {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut config = wasmtime::Config::new();
        config.epoch_interruption(true);
        config.consume_fuel(false);
        let engine = wasmtime::Engine::new(&config)?;

        // Spawn a background thread that increments the epoch every 100 ms.
        // This is a one-shot spawn; the thread persists for the process lifetime,
        // which is fine because cxpak is a short-lived CLI tool.
        let engine_clone = engine.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            engine_clone.increment_epoch();
        });

        Ok(Self { engine })
    }

    /// Verify `expected_checksum` on raw bytes **before** compiling WASM.
    fn verify_checksum_bytes(
        bytes: &[u8],
        expected_hex: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use sha2::{Digest, Sha256};
        use subtle::ConstantTimeEq;

        let actual = format!("{:x}", Sha256::digest(bytes));
        if actual.len() != expected_hex.len()
            || !bool::from(actual.as_bytes().ct_eq(expected_hex.as_bytes()))
        {
            return Err(format!("checksum mismatch: expected {expected_hex}, got {actual}").into());
        }
        Ok(())
    }

    pub fn load(
        &self,
        path: &Path,
        expected_checksum: &str,
    ) -> Result<Box<dyn CxpakPlugin>, Box<dyn std::error::Error>> {
        // Read raw bytes first so we can (a) enforce size limit and (b) verify
        // the checksum BEFORE passing bytes to the wasmtime JIT compiler.
        let bytes = std::fs::read(path)
            .map_err(|e| format!("cannot read plugin at {}: {e}", path.display()))?;

        if bytes.len() as u64 > MAX_PLUGIN_BYTES {
            return Err(format!(
                "plugin too large: {} bytes (max {MAX_PLUGIN_BYTES})",
                bytes.len()
            )
            .into());
        }

        // Integrity check must happen before any wasmtime compilation.
        Self::verify_checksum_bytes(&bytes, expected_checksum)?;

        let module = wasmtime::Module::new(&self.engine, &bytes)
            .map_err(|e| format!("failed to compile WASM module {}: {e}", path.display()))?;

        let mut store = wasmtime::Store::new(
            &self.engine,
            PluginResourceLimiter {
                max_memory: MAX_PLUGIN_MEMORY,
            },
        );
        store.limiter(|state| state as &mut dyn wasmtime::ResourceLimiter);
        store.set_epoch_deadline(EPOCH_DEADLINE);

        let _instance = wasmtime::Instance::new(&mut store, &module, &[])
            .map_err(|e| format!("failed to instantiate WASM module {}: {e}", path.display()))?;
        // Guest function binding is the v2.0.0 skeleton — real binding will be implemented
        // when WIT interface generation is available
        Err(format!(
            "WASM plugin loaded ({} bytes) but guest function binding not yet implemented",
            bytes.len()
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
    use sha2::{Digest, Sha256};
    use std::io::Write;

    /// Compute SHA-256 hex string for a byte slice — used in tests to produce valid checksums.
    fn sha256_hex(data: &[u8]) -> String {
        format!("{:x}", Sha256::digest(data))
    }

    #[test]
    fn load_plugin_non_existent_path_returns_err_with_path() {
        let loader = PluginLoader::new().expect("engine init");
        let path = std::path::Path::new("/nonexistent/plugin.wasm");
        let result = loader.load(path, "deadbeef");
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
        // Checksum doesn't matter — size check happens first.
        let result = loader.load(tmp.path(), "deadbeef");
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

    /// A WASM module whose byte contents don't match the expected checksum is
    /// rejected BEFORE any `wasmtime::Module::new` call.
    #[test]
    fn load_rejects_wrong_checksum_before_compilation() {
        // Minimal valid WASM (would compile fine).
        let minimal_wasm: &[u8] = &[
            0x00, 0x61, 0x73, 0x6d, // magic: \0asm
            0x01, 0x00, 0x00, 0x00, // version: 1
        ];
        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        tmp.write_all(minimal_wasm).expect("write wasm");
        tmp.flush().expect("flush");

        let loader = PluginLoader::new().expect("engine init");
        // Deliberately wrong checksum.
        let result = loader.load(
            tmp.path(),
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
        assert!(result.is_err(), "wrong checksum must return Err");
        let msg = result.err().expect("is err").to_string();
        assert!(
            msg.contains("checksum mismatch"),
            "error should mention 'checksum mismatch', got: {msg}"
        );
    }

    /// Loading a valid (but trivial) WASM module with the correct checksum should
    /// fail with an error that mentions "guest function binding", confirming that
    /// module compilation succeeds but the v2.0.0 stub rejects the call.
    #[test]
    fn load_valid_wasm_with_correct_checksum_returns_guest_function_binding_err() {
        // Minimal valid WASM module: magic header + version (8 bytes).
        let minimal_wasm: &[u8] = &[
            0x00, 0x61, 0x73, 0x6d, // magic: \0asm
            0x01, 0x00, 0x00, 0x00, // version: 1
        ];
        let checksum = sha256_hex(minimal_wasm);

        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        tmp.write_all(minimal_wasm).expect("write wasm");
        tmp.flush().expect("flush");

        let loader = PluginLoader::new().expect("engine init");
        let result = loader.load(tmp.path(), &checksum);
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

    /// verify_checksum_bytes returns Ok for a matching hash.
    #[test]
    fn verify_checksum_bytes_correct_hash_succeeds() {
        let data = b"hello plugin";
        let hash = sha256_hex(data);
        PluginLoader::verify_checksum_bytes(data, &hash).expect("should succeed");
    }

    /// verify_checksum_bytes returns Err for a non-matching hash.
    #[test]
    fn verify_checksum_bytes_wrong_hash_fails() {
        let data = b"hello plugin";
        let result = PluginLoader::verify_checksum_bytes(
            data,
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("checksum mismatch"), "got: {msg}");
    }
}
