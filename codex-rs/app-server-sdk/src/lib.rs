//! Thin production SDK for embedding the Codex app-server runtime.
//!
//! This crate intentionally delegates to `codex_app_server::in_process`.
//! It does not duplicate JSON-RPC routing, `MessageProcessor`, event delivery,
//! approval, sandbox, or tool execution semantics.

pub use codex_app_server::in_process::InProcessClientHandle;
pub use codex_app_server::in_process::InProcessStartArgs;

use codex_runtime_api::RuntimeRegistry;
use std::io::Result as IoResult;

/// Builder for starting an in-process Codex app-server with optional runtime extensions.
///
/// The builder owns the same [`InProcessStartArgs`] used by the stock
/// app-server in-process host. Layer 2 embedders can replace only the
/// [`RuntimeRegistry`] while leaving the existing app-server startup path in
/// charge of `MessageProcessor`, thread lifecycle, app-server events, tools,
/// approvals, sandboxing, and shutdown.
pub struct AppServerBuilder {
    args: InProcessStartArgs,
}

impl AppServerBuilder {
    /// Creates a builder from fully resolved in-process app-server startup args.
    ///
    /// Callers are expected to construct these args using the same config,
    /// auth, state, environment, and initialize sources they would pass to
    /// `codex_app_server::in_process::start` directly.
    pub fn new(args: InProcessStartArgs) -> Self {
        Self { args }
    }

    /// Installs the runtime registry used by fork-owned backend extension seams.
    pub fn runtime_registry(mut self, runtime_registry: RuntimeRegistry) -> Self {
        self.args.runtime_registry = runtime_registry;
        self
    }

    /// Returns the final app-server startup args without starting the runtime.
    ///
    /// This is useful for embedders that want one last chance to inspect or
    /// hand off the exact in-process startup contract.
    pub fn into_in_process_start_args(self) -> InProcessStartArgs {
        self.args
    }

    /// Starts the existing in-process Codex app-server runtime.
    pub async fn start(self) -> IoResult<InProcessClientHandle> {
        codex_app_server::in_process::start(self.args).await
    }
}
