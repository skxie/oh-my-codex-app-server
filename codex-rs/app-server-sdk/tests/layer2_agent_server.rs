use anyhow::Context as _;
use anyhow::anyhow;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ConfigRequirementsReadResponse;
use codex_app_server_protocol::DynamicToolCallOutputContentItem;
use codex_app_server_protocol::DynamicToolCallParams;
use codex_app_server_protocol::DynamicToolCallResponse;
use codex_app_server_protocol::DynamicToolSpec;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ServerRequest;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::ThreadTokenUsageUpdatedNotification;
use codex_app_server_protocol::TurnCompletedNotification;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStatus;
use codex_app_server_protocol::UserInput;
use codex_app_server_sdk::AppServerBuilder;
use codex_app_server_sdk::AppServerClient;
use codex_app_server_sdk::AppServerEvent;
use codex_app_server_sdk::InProcessClientStartArgs;
use codex_arg0::Arg0DispatchPaths;
use codex_config::CloudConfigBundleLoader;
use codex_config::LoaderOverrides;
use codex_core::config::Config;
use codex_core::config::ConfigBuilder;
use codex_exec_server::EnvironmentManager;
use codex_feedback::CodexFeedback;
use codex_protocol::protocol::SessionSource;
use codex_runtime_api::ContextAssemblyObserver;
use codex_runtime_api::ContextAssemblyObserverId;
use codex_runtime_api::ContextAssemblyObserverInput;
use codex_runtime_api::ContextBlock;
use codex_runtime_api::ContextBlockSlot;
use codex_runtime_api::ContextContributor;
use codex_runtime_api::ContextContributorId;
use codex_runtime_api::ContextContributorInput;
use codex_runtime_api::ContextError;
use codex_runtime_api::ModelApiRequest;
use codex_runtime_api::ModelRequestAdapter;
use codex_runtime_api::ModelRequestAdapterError;
use codex_runtime_api::ModelRequestAdapterId;
use codex_runtime_api::ModelRequestAdapterInput;
use codex_runtime_api::RuntimeCapability;
use codex_runtime_api::RuntimeExtensionErrorInfo;
use codex_runtime_api::RuntimeExtensionPhase;
use codex_runtime_api::RuntimeRegistry;
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
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use tempfile::TempDir;
use tokio::time::Duration;
use tokio::time::timeout;

const LAYER2_CONTEXT: &str = "layer2-sdk-context: stable prefix from custom backend";
const LAYER2_TOOL_NAME: &str = "layer2_lookup";
const LAYER2_TOOL_CALL_ID: &str = "call-layer2";

fn runtime_test_error(
    capability: RuntimeCapability,
    contributor_id: &str,
    phase: RuntimeExtensionPhase,
    what_happened: &str,
) -> RuntimeExtensionErrorInfo {
    RuntimeExtensionErrorInfo::new(
        capability,
        contributor_id,
        phase,
        what_happened,
        "the SDK fixture shared state lock was poisoned",
        "rerun the test and inspect prior panic output",
        Some("sdk-layer2-agent-server-fixture"),
    )
}

struct Layer2ModelRequestAdapter;

impl ModelRequestAdapter for Layer2ModelRequestAdapter {
    fn id(&self) -> ModelRequestAdapterId {
        ModelRequestAdapterId::new("layer2.test.model_request_adapter")
    }

    async fn build_request(
        &self,
        input: ModelRequestAdapterInput,
    ) -> Result<ModelApiRequest, ModelRequestAdapterError> {
        let mut body = input.body;
        body["client_metadata"] = json!({
            "source": "layer2-sdk-agent-server"
        });
        Ok(ModelApiRequest {
            api_kind: input.api_kind,
            endpoint_path: "responses".to_string(),
            body,
            response_mapper: codex_runtime_api::ProtocolResponseMapperKind::Responses,
        })
    }
}

struct Layer2ContextContributor;

impl ContextContributor for Layer2ContextContributor {
    fn id(&self) -> ContextContributorId {
        ContextContributorId::new("layer2.test.context_contributor")
    }

