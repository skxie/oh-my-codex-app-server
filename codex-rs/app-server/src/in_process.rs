//! In-process app-server runtime host for local embedders.
//!
//! This module runs the existing [`MessageProcessor`] and outbound routing logic
//! on Tokio tasks, but replaces socket/stdio transports with bounded in-memory
//! channels. The intent is to preserve app-server semantics while avoiding a
//! process boundary for CLI surfaces that run in the same process.
//!
//! # Lifecycle
//!
//! 1. Construct runtime state with [`InProcessStartArgs`].
//! 2. Call [`start`], which performs the `initialize` / `initialized` handshake
//!    internally and returns a ready-to-use [`InProcessClientHandle`].
//! 3. Send requests via [`InProcessClientHandle::request`], notifications via
//!    [`InProcessClientHandle::notify`], and consume events via
//!    [`InProcessClientHandle::next_event`].
//! 4. Terminate with [`InProcessClientHandle::shutdown`].
//!
//! # Transport model
//!
//! The runtime is transport-local but not protocol-free. Incoming requests are
//! typed [`ClientRequest`] values, yet responses still come back through the
//! same JSON-RPC result envelope that `MessageProcessor` uses for stdio and
//! websocket transports. This keeps in-process behavior aligned with
//! app-server rather than creating a second execution contract.
//!
//! # Backpressure
//!
//! Command submission uses `try_send` and can return `WouldBlock`, while event
//! fanout may drop notifications under saturation. Server requests are never
//! silently abandoned: if they cannot be queued they are failed back into
//! `MessageProcessor` with overload or internal errors so approval flows do
//! not hang indefinitely.
//!
//! # Relationship to `codex-app-server-client`
//!
//! This module provides the low-level runtime handle ([`InProcessClientHandle`]).
//! Higher-level callers (TUI, exec) should go through `codex-app-server-client`,
//! which wraps this module behind a worker task with async request/response
//! helpers, surface-specific startup policy, and bounded shutdown.

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::Entry;
use std::io::Error as IoError;
use std::io::ErrorKind;
use std::io::Result as IoResult;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::analytics_utils::analytics_events_client_from_config;
use crate::config_manager::ConfigManager;
use crate::error_code::OVERLOADED_ERROR_CODE;
use crate::error_code::internal_error;
use crate::error_code::invalid_request;
use crate::message_processor::ConnectionSessionState;
use crate::message_processor::MessageProcessor;
use crate::message_processor::MessageProcessorArgs;
use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::OutgoingEnvelope;
use crate::outgoing_message::OutgoingMessage;
use crate::outgoing_message::OutgoingMessageSender;
use crate::outgoing_message::QueuedOutgoingMessage;
use crate::transport::CHANNEL_CAPACITY;
use crate::transport::OutboundConnectionState;
use crate::transport::route_outgoing_envelope;
use codex_analytics::AppServerRpcTransport;
use codex_app_server_protocol::ClientNotification;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ConfigWarningNotification;
use codex_app_server_protocol::InitializeParams;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::Result;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ServerRequest;
use codex_arg0::Arg0DispatchPaths;
use codex_config::CloudConfigBundleLoader;
use codex_config::LoaderOverrides;
use codex_config::ThreadConfigLoader;
use codex_core::config::Config;
use codex_core::resolve_installation_id;
use codex_exec_server::EnvironmentManager;
use codex_feedback::CodexFeedback;
use codex_login::AuthManager;
use codex_protocol::protocol::SessionSource;
pub use codex_rollout::StateDbHandle;
use codex_runtime_api::RuntimeRegistry;
pub use codex_state::log_db::LogDbLayer;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::time::timeout;
use toml::Value as TomlValue;
use tracing::warn;

const IN_PROCESS_CONNECTION_ID: ConnectionId = ConnectionId(0);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
/// Default bounded channel capacity for in-process runtime queues.
pub const DEFAULT_IN_PROCESS_CHANNEL_CAPACITY: usize = CHANNEL_CAPACITY;

type PendingClientRequestResponse = std::result::Result<Result, JSONRPCErrorError>;

fn server_notification_requires_delivery(notification: &ServerNotification) -> bool {
    matches!(
        notification,
        ServerNotification::TurnCompleted(_)
            | ServerNotification::ThreadSettingsUpdated(_)
            | ServerNotification::ExternalAgentConfigImportCompleted(_)
    )
}

/// Input needed to start an in-process app-server runtime.
///
/// These fields mirror the pieces of ambient process state that stdio and
/// websocket transports normally assemble before `MessageProcessor` starts.
#[derive(Clone)]
pub struct InProcessStartArgs {
    /// Resolved argv0 dispatch paths used by command execution internals.
    pub arg0_paths: Arg0DispatchPaths,
    /// Shared base config used to initialize core components.
    pub config: Arc<Config>,
    /// CLI config overrides that are already parsed into TOML values.
    pub cli_overrides: Vec<(String, TomlValue)>,
    /// Loader override knobs used by config API paths.
    pub loader_overrides: LoaderOverrides,
    /// Whether config API paths should reject unknown config fields.
    pub strict_config: bool,
    /// Preloaded cloud config bundle provider.
    pub cloud_config_bundle: CloudConfigBundleLoader,
    /// Loader used to fetch typed thread config sources before a thread starts.
    pub thread_config_loader: Arc<dyn ThreadConfigLoader>,
    /// Feedback sink used by app-server/core telemetry and logs.
    pub feedback: CodexFeedback,
    /// SQLite tracing layer used to flush recently emitted logs before feedback upload.
    pub log_db: Option<LogDbLayer>,
    /// Process-wide SQLite state handle shared with embedded app-server consumers.
    pub state_db: Option<StateDbHandle>,
    /// Runtime control-plane registry for fork-owned backend extension seams.
    pub runtime_registry: RuntimeRegistry,
    /// Environment manager used by core execution and filesystem operations.
    pub environment_manager: Arc<EnvironmentManager>,
    /// Startup warnings emitted after initialize succeeds.
    pub config_warnings: Vec<ConfigWarningNotification>,
    /// Session source stamped into thread/session metadata.
    pub session_source: SessionSource,
    /// Whether auth loading should honor the `CODEX_API_KEY` environment variable.
    pub enable_codex_api_key_env: bool,
    /// Initialize params used for initial handshake.
    pub initialize: InitializeParams,
    /// Capacity used for all runtime queues (clamped to at least 1).
    pub channel_capacity: usize,
}

