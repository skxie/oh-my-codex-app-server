# Building a Layer 2 app-server with the SDK

This document describes how a custom Layer 2 agent harness backend can embed
this fork's Codex app-server through `codex-app-server-sdk`.

Layer 2 code starts the existing app-server runtime in-process, installs a
custom `RuntimeRegistry`, then talks to app-server through the normal typed
client. The SDK does not create a second runtime and does not move execution
ownership out of Codex app-server.

## Architecture

```text
Layer 2 backend
  -> codex_app_server_sdk::AppServerBuilder
  -> RuntimeRegistry from codex-runtime-api
  -> existing Codex app-server runtime
  -> model, context, tool, approval, sandbox, event, and persistence paths
```

The split is:

- Layer 2 owns product-specific runtime policy: provider request shaping,
  context strategy, tool-call repair, usage interpretation, memory, and custom
  backend behavior.
- Layer 1 owns the generic extension seams and SDK entrypoint.
- Codex app-server still owns thread and turn lifecycle, JSON-RPC request and
  event delivery, dynamic tool dispatch, MCP/tool execution, approval, sandbox,
  persistence, and shutdown.

## Dependencies

In a separate Layer 2 Rust repo, depend on this fork's workspace crates. Use the
branch or revision that contains the Layer 1 implementation:

```toml
[dependencies]
anyhow = "1"
serde_json = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }

codex-app-server-sdk = { git = "https://github.com/skxie/codex.git", branch = "main", package = "codex-app-server-sdk" }
codex-runtime-api = { git = "https://github.com/skxie/codex.git", branch = "main", package = "codex-runtime-api" }
codex-app-server-protocol = { git = "https://github.com/skxie/codex.git", branch = "main", package = "codex-app-server-protocol" }
```

If the Layer 2 backend lives inside this workspace, use path dependencies
instead.

## Startup Shape

Most embedders should construct the same
`codex_app_server_sdk::InProcessClientStartArgs` that the production app-server
client path uses, then install only the runtime registry:

```rust
use codex_app_server_sdk::AppServerBuilder;
use codex_app_server_sdk::InProcessClientStartArgs;
use codex_runtime_api::RuntimeRegistry;

async fn start_layer2_app_server(
    client_start_args: InProcessClientStartArgs,
) -> anyhow::Result<codex_app_server_sdk::AppServerClient> {
    let mut registry = RuntimeRegistry::builder();
    registry
        .model_request_adapter(MyModelRequestAdapter)?
        .context_contributor(MyContextContributor)?
        .context_policy(MyContextPolicy)?
        .tool_middleware(MyToolMiddleware)?
        .usage_metadata_mapper(MyUsageMetadataMapper)?;

    let client = AppServerBuilder::from_client_start_args(client_start_args)
        .runtime_registry(registry.build())
        .start_client()
        .await?;

    Ok(client)
}
```

Use `AppServerBuilder::new(in_process_start_args)` only when your backend has
already resolved the lower-level `InProcessStartArgs`. Use `start()` only when
you need the raw in-process transport handle. Most Layer 2 backends should use
`start_client()` so they receive the typed `AppServerClient` facade.

## Runtime Capabilities

`RuntimeRegistry` accepts one active implementation for each capability. Each
capability has a no-op default, so installing no registry preserves stock Codex
behavior.

`ModelRequestAdapter` works at the request body level. It can choose the model
API request shape and response mapper kind, but it does not own HTTP,
WebSocket, retries, auth, or streaming transport:

```rust
use codex_runtime_api::ModelApiRequest;
use codex_runtime_api::ModelRequestAdapter;
use codex_runtime_api::ModelRequestAdapterError;
use codex_runtime_api::ModelRequestAdapterId;
use codex_runtime_api::ModelRequestAdapterInput;

struct MyModelRequestAdapter;

impl ModelRequestAdapter for MyModelRequestAdapter {
    fn id(&self) -> ModelRequestAdapterId {
        ModelRequestAdapterId::new("layer2.model_request_adapter")
    }

    async fn build_request(
        &self,
        input: ModelRequestAdapterInput,
    ) -> Result<ModelApiRequest, ModelRequestAdapterError> {
        let mut body = input.body;
        body["client_metadata"] = serde_json::json!({
            "source": "layer2-agent-server"
        });

        Ok(ModelApiRequest {
            api_kind: input.api_kind,
            endpoint_path: "responses".to_string(),
            body,
            response_mapper: codex_runtime_api::ProtocolResponseMapperKind::Responses,
        })
    }
}
```

`ContextContributor` adds bounded context blocks into Codex-owned prompt
assembly slots. `ContextPolicy` selects or replaces candidate history/context
items before model sampling:

