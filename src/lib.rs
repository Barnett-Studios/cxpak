pub mod auto_context;
pub mod budget;
pub mod cache;
pub mod cli;
pub mod commands;
pub mod context_quality;
pub mod conventions;
#[cfg(feature = "daemon")]
pub mod daemon;
pub mod dev_maintenance;
#[cfg(feature = "embeddings")]
pub mod embeddings;
pub mod git;
pub mod index;
pub mod intelligence;
#[cfg(feature = "lsp")]
pub mod lsp;
pub mod output;
pub mod parser;
pub mod plugin;
pub mod relevance;
pub mod scanner;
pub mod schema;
/// Shared test scaffolding (index builder + SPA fixture). Present only in test
/// builds — `cfg(test)` for unit tests, the `test-support` feature (enabled via
/// the self dev-dependency) for integration tests. Never in a release binary.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod util;
#[cfg(feature = "visual")]
pub mod visual;