/// Event emitted from the app-server to the in-process client.
///
/// [`Lagged`](Self::Lagged) is a transport health marker, not an application
/// event — it signals that the consumer fell behind and some events were dropped.
#[derive(Debug, Clone)]
pub enum InProcessServerEvent {
    /// Server request that requires client response/rejection.
    ServerRequest(ServerRequest),
    /// App-server notification directed to the embedded client.
    ServerNotification(ServerNotification),
    /// Indicates one or more events were dropped due to backpressure.
    Lagged { skipped: usize },
}

/// Internal message sent from [`InProcessClientHandle`] methods to the runtime task.
///
/// Requests carry a oneshot sender for the response; notifications and server-request
/// replies are fire-and-forget from the caller's perspective (transport errors are
/// caught by `try_send` on the outer channel).
enum InProcessClientMessage {
    Request {
        request: Box<ClientRequest>,
        response_tx: oneshot::Sender<PendingClientRequestResponse>,
    },
    Notification {
        notification: ClientNotification,
    },
    ServerRequestResponse {
        request_id: RequestId,
        result: Result,
    },
    ServerRequestError {
        request_id: RequestId,
        error: JSONRPCErrorError,
    },
    Shutdown {
        done_tx: oneshot::Sender<()>,
    },
}

enum ProcessorCommand {
    Request(Box<ClientRequest>),
    Notification(ClientNotification),
}

#[derive(Clone)]
pub struct InProcessClientSender {
    client_tx: mpsc::Sender<InProcessClientMessage>,
}

impl InProcessClientSender {
    pub async fn request(&self, request: ClientRequest) -> IoResult<PendingClientRequestResponse> {
        let (response_tx, response_rx) = oneshot::channel();
        self.try_send_client_message(InProcessClientMessage::Request {
            request: Box::new(request),
            response_tx,
        })?;
        response_rx.await.map_err(|err| {
            IoError::new(
                ErrorKind::BrokenPipe,
                format!("in-process request response channel closed: {err}"),
            )
        })
    }

    pub fn notify(&self, notification: ClientNotification) -> IoResult<()> {
        self.try_send_client_message(InProcessClientMessage::Notification { notification })
    }

    pub fn respond_to_server_request(&self, request_id: RequestId, result: Result) -> IoResult<()> {
        self.try_send_client_message(InProcessClientMessage::ServerRequestResponse {
            request_id,
            result,
        })
    }

    pub fn fail_server_request(
        &self,
        request_id: RequestId,
        error: JSONRPCErrorError,
    ) -> IoResult<()> {
        self.try_send_client_message(InProcessClientMessage::ServerRequestError {
            request_id,
            error,
        })
    }

    fn try_send_client_message(&self, message: InProcessClientMessage) -> IoResult<()> {
        match self.client_tx.try_send(message) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err(IoError::new(
                ErrorKind::WouldBlock,
                "in-process app-server client queue is full",
            )),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(IoError::new(
                ErrorKind::BrokenPipe,
                "in-process app-server runtime is closed",
            )),
        }
    }
}

/// Handle used by an in-process client to call app-server and consume events.
///
/// This is the low-level runtime handle. Higher-level callers should usually go
/// through `codex-app-server-client`, which adds worker-task buffering,
/// request/response helpers, and surface-specific startup policy.
pub struct InProcessClientHandle {
    client: InProcessClientSender,
    event_rx: mpsc::Receiver<InProcessServerEvent>,
    runtime_handle: tokio::task::JoinHandle<()>,
    #[cfg(test)]
    _test_codex_home: Option<tempfile::TempDir>,
}

impl InProcessClientHandle {
    /// Sends a typed client request into the in-process runtime.
    ///
    /// The returned value is a transport-level `IoResult` containing either a
    /// JSON-RPC success payload or JSON-RPC error payload. Callers must keep
    /// request IDs unique among concurrent requests; reusing an in-flight ID
    /// produces an `INVALID_REQUEST` response and can make request routing
    /// ambiguous in the caller.
    pub async fn request(&self, request: ClientRequest) -> IoResult<PendingClientRequestResponse> {
        self.client.request(request).await
    }

    /// Sends a typed client notification into the in-process runtime.
    ///
    /// Notifications do not have an application-level response. Transport
    /// errors indicate queue saturation or closed runtime.
    pub fn notify(&self, notification: ClientNotification) -> IoResult<()> {
        self.client.notify(notification)
    }

    /// Resolves a pending [`ServerRequest`](InProcessServerEvent::ServerRequest).
    ///
    /// This should be used only with request IDs received from the current
    /// runtime event stream; sending arbitrary IDs has no effect on app-server
    /// state and can mask a stuck approval flow in the caller.
    pub fn respond_to_server_request(&self, request_id: RequestId, result: Result) -> IoResult<()> {
        self.client.respond_to_server_request(request_id, result)
    }

    /// Rejects a pending [`ServerRequest`](InProcessServerEvent::ServerRequest).
    ///
    /// Use this when the embedder cannot satisfy a server request; leaving
    /// requests unanswered can stall turn progress.
    pub fn fail_server_request(
        &self,
        request_id: RequestId,
        error: JSONRPCErrorError,
    ) -> IoResult<()> {
        self.client.fail_server_request(request_id, error)
    }

    /// Receives the next server event from the in-process runtime.
    ///
    /// Returns `None` when the runtime task exits and no more events are
    /// available.
    pub async fn next_event(&mut self) -> Option<InProcessServerEvent> {
        self.event_rx.recv().await
    }

    /// Requests runtime shutdown and waits for worker termination.
    ///
    /// Shutdown is bounded by internal timeouts and may abort background tasks
    /// if graceful drain does not complete in time.
    pub async fn shutdown(self) -> IoResult<()> {
        let mut runtime_handle = self.runtime_handle;
        let (done_tx, done_rx) = oneshot::channel();

        if self
            .client
            .client_tx
            .send(InProcessClientMessage::Shutdown { done_tx })
            .await
            .is_ok()
        {
            let _ = timeout(SHUTDOWN_TIMEOUT, done_rx).await;
        }

        if let Err(_elapsed) = timeout(SHUTDOWN_TIMEOUT, &mut runtime_handle).await {
            runtime_handle.abort();
            let _ = runtime_handle.await;
        }
        Ok(())
    }