    async fn contribute(
        &self,
        _input: ContextContributorInput,
    ) -> Result<Vec<ContextBlock>, ContextError> {
        Ok(vec![ContextBlock {
            id: "layer2-stable-prefix".to_string(),
            slot: ContextBlockSlot::DeveloperPolicy,
            content: LAYER2_CONTEXT.to_string(),
            source: "layer2-test".to_string(),
            metadata: BTreeMap::new(),
        }])
    }
}

#[derive(Default)]
struct Layer2ContextObserver {
    observed: Arc<Mutex<Vec<Value>>>,
}

impl ContextAssemblyObserver for Layer2ContextObserver {
    fn id(&self) -> ContextAssemblyObserverId {
        ContextAssemblyObserverId::new("layer2.test.context_observer")
    }

    async fn observe(&self, input: ContextAssemblyObserverInput) -> Result<(), ContextError> {
        self.observed
            .lock()
            .map_err(|_| {
                runtime_test_error(
                    RuntimeCapability::ContextAssemblyObserver,
                    "layer2.test.context_observer",
                    RuntimeExtensionPhase::ContextObservation,
                    "observer could not record provider-bound input",
                )
            })?
            .push(input.provider_bound_input);
        Ok(())
    }
}

#[derive(Default)]
struct Layer2ToolMiddleware {
    repairs: Arc<Mutex<Vec<ToolCallRepairRecord>>>,
}

impl ToolMiddleware for Layer2ToolMiddleware {
    fn id(&self) -> ToolMiddlewareId {
        ToolMiddlewareId::new("layer2.test.tool_middleware")
    }

    async fn before_tool_call(
        &self,
        call: ToolCall,
    ) -> Result<ToolCallDecision, ToolMiddlewareError> {
        let decision = if call.call_id == LAYER2_TOOL_CALL_ID {
            ToolCallDecision::Repair {
                repaired_arguments: json!({
                    "query": "sdk-layer2",
                    "repaired": true
                }),
            }
        } else {
            ToolCallDecision::Continue
        };
        if let Some(repair) = decision.repair_record(&call) {
            self.repairs
                .lock()
                .map_err(|_| {
                    runtime_test_error(
                        RuntimeCapability::ToolMiddleware,
                        "layer2.test.tool_middleware",
                        RuntimeExtensionPhase::ToolBeforeCall,
                        "tool middleware could not record repair metadata",
                    )
                })?
                .push(repair);
        }
        Ok(decision)
    }

    async fn after_tool_call(
        &self,
        _call: ToolCall,
        _result: ToolResult,
    ) -> Result<ToolResultDecision, ToolMiddlewareError> {
        Ok(ToolResultDecision::Preserve)
    }
}

struct Layer2UsageMapper;

impl UsageMetadataMapper for Layer2UsageMapper {
    fn id(&self) -> UsageMetadataMapperId {
        UsageMetadataMapperId::new("layer2.test.usage_mapper")
    }

    async fn map_usage_metadata(
        &self,
        input: UsageMetadataMapperInput,
    ) -> Result<Option<UsageMetadata>, UsageMetadataMapperError> {
        Ok(input
            .raw_provider_metadata
            .values
            .get("response.metadata")
            .and_then(|metadata| metadata.get("layer2_usage"))
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .or(input.fallback_usage))
    }
}

async fn test_config(codex_home: &Path) -> anyhow::Result<Config> {
    match ConfigBuilder::default()
        .codex_home(codex_home.to_path_buf())
        .build()
        .await
    {
        Ok(config) => Ok(config),
        Err(_) => Ok(Config::load_default_with_cli_overrides_for_codex_home(
            codex_home.to_path_buf(),
            Vec::new(),
        )
        .await?),
    }
}

fn write_layer2_config(codex_home: &Path, server_uri: &str) -> anyhow::Result<()> {
    std::fs::write(
        codex_home.join("config.toml"),
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "read-only"
model_provider = "mock_provider"

[model_providers.mock_provider]
name = "Mock provider for SDK Layer 2 fixture"
base_url = "{server_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
"#
        ),
    )?;
    Ok(())
}