```rust
use codex_runtime_api::ContextBlock;
use codex_runtime_api::ContextBlockSlot;
use codex_runtime_api::ContextCandidateSource;
use codex_runtime_api::ContextContributor;
use codex_runtime_api::ContextContributorId;
use codex_runtime_api::ContextContributorInput;
use codex_runtime_api::ContextError;
use codex_runtime_api::ContextPolicy;
use codex_runtime_api::ContextPolicyDecision;
use codex_runtime_api::ContextPolicyId;
use codex_runtime_api::ContextPolicyInput;
use std::collections::BTreeMap;

struct MyContextContributor;

impl ContextContributor for MyContextContributor {
    fn id(&self) -> ContextContributorId {
        ContextContributorId::new("layer2.context_contributor")
    }

    async fn contribute(
        &self,
        _input: ContextContributorInput,
    ) -> Result<Vec<ContextBlock>, ContextError> {
        Ok(vec![ContextBlock {
            id: "layer2-stable-prefix".to_string(),
            slot: ContextBlockSlot::DeveloperPolicy,
            content: "layer2 stable project context".to_string(),
            source: "layer2".to_string(),
            metadata: BTreeMap::new(),
        }])
    }
}

struct MyContextPolicy;

impl ContextPolicy for MyContextPolicy {
    fn id(&self) -> ContextPolicyId {
        ContextPolicyId::new("layer2.context_policy")
    }

    async fn select_context(
        &self,
        input: ContextPolicyInput,
    ) -> Result<ContextPolicyDecision, ContextError> {
        Ok(ContextPolicyDecision {
            selected: input
                .candidates
                .into_iter()
                .map(|mut candidate| {
                    if matches!(candidate.source, ContextCandidateSource::History) {
                        candidate.content = "layer2 summary of earlier history".to_string();
                    }
                    candidate
                })
                .collect(),
        })
    }
}
```

`ToolMiddleware` can repair or block tool calls before app-server dispatches
them. Approval, sandbox, and executor ownership still stay inside app-server:

```rust
use codex_runtime_api::ToolCall;
use codex_runtime_api::ToolCallDecision;
use codex_runtime_api::ToolMiddleware;
use codex_runtime_api::ToolMiddlewareError;
use codex_runtime_api::ToolMiddlewareId;
use codex_runtime_api::ToolResult;
use codex_runtime_api::ToolResultDecision;

struct MyToolMiddleware;

impl ToolMiddleware for MyToolMiddleware {
    fn id(&self) -> ToolMiddlewareId {
        ToolMiddlewareId::new("layer2.tool_middleware")
    }

    async fn before_tool_call(
        &self,
        call: ToolCall,
    ) -> Result<ToolCallDecision, ToolMiddlewareError> {
        if call.arguments == serde_json::json!("{malformed") {
            Ok(ToolCallDecision::Repair {
                repaired_arguments: serde_json::json!({ "query": "fixed" }),
            })
        } else {
            Ok(ToolCallDecision::Continue)
        }
    }

    async fn after_tool_call(
        &self,
        _call: ToolCall,
        _result: ToolResult,
    ) -> Result<ToolResultDecision, ToolMiddlewareError> {
        Ok(ToolResultDecision::Preserve)
    }
}
```

`UsageMetadataMapper` maps provider-specific cache and reasoning metadata into
Codex token usage fields:

```rust
use codex_runtime_api::UsageMetadata;
use codex_runtime_api::UsageMetadataMapper;
use codex_runtime_api::UsageMetadataMapperError;
use codex_runtime_api::UsageMetadataMapperId;
use codex_runtime_api::UsageMetadataMapperInput;

struct MyUsageMetadataMapper;

impl UsageMetadataMapper for MyUsageMetadataMapper {
    fn id(&self) -> UsageMetadataMapperId {
        UsageMetadataMapperId::new("layer2.usage_mapper")
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
```

`ContextAssemblyObserver` is optional. Use it when a Layer 2 backend needs to
capture final provider-bound input for diagnostics or take-effect tests.

## Calling app-server from Layer 2

After startup, use `AppServerClient` exactly like a normal app-server client.
The same client can call stock app-server RPCs and receive server events while
the runtime registry changes backend behavior:

```rust
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ConfigRequirementsReadResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::UserInput;

let _config: ConfigRequirementsReadResponse = client
    .request_typed(ClientRequest::ConfigRequirementsRead {
        request_id: RequestId::Integer(1),
        params: None,
    })
    .await?;

let thread: ThreadStartResponse = client
    .request_typed(ClientRequest::ThreadStart {
        request_id: RequestId::Integer(2),
        params: ThreadStartParams::default(),
    })
    .await?;

client
    .request_typed::<serde_json::Value>(ClientRequest::TurnStart {
        request_id: RequestId::Integer(3),
        params: TurnStartParams {
            thread_id: thread.thread.id.clone(),
            input: vec![UserInput::Text {
                text: "Run the Layer 2 backend".to_string(),
                text_elements: Vec::new(),
            }],
            ..TurnStartParams::default()
        },
    })
    .await?;
```

Dynamic tool calls are still emitted as app-server server requests. A Layer 2
client handles and resolves them through the same app-server client facade:

```rust
use codex_app_server_protocol::DynamicToolCallResponse;
use codex_app_server_protocol::DynamicToolCallOutputContentItem;
use codex_app_server_protocol::ServerRequest;
use codex_app_server_sdk::AppServerEvent;

if let Some(AppServerEvent::ServerRequest(ServerRequest::DynamicToolCall {
    request_id,
    params,
})) = client.next_event().await
{
    let _tool_name = params.tool;

    client
        .resolve_server_request(
            request_id,
            serde_json::to_value(DynamicToolCallResponse {
                content_items: vec![DynamicToolCallOutputContentItem::InputText {
                    text: "layer2-tool-ok".to_string(),
                }],
                success: true,
            })?,
        )
        .await?;
}
```

## Complete SDK fixture

The compile-checked Layer 2 server fixture lives at:

```text
codex-rs/app-server-sdk/tests/layer2_agent_server.rs
```

The main test is:

```text
sdk_starts_runnable_layer2_agent_server_client_with_codex_and_runtime_capabilities
```

It starts an app-server through `codex-app-server-sdk`, installs a custom
runtime registry, then verifies both stock app-server behavior and Layer 1
runtime take-effect:

- `config/requirements/read`, `thread/start`, and `turn/start` work through the
  SDK-created client.
- `ModelRequestAdapter` adds `client_metadata.source` with
  `layer2-sdk-agent-server` to the provider request body.
- `ContextContributor` injects a stable context prefix into outbound model
  input.
- `ContextPolicy` replaces prior history with a Layer 2 summary before the
  second provider request.
- `ContextAssemblyObserver` captures provider-bound input.
- `ToolMiddleware` repairs malformed dynamic tool arguments while preserving
  call identity and repair metadata.
- The dynamic tool response is sent back to the model through app-server.
- `UsageMetadataMapper` maps provider metadata into Codex token usage fields,
  including cached and reasoning tokens.
- The turn completes normally through app-server events.

Run it with:

```bash
cd codex-rs
just test -p codex-app-server-sdk
```

## Other take-effect tests

The SDK fixture is the best end-to-end example for a Layer 2 backend. Additional
tests cover smaller boundaries:

- `codex-rs/app-server/src/in_process.rs`
  - `runtime_registry_fake_backend_fixture_takes_effect_through_in_process_app_server`
  - Proves the registry affects the existing in-process app-server runtime path
    and prints the hello-world proof output.
- `codex-rs/app-server/tests/suite/layer2_cookbook_examples.rs`
  - `model_request_adapter`
  - `context_contributor_and_policy`
  - `tool_middleware`
  - `usage_metadata_mapper`
  - Provides focused, copyable examples for each runtime capability.

Useful commands:

```bash
cd codex-rs
just tthw-layer1
just verify-layer1-adapters
just pre-push-layer1
```

`just tthw-layer1` is the warm-checkout hello-world gate. It prints proof that
the fake backend changed request body, context, policy, tool repair, usage
metadata, and app-server events.

`just verify-layer1-adapters` is the focused rebase guard for Layer 1 runtime
seams.

`just pre-push-layer1` is the full local push gate used by this branch and the
GitHub workflow.

## Boundaries

- Runtime extension seams apply to regular turn model requests. Manual and
  automatic history compaction remain Codex-owned and do not currently pass
  through `ModelRequestAdapter` or `ContextAssemblyObserver`.
- `ModelRequestAdapter` works at request body and response mapper level. It
  does not choose WebSocket vs HTTP, streaming implementation, auth, retry, or
  transport policy.
- Tool middleware changes the call payload before dispatch or the result after
  execution. It does not bypass app-server approval, sandbox, or executor
  ownership.
- Register one implementation per capability. Different Layer 2 behaviors such
  as memory, retrieval, cache prefixing, and project context should be composed
  inside that capability implementation.
- Keep provider-specific and product-specific logic in Layer 2. Layer 1 should
  remain the generic app-server foundation.