    pub fn sender(&self) -> InProcessClientSender {
        self.client.clone()
    }
}

/// Starts an in-process app-server runtime and performs initialize handshake.
///
/// This function sends `initialize` followed by `initialized` before returning
/// the handle, so callers receive a ready-to-use runtime. If initialize fails,
/// the runtime is shut down and an `InvalidData` error is returned.
pub async fn start(args: InProcessStartArgs) -> IoResult<InProcessClientHandle> {
    let initialize = args.initialize.clone();
    let client = start_uninitialized(args).await?;

    let initialize_response = client
        .request(ClientRequest::Initialize {
            request_id: RequestId::Integer(0),
            params: initialize,
        })
        .await?;
    if let Err(error) = initialize_response {
        let _ = client.shutdown().await;
        return Err(IoError::new(
            ErrorKind::InvalidData,
            format!("in-process initialize failed: {}", error.message),
        ));
    }
    client.notify(ClientNotification::Initialized)?;

    Ok(client)
}

async fn start_uninitialized(args: InProcessStartArgs) -> IoResult<InProcessClientHandle> {
    let channel_capacity = args.channel_capacity.max(1);
    let installation_id = resolve_installation_id(&args.config.codex_home).await?;
    let (client_tx, mut client_rx) = mpsc::channel::<InProcessClientMessage>(channel_capacity);
    let (event_tx, event_rx) = mpsc::channel::<InProcessServerEvent>(channel_capacity);

    let runtime_handle = tokio::spawn(async move {
        let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<OutgoingEnvelope>(channel_capacity);
        let auth_manager =
            AuthManager::shared_from_config(args.config.as_ref(), args.enable_codex_api_key_env)
                .await;
        let analytics_events_client =
            analytics_events_client_from_config(Arc::clone(&auth_manager), args.config.as_ref());
        let outgoing_message_sender = Arc::new(OutgoingMessageSender::new(
            outgoing_tx,
            analytics_events_client.clone(),
        ));

        let (writer_tx, mut writer_rx) = mpsc::channel::<QueuedOutgoingMessage>(channel_capacity);
        let outbound_initialized = Arc::new(AtomicBool::new(false));
        let outbound_experimental_api_enabled = Arc::new(AtomicBool::new(false));
        let outbound_opted_out_notification_methods = Arc::new(RwLock::new(HashSet::new()));

        let mut outbound_connections = HashMap::<ConnectionId, OutboundConnectionState>::new();
        outbound_connections.insert(
            IN_PROCESS_CONNECTION_ID,
            OutboundConnectionState::new(
                writer_tx,
                Arc::clone(&outbound_initialized),
                Arc::clone(&outbound_experimental_api_enabled),
                Arc::clone(&outbound_opted_out_notification_methods),
                /*disconnect_sender*/ None,
            ),
        );
        let mut outbound_handle = tokio::spawn(async move {
            while let Some(envelope) = outgoing_rx.recv().await {
                route_outgoing_envelope(&mut outbound_connections, envelope).await;
            }
        });

        let processor_outgoing = Arc::clone(&outgoing_message_sender);
        let config_manager = ConfigManager::new(
            args.config.codex_home.to_path_buf(),
            args.cli_overrides,
            args.loader_overrides,
            args.strict_config,
            args.cloud_config_bundle,
            args.arg0_paths.clone(),
            args.thread_config_loader,
        );
        let (processor_tx, mut processor_rx) = mpsc::channel::<ProcessorCommand>(channel_capacity);
        let mut processor_handle = tokio::spawn(async move {
            let processor = Arc::new(MessageProcessor::new(MessageProcessorArgs {
                outgoing: Arc::clone(&processor_outgoing),
                analytics_events_client,
                arg0_paths: args.arg0_paths,
                config: args.config,
                config_manager,
                environment_manager: args.environment_manager,
                feedback: args.feedback,
                log_db: args.log_db,
                state_db: args.state_db,
                runtime_registry: args.runtime_registry,
                config_warnings: args.config_warnings,
                session_source: args.session_source,
                auth_manager,
                installation_id,
                rpc_transport: AppServerRpcTransport::InProcess,
                remote_control_handle: None,
                plugin_startup_tasks: crate::PluginStartupTasks::Start,
            }));
            let mut thread_created_rx = processor.thread_created_receiver();
            let session = Arc::new(ConnectionSessionState::new());
            let mut listen_for_threads = true;

            loop {
                tokio::select! {
                    command = processor_rx.recv() => {
                        match command {
                            Some(ProcessorCommand::Request(request)) => {
                                let was_initialized = session.initialized();
                                processor
                                    .process_client_request(
                                        IN_PROCESS_CONNECTION_ID,
                                        *request,
                                        Arc::clone(&session),
                                        &outbound_initialized,
                                    )
                                    .await;
                                let opted_out_notification_methods_snapshot =
                                    session.opted_out_notification_methods();
                                let experimental_api_enabled =
                                    session.experimental_api_enabled();
                                let is_initialized = session.initialized();
                                if let Ok(mut opted_out_notification_methods) =
                                    outbound_opted_out_notification_methods.write()
                                {
                                    *opted_out_notification_methods =
                                        opted_out_notification_methods_snapshot;
                                } else {
                                    warn!("failed to update outbound opted-out notifications");
                                }
                                outbound_experimental_api_enabled.store(
                                    experimental_api_enabled,
                                    Ordering::Release,
                                );
                                if !was_initialized && is_initialized {
                                    processor.send_initialize_notifications().await;
                                }
                            }
                            Some(ProcessorCommand::Notification(notification)) => {
                                processor.process_client_notification(notification).await;
                            }
                            None => {
                                break;
                            }
                        }
                    }
                    created = thread_created_rx.recv(), if listen_for_threads => {
                        match created {
                            Ok(thread_id) => {
                                let connection_ids = if session.initialized() {
                                    vec![IN_PROCESS_CONNECTION_ID]
                                } else {
                                    Vec::<ConnectionId>::new()
                                };
                                processor
                                    .try_attach_thread_listener(thread_id, connection_ids)
                                    .await;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                warn!("thread_created receiver lagged; skipping resync");
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                listen_for_threads = false;
                            }
                        }
                    }
                }
            }

            processor.clear_runtime_references();
            processor.cancel_active_login().await;
            processor
                .connection_closed(IN_PROCESS_CONNECTION_ID, &session)
                .await;
            processor.clear_all_thread_listeners().await;
            processor.drain_background_tasks().await;
            processor.shutdown_threads().await;
        });
        let mut pending_request_responses =
            HashMap::<RequestId, oneshot::Sender<PendingClientRequestResponse>>::new();
        let mut shutdown_ack = None;

        loop {
            tokio::select! {
                message = client_rx.recv() => {
                    match message {
                        Some(InProcessClientMessage::Request { request, response_tx }) => {
                            let request = *request;
                            let request_id = request.id().clone();
                            match pending_request_responses.entry(request_id.clone()) {
                                Entry::Vacant(entry) => {
                                    entry.insert(response_tx);
                                }
                                Entry::Occupied(_) => {
                                    let _ = response_tx.send(Err(invalid_request(format!(
                                        "duplicate request id: {request_id:?}"
                                    ))));
                                    continue;
                                }
                            }

                            match processor_tx.try_send(ProcessorCommand::Request(Box::new(request))) {
                                Ok(()) => {}
                                Err(mpsc::error::TrySendError::Full(_)) => {
                                    if let Some(response_tx) =
                                        pending_request_responses.remove(&request_id)
                                    {
                                        let _ = response_tx.send(Err(JSONRPCErrorError {
                                            code: OVERLOADED_ERROR_CODE,
                                            message: "in-process app-server request queue is full"
                                                .to_string(),
                                            data: None,
                                        }));
                                    }
                                }
                                Err(mpsc::error::TrySendError::Closed(_)) => {
                                    if let Some(response_tx) =
                                        pending_request_responses.remove(&request_id)
                                    {
                                        let _ = response_tx.send(Err(internal_error(
                                            "in-process app-server request processor is closed",
                                        )));
                                    }
                                    break;
                                }
                            }
                        }
                        Some(InProcessClientMessage::Notification { notification }) => {
                            match processor_tx.try_send(ProcessorCommand::Notification(notification)) {
                                Ok(()) => {}
                                Err(mpsc::error::TrySendError::Full(_)) => {
                                    warn!("dropping in-process client notification (queue full)");
                                }
                                Err(mpsc::error::TrySendError::Closed(_)) => {
                                    break;
                                }
                            }
                        }
                        Some(InProcessClientMessage::ServerRequestResponse { request_id, result }) => {
                            outgoing_message_sender
                                .notify_client_response(request_id, result)
                                .await;
                        }
                        Some(InProcessClientMessage::ServerRequestError { request_id, error }) => {
                            outgoing_message_sender
                                .notify_client_error(request_id, error)
                                .await;
                        }
                        Some(InProcessClientMessage::Shutdown { done_tx }) => {
                            shutdown_ack = Some(done_tx);
                            break;
                        }
                        None => {
                            break;
                        }
                    }
                }
                queued_message = writer_rx.recv() => {
                    let Some(queued_message) = queued_message else {
                        break;
                    };
                    let outgoing_message = queued_message.message;
                    match outgoing_message {
                        OutgoingMessage::Response(response) => {
                            if let Some(response_tx) = pending_request_responses.remove(&response.id) {
                                let _ = response_tx.send(Ok(response.result));
                            } else {
                                warn!(
                                    request_id = ?response.id,
                                    "dropping unmatched in-process response"
                                );
                            }
                        }
                        OutgoingMessage::Error(error) => {
                            if let Some(response_tx) = pending_request_responses.remove(&error.id) {
                                let _ = response_tx.send(Err(error.error));
                            } else {
                                warn!(
                                    request_id = ?error.id,
                                    "dropping unmatched in-process error response"
                                );
                            }
                        }
                        OutgoingMessage::Request(request) => {
                            // Send directly to avoid cloning; on failure the
                            // original value is returned inside the error.
                            if let Err(send_error) = event_tx
                                .try_send(InProcessServerEvent::ServerRequest(request))
                            {
                                let (error, inner) = match send_error {
                                    mpsc::error::TrySendError::Full(inner) => (
                                        JSONRPCErrorError {
                                            code: OVERLOADED_ERROR_CODE,
                                            message:
                                                "in-process server request queue is full".to_string(),
                                            data: None,
                                        },
                                        inner,
                                    ),
                                    mpsc::error::TrySendError::Closed(inner) => (
                                        internal_error(
                                            "in-process server request consumer is closed",
                                        ),
                                        inner,
                                    ),
                                };
                                let request_id = match inner {
                                    InProcessServerEvent::ServerRequest(req) => req.id().clone(),
                                    _ => unreachable!("we just sent a ServerRequest variant"),
                                };
                                outgoing_message_sender
                                    .notify_client_error(request_id, error)
                                    .await;
                            }
                        }
                        OutgoingMessage::AppServerNotification(notification) => {
                            if server_notification_requires_delivery(&notification) {
                                if event_tx
                                    .send(InProcessServerEvent::ServerNotification(notification))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            } else if let Err(send_error) =
                                event_tx.try_send(InProcessServerEvent::ServerNotification(notification))
                            {
                                match send_error {
                                    mpsc::error::TrySendError::Full(_) => {
                                        warn!("dropping in-process server notification (queue full)");
                                    }
                                    mpsc::error::TrySendError::Closed(_) => {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    if let Some(write_complete_tx) = queued_message.write_complete_tx {
                        let _ = write_complete_tx.send(());
                    }
                }
            }
        }

        drop(writer_rx);
        drop(processor_tx);
        outgoing_message_sender
            .cancel_all_requests(Some(internal_error(
                "in-process app-server runtime is shutting down",
            )))
            .await;
        // Drop the runtime's last sender before awaiting the router task so
        // `outgoing_rx.recv()` can observe channel closure and exit cleanly.
        drop(outgoing_message_sender);
        for (_, response_tx) in pending_request_responses {
            let _ = response_tx.send(Err(internal_error(
                "in-process app-server runtime is shutting down",
            )));
        }

        if let Err(_elapsed) = timeout(SHUTDOWN_TIMEOUT, &mut processor_handle).await {
            processor_handle.abort();
            let _ = processor_handle.await;
        }
        if let Err(_elapsed) = timeout(SHUTDOWN_TIMEOUT, &mut outbound_handle).await {
            outbound_handle.abort();
            let _ = outbound_handle.await;
        }

        if let Some(done_tx) = shutdown_ack {
            let _ = done_tx.send(());
        }
    });

    Ok(InProcessClientHandle {
        client: InProcessClientSender { client_tx },
        event_rx,
        runtime_handle,
        #[cfg(test)]
        _test_codex_home: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_app_server_protocol::ClientInfo;
    use codex_app_server_protocol::ConfigRequirementsReadResponse;
    use codex_app_server_protocol::DynamicToolCallOutputContentItem;
    use codex_app_server_protocol::DynamicToolCallParams;
    use codex_app_server_protocol::DynamicToolCallResponse;
    use codex_app_server_protocol::DynamicToolSpec;
    use codex_app_server_protocol::ExternalAgentConfigImportCompletedNotification;
    use codex_app_server_protocol::InitializeCapabilities;
    use codex_app_server_protocol::SessionSource as ApiSessionSource;
    use codex_app_server_protocol::ThreadStartParams;
    use codex_app_server_protocol::ThreadStartResponse;
    use codex_app_server_protocol::ThreadTokenUsageUpdatedNotification;
    use codex_app_server_protocol::Turn;
    use codex_app_server_protocol::TurnCompletedNotification;
    use codex_app_server_protocol::TurnItemsView;
    use codex_app_server_protocol::TurnStartParams;
    use codex_app_server_protocol::TurnStatus;
    use codex_app_server_protocol::UserInput;
    use codex_core::config::ConfigBuilder;
    use codex_runtime_api::ContextAssemblyObserver;
    use codex_runtime_api::ContextAssemblyObserverId;
    use codex_runtime_api::ContextAssemblyObserverInput;
    use codex_runtime_api::ContextBlock;
    use codex_runtime_api::ContextBlockSlot;
    use codex_runtime_api::ContextContributor;
    use codex_runtime_api::ContextContributorId;
    use codex_runtime_api::ContextContributorInput;
    use codex_runtime_api::ContextError;
    use codex_runtime_api::ModelApiKind;
    use codex_runtime_api::ModelApiRequest;
    use codex_runtime_api::ModelRequestAdapter;
    use codex_runtime_api::ModelRequestAdapterError;
    use codex_runtime_api::ModelRequestAdapterId;
    use codex_runtime_api::ModelRequestAdapterInput;
    use codex_runtime_api::ProtocolResponseMapperKind;
    use codex_runtime_api::ToolCall;
    use codex_runtime_api::ToolCallDecision;
    use codex_runtime_api::ToolCallRepairRecord;
    use codex_runtime_api::ToolMiddleware;
    use codex_runtime_api::ToolMiddlewareError;
    use codex_runtime_api::ToolMiddlewareId;
    use codex_runtime_api::ToolResult;
    use codex_runtime_api::ToolResultDecision;
    use codex_runtime_api::UsageMetadata;
    use codex_runtime_api::UsageMetadataMapper;
    use codex_runtime_api::UsageMetadataMapperError;
    use codex_runtime_api::UsageMetadataMapperId;
    use codex_runtime_api::UsageMetadataMapperInput;
    use core_test_support::responses;
    use pretty_assertions::assert_eq;
    use serde_json::Value;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::path::Path;
    use std::sync::Mutex;
    use std::sync::MutexGuard;
    use tempfile::TempDir;

    const RUNTIME_FIXTURE_CONTEXT: &str = "runtime fixture contributed context";
    const RUNTIME_FIXTURE_TOOL_NAME: &str = "runtime_fixture_tool";
    const RUNTIME_FIXTURE_TOOL_CALL_ID: &str = "runtime-tool-call-1";

    struct RuntimeFixtureModelRequestAdapter;

    impl ModelRequestAdapter for RuntimeFixtureModelRequestAdapter {
        fn id(&self) -> ModelRequestAdapterId {
            ModelRequestAdapterId::new("test.runtime_fixture.model_request_adapter")
        }

        async fn build_request(
            &self,
            input: ModelRequestAdapterInput,
        ) -> std::result::Result<ModelApiRequest, ModelRequestAdapterError> {
            let mut body = input.body;
            body.as_object_mut()
                .expect("stock request body should be a JSON object")
                .insert(
                    "client_metadata".to_string(),
                    json!({
                        "source": "runtime-fixture"
                    }),
                );
            Ok(ModelApiRequest {
                api_kind: ModelApiKind::Responses,
                endpoint_path: "responses".to_string(),
                body,
                response_mapper: ProtocolResponseMapperKind::Responses,
            })
        }
    }

    struct RuntimeFixtureContextContributor;

    impl ContextContributor for RuntimeFixtureContextContributor {
        fn id(&self) -> ContextContributorId {
            ContextContributorId::new("test.runtime_fixture.context_contributor")
        }

        async fn contribute(
            &self,
            _input: ContextContributorInput,
        ) -> std::result::Result<Vec<ContextBlock>, ContextError> {
            Ok(vec![ContextBlock {
                id: "fixture-context".to_string(),
                slot: ContextBlockSlot::ContextualUser,
                content: RUNTIME_FIXTURE_CONTEXT.to_string(),
                source: "runtime-fixture".to_string(),
                metadata: BTreeMap::new(),
            }])
        }
    }

    #[derive(Clone, Default)]
    struct RuntimeFixtureContextObserver {
        observed: Arc<Mutex<Vec<Value>>>,
    }

    impl ContextAssemblyObserver for RuntimeFixtureContextObserver {
        fn id(&self) -> ContextAssemblyObserverId {
            ContextAssemblyObserverId::new("test.runtime_fixture.context_observer")
        }

        async fn observe(
            &self,
            input: ContextAssemblyObserverInput,
        ) -> std::result::Result<(), ContextError> {
            self.observed
                .lock()
                .expect("observer lock should not be poisoned")
                .push(input.provider_bound_input);
            Ok(())
        }
    }

    struct RuntimeFixtureUsageMetadataMapper;

    impl UsageMetadataMapper for RuntimeFixtureUsageMetadataMapper {
        fn id(&self) -> UsageMetadataMapperId {
            UsageMetadataMapperId::new("test.runtime_fixture.usage_metadata_mapper")
        }

        async fn map_usage_metadata(
            &self,
            input: UsageMetadataMapperInput,
        ) -> std::result::Result<Option<UsageMetadata>, UsageMetadataMapperError> {
            let Some(usage) = input
                .raw_provider_metadata
                .values
                .get("response.metadata")
                .and_then(|metadata| metadata.get("runtime_fixture_usage"))
            else {
                return Ok(input.fallback_usage);
            };
            Ok(Some(UsageMetadata {
                prompt_tokens: usage["prompt_tokens"]
                    .as_u64()
                    .expect("prompt_tokens should be u64"),
                completion_tokens: usage["completion_tokens"]
                    .as_u64()
                    .expect("completion_tokens should be u64"),
                cached_prompt_tokens: Some(
                    usage["cached_prompt_tokens"]
                        .as_u64()
                        .expect("cached_prompt_tokens should be u64"),
                ),
                cache_miss_prompt_tokens: Some(
                    usage["cache_miss_prompt_tokens"]
                        .as_u64()
                        .expect("cache_miss_prompt_tokens should be u64"),
                ),
                reasoning_tokens: Some(
                    usage["reasoning_tokens"]
                        .as_u64()
                        .expect("reasoning_tokens should be u64"),
                ),
            }))
        }
    }

    #[derive(Clone, Default)]
    struct RuntimeFixtureToolMiddleware {
        repairs: Arc<Mutex<Vec<ToolCallRepairRecord>>>,
    }

    impl RuntimeFixtureToolMiddleware {
        fn repairs(&self) -> MutexGuard<'_, Vec<ToolCallRepairRecord>> {
            self.repairs
                .lock()
                .expect("tool middleware repair lock should not be poisoned")
        }
    }

    impl ToolMiddleware for RuntimeFixtureToolMiddleware {
        fn id(&self) -> ToolMiddlewareId {
            ToolMiddlewareId::new("test.runtime_fixture.tool_middleware")
        }

        async fn before_tool_call(
            &self,
            call: ToolCall,
        ) -> std::result::Result<ToolCallDecision, ToolMiddlewareError> {
            let decision = if call.call_id == RUNTIME_FIXTURE_TOOL_CALL_ID {
                ToolCallDecision::Repair {
                    repaired_arguments: json!({
                        "city": "Paris",
                        "repaired": true
                    }),
                }
            } else {
                ToolCallDecision::Continue
            };
            if let Some(repair) = decision.repair_record(&call) {
                self.repairs().push(repair);
            }
            Ok(decision)
        }

        async fn after_tool_call(
            &self,
            _call: ToolCall,
            _result: ToolResult,
        ) -> std::result::Result<ToolResultDecision, ToolMiddlewareError> {
            Ok(ToolResultDecision::Preserve)
        }
    }

    async fn build_test_config(codex_home: &Path) -> Config {
        match ConfigBuilder::default()
            .codex_home(codex_home.to_path_buf())
            .build()
            .await
        {
            Ok(config) => config,
            Err(_) => Config::load_default_with_cli_overrides_for_codex_home(
                codex_home.to_path_buf(),
                Vec::new(),
            )
            .await
            .expect("default config should load"),
        }
    }

    fn write_runtime_fixture_config(codex_home: &Path, server_uri: &str) {
        std::fs::write(
            codex_home.join("config.toml"),
            format!(
                r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "read-only"
model_provider = "mock_provider"

[model_providers.mock_provider]
name = "Mock provider for runtime fixture"
base_url = "{server_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
"#
            ),
        )
        .expect("runtime fixture config should be written");
    }

    async fn start_test_client_with_capacity(
        session_source: SessionSource,
        channel_capacity: usize,
    ) -> InProcessClientHandle {
        let codex_home = TempDir::new().expect("temp dir");
        let config = Arc::new(build_test_config(codex_home.path()).await);
        let state_db = codex_rollout::state_db::try_init(config.as_ref())
            .await
            .expect("state db should initialize for in-process test");
        let args = InProcessStartArgs {
            arg0_paths: Arg0DispatchPaths::default(),
            config,
            cli_overrides: Vec::new(),
            loader_overrides: LoaderOverrides::default(),
            strict_config: false,
            cloud_config_bundle: CloudConfigBundleLoader::default(),
            thread_config_loader: Arc::new(codex_config::NoopThreadConfigLoader),
            feedback: CodexFeedback::new(),
            log_db: None,
            state_db: Some(state_db),
            runtime_registry: RuntimeRegistry::default(),
            environment_manager: Arc::new(EnvironmentManager::default_for_tests()),
            config_warnings: Vec::new(),
            session_source,
            enable_codex_api_key_env: false,
            initialize: InitializeParams {
                client_info: ClientInfo {
                    name: "codex-in-process-test".to_string(),
                    title: None,
                    version: "0.0.0".to_string(),
                },
                capabilities: None,
            },
            channel_capacity,
        };
        let mut client = start(args).await.expect("in-process runtime should start");
        client._test_codex_home = Some(codex_home);
        client
    }

    async fn start_test_client_with_runtime_registry(
        runtime_registry: RuntimeRegistry,
        codex_home: TempDir,
    ) -> InProcessClientHandle {
        let config = Arc::new(build_test_config(codex_home.path()).await);
        let state_db = codex_rollout::state_db::try_init(config.as_ref())
            .await
            .expect("state db should initialize for in-process test");
        let args = InProcessStartArgs {
            arg0_paths: Arg0DispatchPaths::default(),
            config,
            cli_overrides: Vec::new(),
            loader_overrides: LoaderOverrides::default(),
            strict_config: false,
            cloud_config_bundle: CloudConfigBundleLoader::default(),
            thread_config_loader: Arc::new(codex_config::NoopThreadConfigLoader),
            feedback: CodexFeedback::new(),
            log_db: None,
            state_db: Some(state_db),
            runtime_registry,
            environment_manager: Arc::new(EnvironmentManager::default_for_tests()),
            config_warnings: Vec::new(),
            session_source: SessionSource::Cli,
            enable_codex_api_key_env: false,
            initialize: InitializeParams {
                client_info: ClientInfo {
                    name: "codex-in-process-runtime-fixture-test".to_string(),
                    title: None,
                    version: "0.0.0".to_string(),
                },
                capabilities: Some(InitializeCapabilities {
                    experimental_api: true,
                    request_attestation: false,
                    opt_out_notification_methods: None,
                }),
            },
            channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
        };
        let mut client = start(args).await.expect("in-process runtime should start");
        client._test_codex_home = Some(codex_home);
        client
    }

    async fn start_test_client(session_source: SessionSource) -> InProcessClientHandle {
        start_test_client_with_capacity(session_source, DEFAULT_IN_PROCESS_CHANNEL_CAPACITY).await
    }

    async fn read_until_runtime_fixture_events(
        client: &mut InProcessClientHandle,
    ) -> (
        ThreadTokenUsageUpdatedNotification,
        TurnCompletedNotification,
    ) {
        let mut usage = None;
        let mut completed = None;

        while usage.is_none() || completed.is_none() {
            let event = timeout(Duration::from_secs(60), client.next_event())
                .await
                .expect("runtime fixture should emit next event")
                .expect("runtime fixture event stream should stay open");
            if let InProcessServerEvent::ServerNotification(notification) = event {
                match notification {
                    ServerNotification::ThreadTokenUsageUpdated(next_usage) => {
                        usage = Some(next_usage);
                    }
                    ServerNotification::TurnCompleted(next_completed) => {
                        completed = Some(next_completed);
                    }
                    _ => {}
                }
            }
        }

        (
            usage.expect("usage notification should be observed"),
            completed.expect("turn completed notification should be observed"),
        )
    }

    fn value_contains_text(value: &Value, expected: &str) -> bool {
        match value {
            Value::String(text) => text.contains(expected),
            Value::Array(items) => items.iter().any(|item| value_contains_text(item, expected)),
            Value::Object(map) => map.values().any(|item| value_contains_text(item, expected)),
            Value::Null | Value::Bool(_) | Value::Number(_) => false,
        }
    }

    fn write_fixture_proof_line(line: &str) {
        let mut stderr = std::io::stderr().lock();
        writeln!(stderr, "{line}").expect("runtime fixture proof output should write to stderr");
    }

    async fn read_until_runtime_fixture_tool_request(
        client: &mut InProcessClientHandle,
    ) -> (RequestId, DynamicToolCallParams) {
        loop {
            let event = timeout(Duration::from_secs(60), client.next_event())
                .await
                .expect("runtime fixture should emit dynamic tool request")
                .expect("runtime fixture event stream should stay open");
            if let InProcessServerEvent::ServerRequest(ServerRequest::DynamicToolCall {
                request_id,
                params,
            }) = event
            {
                return (request_id, params);
            }
        }
    }

    #[tokio::test]
    async fn in_process_start_initializes_and_handles_typed_v2_request() {
        let client = start_test_client(SessionSource::Cli).await;
        let response = client
            .request(ClientRequest::ConfigRequirementsRead {
                request_id: RequestId::Integer(1),
                params: None,
            })
            .await
            .expect("request transport should work")
            .expect("request should succeed");
        assert!(response.is_object());

        let _parsed: ConfigRequirementsReadResponse =
            serde_json::from_value(response).expect("response should match v2 schema");
        client
            .shutdown()
            .await
            .expect("in-process runtime should shutdown cleanly");
    }

    #[tokio::test]
    async fn runtime_registry_fake_backend_fixture_takes_effect_through_in_process_app_server() {
        let server = responses::start_mock_server().await;
        let first_response = responses::sse(vec![
            responses::ev_response_created("resp-1"),
            responses::ev_function_call(
                RUNTIME_FIXTURE_TOOL_CALL_ID,
                RUNTIME_FIXTURE_TOOL_NAME,
                "{malformed",
            ),
            responses::ev_completed("resp-1"),
        ]);
        let second_response = responses::sse(vec![
            responses::ev_response_created("resp-2"),
            responses::ev_assistant_message("msg-1", "Done"),
            json!({
                "type": "response.completed",
                "response": {
                    "id": "resp-2",
                    "usage": {
                        "input_tokens": 7,
                        "input_tokens_details": null,
                        "output_tokens": 0,
                        "output_tokens_details": null,
                        "total_tokens": 7
                    },
                    "metadata": {
                        "runtime_fixture_usage": {
                            "prompt_tokens": 11,
                            "completion_tokens": 22,
                            "cached_prompt_tokens": 3,
                            "cache_miss_prompt_tokens": 8,
                            "reasoning_tokens": 5
                        }
                    }
                }
            }),
        ]);
        let response_mock =
            responses::mount_sse_sequence(&server, vec![first_response, second_response]).await;

        let codex_home = TempDir::new().expect("temp dir");
        write_runtime_fixture_config(codex_home.path(), &server.uri());

        let observer = RuntimeFixtureContextObserver::default();
        let observed = Arc::clone(&observer.observed);
        let middleware = RuntimeFixtureToolMiddleware::default();
        let repairs = Arc::clone(&middleware.repairs);
        let mut registry = RuntimeRegistry::builder();
        registry
            .model_request_adapter(RuntimeFixtureModelRequestAdapter)
            .expect("model request adapter should register")
            .context_contributor(RuntimeFixtureContextContributor)
            .expect("context contributor should register")
            .context_assembly_observer(observer)
            .expect("context observer should register")
            .tool_middleware(middleware)
            .expect("tool middleware should register")
            .usage_metadata_mapper(RuntimeFixtureUsageMetadataMapper)
            .expect("usage mapper should register");
        let mut client =
            start_test_client_with_runtime_registry(registry.build(), codex_home).await;

        let thread_response = client
            .request(ClientRequest::ThreadStart {
                request_id: RequestId::Integer(10),
                params: ThreadStartParams {
                    model: Some("mock-model".to_string()),
                    dynamic_tools: Some(vec![DynamicToolSpec {
                        namespace: None,
                        name: RUNTIME_FIXTURE_TOOL_NAME.to_string(),
                        description: "Runtime fixture dynamic tool".to_string(),
                        input_schema: json!({
                            "type": "object",
                            "properties": {
                                "city": { "type": "string" },
                                "repaired": { "type": "boolean" }
                            },
                            "required": ["city", "repaired"],
                            "additionalProperties": false
                        }),
                        defer_loading: false,
                    }]),
                    ..ThreadStartParams::default()
                },
            })
            .await
            .expect("thread/start transport should work")
            .expect("thread/start should succeed");
        let thread: ThreadStartResponse =
            serde_json::from_value(thread_response).expect("thread/start should parse");

        client
            .request(ClientRequest::TurnStart {
                request_id: RequestId::Integer(11),
                params: TurnStartParams {
                    thread_id: thread.thread.id.clone(),
                    input: vec![UserInput::Text {
                        text: "Hello runtime fixture".to_string(),
                        text_elements: Vec::new(),
                    }],
                    ..TurnStartParams::default()
                },
            })
            .await
            .expect("turn/start transport should work")
            .expect("turn/start should succeed");

        let (tool_request_id, tool_params) =
            read_until_runtime_fixture_tool_request(&mut client).await;
        assert_eq!(tool_params.call_id, RUNTIME_FIXTURE_TOOL_CALL_ID);
        assert_eq!(tool_params.tool, RUNTIME_FIXTURE_TOOL_NAME);
        assert_eq!(
            tool_params.arguments,
            json!({
                "city": "Paris",
                "repaired": true
            })
        );

        let repair_records = repairs
            .lock()
            .expect("repair record lock should not be poisoned")
            .clone();
        assert_eq!(
            repair_records,
            vec![ToolCallRepairRecord {
                call_id: RUNTIME_FIXTURE_TOOL_CALL_ID.to_string(),
                tool_name: RUNTIME_FIXTURE_TOOL_NAME.to_string(),
                original_arguments: Value::String("{malformed".to_string()),
                repaired_arguments: json!({
                    "city": "Paris",
                    "repaired": true
                }),
            }]
        );

        client
            .respond_to_server_request(
                tool_request_id,
                serde_json::to_value(DynamicToolCallResponse {
                    content_items: vec![DynamicToolCallOutputContentItem::InputText {
                        text: "dynamic-ok".to_string(),
                    }],
                    success: true,
                })
                .expect("dynamic tool response should serialize"),
            )
            .expect("dynamic tool response should be sent");

        let (usage, completed) = read_until_runtime_fixture_events(&mut client).await;
        assert_eq!(completed.thread_id, thread.thread.id);
        assert_eq!(completed.turn.status, TurnStatus::Completed);
        assert_eq!(usage.token_usage.last.total_tokens, 38);
        assert_eq!(usage.token_usage.last.input_tokens, 11);
        assert_eq!(usage.token_usage.last.cached_input_tokens, 3);
        assert_eq!(usage.token_usage.last.output_tokens, 22);
        assert_eq!(usage.token_usage.last.reasoning_output_tokens, 5);

        let requests = response_mock.requests();
        assert_eq!(requests.len(), 2);
        let body = requests[0].body_json();
        assert_eq!(
            body["client_metadata"],
            json!({
                "source": "runtime-fixture"
            })
        );
        assert!(
            value_contains_text(&body["input"], RUNTIME_FIXTURE_CONTEXT),
            "runtime contributor context should reach the provider-bound request"
        );
        let tool_output = requests[1].function_call_output(RUNTIME_FIXTURE_TOOL_CALL_ID);
        assert_eq!(
            tool_output["call_id"],
            Value::String(RUNTIME_FIXTURE_TOOL_CALL_ID.to_string())
        );
        assert!(
            value_contains_text(&tool_output, "dynamic-ok"),
            "runtime repaired tool response should be sent back to the model"
        );

        let observed = observed
            .lock()
            .expect("observer lock should not be poisoned")
            .clone();
        assert_eq!(observed.len(), 2);
        assert!(
            observed
                .iter()
                .any(|input| value_contains_text(input, RUNTIME_FIXTURE_CONTEXT)),
            "runtime observer should see the provider-bound input"
        );

        write_fixture_proof_line("fake request adapter: apiKind=responses bodyChanged=true");
        write_fixture_proof_line("fake context contributor: stablePrefixFound=true");
        write_fixture_proof_line("context observer: providerBoundInputCaptured=true");
        write_fixture_proof_line(
            "fake tool middleware: repairedArgsApplied=true callIdentityPreserved=true",
        );
        write_fixture_proof_line(
            "usage metadata mapper: rawProviderMetadataConsumed=true promptTokens=11 cachedPromptTokens=3 reasoningTokens=5",
        );
        write_fixture_proof_line(
            "app-server events: thread/start turn/start tokenUsage/updated turn/completed",
        );

        client
            .shutdown()
            .await
            .expect("in-process runtime should shutdown cleanly");
    }

    #[tokio::test]
    async fn in_process_start_uses_requested_session_source_for_thread_start() {
        for (requested_source, expected_source) in [
            (SessionSource::Cli, ApiSessionSource::Cli),
            (SessionSource::Exec, ApiSessionSource::Exec),
        ] {
            let client = start_test_client(requested_source).await;
            let response = client
                .request(ClientRequest::ThreadStart {
                    request_id: RequestId::Integer(2),
                    params: ThreadStartParams {
                        ephemeral: Some(true),
                        ..ThreadStartParams::default()
                    },
                })
                .await
                .expect("request transport should work")
                .expect("thread/start should succeed");
            let parsed: ThreadStartResponse =
                serde_json::from_value(response).expect("thread/start response should parse");
            assert_eq!(parsed.thread.source, expected_source);
            client
                .shutdown()
                .await
                .expect("in-process runtime should shutdown cleanly");
        }
    }

    #[tokio::test]
    async fn in_process_start_clamps_zero_channel_capacity() {
        let client =
            start_test_client_with_capacity(SessionSource::Cli, /*channel_capacity*/ 0).await;
        let response = loop {
            match client
                .request(ClientRequest::ConfigRequirementsRead {
                    request_id: RequestId::Integer(4),
                    params: None,
                })
                .await
            {
                Ok(response) => break response.expect("request should succeed"),
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    tokio::task::yield_now().await;
                }
                Err(err) => panic!("request transport should work: {err}"),
            }
        };
        let _parsed: ConfigRequirementsReadResponse =
            serde_json::from_value(response).expect("response should match v2 schema");
        client
            .shutdown()
            .await
            .expect("in-process runtime should shutdown cleanly");
    }

    #[test]
    fn guaranteed_delivery_helpers_cover_terminal_server_notifications() {
        assert!(server_notification_requires_delivery(
            &ServerNotification::TurnCompleted(TurnCompletedNotification {
                thread_id: "thread-1".to_string(),
                turn: Turn {
                    id: "turn-1".to_string(),
                    items: Vec::new(),
                    items_view: TurnItemsView::NotLoaded,
                    status: TurnStatus::Completed,
                    error: None,
                    started_at: None,
                    completed_at: Some(0),
                    duration_ms: None,
                },
            })
        ));
        assert!(server_notification_requires_delivery(
            &ServerNotification::ExternalAgentConfigImportCompleted(
                ExternalAgentConfigImportCompletedNotification {
                    import_id: "import".to_string(),
                    item_type_results: Vec::new(),
                },
            )
        ));
    }
}