async fn layer2_client_start_args(codex_home: &Path) -> anyhow::Result<InProcessClientStartArgs> {
    Ok(InProcessClientStartArgs {
        arg0_paths: Arg0DispatchPaths::default(),
        config: Arc::new(test_config(codex_home).await?),
        cli_overrides: Vec::new(),
        loader_overrides: LoaderOverrides::default(),
        strict_config: false,
        cloud_config_bundle: CloudConfigBundleLoader::default(),
        feedback: CodexFeedback::new(),
        log_db: None,
        state_db: None,
        environment_manager: Arc::new(EnvironmentManager::default_for_tests()),
        config_warnings: Vec::new(),
        session_source: SessionSource::Cli,
        enable_codex_api_key_env: false,
        client_name: "layer2-sdk-agent-server-test".to_string(),
        client_version: "0.1.0".to_string(),
        experimental_api: true,
        opt_out_notification_methods: Vec::new(),
        channel_capacity: codex_app_server::in_process::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
    })
}

async fn next_dynamic_tool_call(
    client: &mut AppServerClient,
) -> anyhow::Result<(RequestId, DynamicToolCallParams)> {
    loop {
        let event = timeout(Duration::from_secs(60), client.next_event())
            .await
            .context("app-server should emit a dynamic tool request")?
            .context("event stream should stay open")?;
        if let AppServerEvent::ServerRequest(ServerRequest::DynamicToolCall {
            request_id,
            params,
        }) = event
        {
            return Ok((request_id, params));
        }
    }
}

async fn read_until_usage_and_completed(
    client: &mut AppServerClient,
) -> anyhow::Result<(
    ThreadTokenUsageUpdatedNotification,
    TurnCompletedNotification,
)> {
    let mut usage = None;
    let mut completed = None;
    while usage.is_none() || completed.is_none() {
        let event = timeout(Duration::from_secs(60), client.next_event())
            .await
            .context("app-server should emit turn events")?
            .context("event stream should stay open")?;
        if let AppServerEvent::ServerNotification(notification) = event {
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
    Ok((
        usage.context("usage should be observed")?,
        completed.context("turn completion should be observed")?,
    ))
}

fn value_contains_text(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(text) => text.contains(expected),
        Value::Array(items) => items.iter().any(|item| value_contains_text(item, expected)),
        Value::Object(map) => map.values().any(|item| value_contains_text(item, expected)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

#[tokio::test]
async fn sdk_starts_runnable_layer2_agent_server_client_with_codex_and_runtime_capabilities()
-> anyhow::Result<()> {
    let server = responses::start_mock_server().await;
    let first_response = responses::sse(vec![
        responses::ev_response_created("resp-1"),
        responses::ev_function_call(LAYER2_TOOL_CALL_ID, LAYER2_TOOL_NAME, "{malformed"),
        responses::ev_completed("resp-1"),
    ]);
    let second_response = responses::sse(vec![
        responses::ev_response_created("resp-2"),
        responses::ev_assistant_message("msg-1", "Layer 2 done"),
        json!({
            "type": "response.completed",
            "response": {
                "id": "resp-2",
                "usage": {
                    "input_tokens": 7,
                    "input_tokens_details": null,
                    "output_tokens": 2,
                    "output_tokens_details": null,
                    "total_tokens": 9
                },
                "metadata": {
                    "layer2_usage": {
                        "prompt_tokens": 13,
                        "completion_tokens": 21,
                        "cached_prompt_tokens": 5,
                        "cache_miss_prompt_tokens": 8,
                        "reasoning_tokens": 3
                    }
                }
            }
        }),
    ]);
    let response_mock =
        responses::mount_sse_sequence(&server, vec![first_response, second_response]).await;

    let codex_home = TempDir::new()?;
    write_layer2_config(codex_home.path(), &server.uri())?;

    let observer = Layer2ContextObserver::default();
    let observed = Arc::clone(&observer.observed);
    let middleware = Layer2ToolMiddleware::default();
    let repairs = Arc::clone(&middleware.repairs);
    let mut registry = RuntimeRegistry::builder();
    registry
        .model_request_adapter(Layer2ModelRequestAdapter)?
        .context_contributor(Layer2ContextContributor)?
        .context_assembly_observer(observer)?
        .tool_middleware(middleware)?
        .usage_metadata_mapper(Layer2UsageMapper)?;

    let mut client = AppServerBuilder::from_client_start_args(
        layer2_client_start_args(codex_home.path()).await?,
    )
    .runtime_registry(registry.build())
    .start_client()
    .await?;

    let _config: ConfigRequirementsReadResponse = client
        .request_typed(ClientRequest::ConfigRequirementsRead {
            request_id: RequestId::Integer(1),
            params: None,
        })
        .await?;

    let thread: ThreadStartResponse = client
        .request_typed(ClientRequest::ThreadStart {
            request_id: RequestId::Integer(2),
            params: ThreadStartParams {
                model: Some("mock-model".to_string()),
                dynamic_tools: Some(vec![DynamicToolSpec {
                    namespace: None,
                    name: LAYER2_TOOL_NAME.to_string(),
                    description: "Layer 2 SDK dynamic tool".to_string(),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "query": { "type": "string" },
                            "repaired": { "type": "boolean" }
                        },
                        "required": ["query", "repaired"],
                        "additionalProperties": false
                    }),
                    defer_loading: false,
                }]),
                ..ThreadStartParams::default()
            },
        })
        .await?;

    client
        .request_typed::<serde_json::Value>(ClientRequest::TurnStart {
            request_id: RequestId::Integer(3),
            params: TurnStartParams {
                thread_id: thread.thread.id.clone(),
                input: vec![UserInput::Text {
                    text: "Run the SDK Layer 2 server fixture".to_string(),
                    text_elements: Vec::new(),
                }],
                ..TurnStartParams::default()
            },
        })
        .await?;

    let (tool_request_id, tool_params) = next_dynamic_tool_call(&mut client).await?;
    assert_eq!(tool_params.call_id, LAYER2_TOOL_CALL_ID);
    assert_eq!(tool_params.tool, LAYER2_TOOL_NAME);
    assert_eq!(
        tool_params.arguments,
        json!({
            "query": "sdk-layer2",
            "repaired": true
        })
    );
    assert_eq!(
        repairs
            .lock()
            .map_err(|_| anyhow!("repair lock should not be poisoned"))?
            .clone(),
        vec![ToolCallRepairRecord {
            call_id: LAYER2_TOOL_CALL_ID.to_string(),
            tool_name: LAYER2_TOOL_NAME.to_string(),
            original_arguments: Value::String("{malformed".to_string()),
            repaired_arguments: json!({
                "query": "sdk-layer2",
                "repaired": true
            }),
        }]
    );

    client
        .resolve_server_request(
            tool_request_id,
            serde_json::to_value(DynamicToolCallResponse {
                content_items: vec![DynamicToolCallOutputContentItem::InputText {
                    text: "layer2-tool-ok".to_string(),
                }],
                success: true,
            })?,
        )
        .await?;

    let (usage, completed) = read_until_usage_and_completed(&mut client).await?;
    assert_eq!(completed.thread_id, thread.thread.id);
    assert_eq!(completed.turn.status, TurnStatus::Completed);
    assert_eq!(usage.token_usage.last.input_tokens, 13);
    assert_eq!(usage.token_usage.last.cached_input_tokens, 5);
    assert_eq!(usage.token_usage.last.output_tokens, 21);
    assert_eq!(usage.token_usage.last.reasoning_output_tokens, 3);
    assert_eq!(usage.token_usage.last.total_tokens, 37);

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 2);
    let first_body = requests[0].body_json();
    assert_eq!(
        first_body["client_metadata"],
        json!({
            "source": "layer2-sdk-agent-server"
        })
    );
    assert!(value_contains_text(&first_body["input"], LAYER2_CONTEXT));
    let tool_output = requests[1].function_call_output(LAYER2_TOOL_CALL_ID);
    assert_eq!(
        tool_output["call_id"],
        Value::String(LAYER2_TOOL_CALL_ID.to_string())
    );
    assert!(value_contains_text(&tool_output, "layer2-tool-ok"));

    let observed = observed
        .lock()
        .map_err(|_| anyhow!("observer lock should not be poisoned"))?
        .clone();
    assert_eq!(observed.len(), 2);
    assert!(
        observed
            .iter()
            .any(|input| value_contains_text(input, LAYER2_CONTEXT))
    );

    client.shutdown().await?;
    Ok(())
}
