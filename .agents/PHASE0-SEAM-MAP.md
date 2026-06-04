# Phase 0 Seam Map: Runtime Extension Layer 1

This map pins each planned Layer 1 runtime capability to the current upstream
Codex owner path before production Rust changes start. It is intentionally a
fork-maintenance artifact: rebase conflicts should be checked against these
small seams instead of rediscovering the full runtime path.

## Scope

Phase 0 does not change production Rust code. It classifies each planned
capability as one of:

- bridge to an existing `ExtensionRegistry` path;
- add a new `RuntimeRegistry` hook;
- add construction plumbing only;
- defer until an integration test proves the hook is required.

## Capability Map

| Capability | Current owner path | Layer 1 action | Hook point | Stock equivalence proof | Custom take-effect proof |
|---|---|---|---|---|---|
| `RuntimeRegistry` | no current equivalent; app-server construction owns extension installation | add new fork-owned crate/API and construction plumbing | app-server startup passes a registry beside the existing `ExtensionRegistry` | default registry exposes stock/no-op implementations | custom registry changes only registered capability behavior |
| `ModelRequestAdapter` | `codex-rs/core/src/client.rs` `ModelClientSession::stream` and Responses request construction | add new `RuntimeRegistry` hook | after Codex has built the stock provider request body and before Codex sends it | default path builds the existing Responses request body | fake adapter builds a non-default request body and mapped response becomes normal assistant output |
| `ProtocolResponseMapper` | `codex-rs/core/src/client.rs` `map_response_stream` | add mapper selected by request adapter | after transport receives provider chunks and before normalized response events enter the turn loop | default mapper emits existing `ResponseEvent` sequence | fake mapper turns provider wire chunks into normalized assistant/usage events |
| `UsageMetadataMapper` | `codex-rs/core/src/session/turn.rs` completion handling and `codex-rs/core/src/session/mod.rs` `record_token_usage_info` | add new mapper before stock usage recording, then bridge to existing token usage extension observers | when a completed response carries usage/raw provider metadata | existing usage fields remain unchanged with default mapper | fake raw cache/reasoning metadata maps into stable usage fields |
| `ContextContributor` | `codex-rs/ext/extension-api/src/contributors.rs` and `codex-rs/context-fragments` prompt fragments | bridge through existing context/prompt fragment path | before final prompt/context assembly reads additive blocks | no custom contributor keeps stock context | fake contributor adds stable prefix visible in final provider-bound input |
| `ContextPolicy` | `codex-rs/core/src/session/mod.rs` context/history selection and `build_initial_context` path | add new `RuntimeRegistry` hook | before selected context/history candidates enter final assembly | default policy keeps stock selection | fake policy replaces an old candidate with a summary while preserving current user input and tool/result pairing |
| `ContextAssemblyObserver` | final model input currently not exposed as a stable observer surface | add optional observer hook | after final provider-bound input is assembled and before request adaptation/send | absent observer has no effect | fake observer captures the exact provider-bound input without mutating it |
| `ToolMiddleware.before_tool_call` | `codex-rs/core/src/tools/router.rs` dispatch path | add new `RuntimeRegistry` hook | after `ToolRouter` builds a tool call and before approval/sandbox/executor dispatch | no middleware preserves stock validation/approval/execution | fake middleware repairs malformed args while call id, tool name, and source stay unchanged |
| `ToolMiddleware.after_tool_call` | `codex-rs/core/src/tools/router.rs` dispatch result path | add new `RuntimeRegistry` hook | after executor returns and before model-visible tool output is emitted | no middleware preserves raw tool result | fake middleware normalizes output while preserving call id/status |
| App-server registry injection | `codex-rs/app-server/src/in_process.rs`, `codex-rs/app-server/src/message_processor.rs`, `codex-rs/app-server/src/extensions.rs` | add sibling registry argument | construction path that currently builds `ExtensionRegistry` and `ThreadManager` | omitted registry equals stock host | supplied registry reaches core runtime seams |

## Current Evidence

### Model Request and Response Mapping

- `codex-rs/core/src/client.rs` defines `ModelClient` and
  `ModelClientSession`.
- `ModelClientSession::stream` owns provider request construction and transport
  dispatch.
- Responses request creation currently includes tool JSON construction before
  transport dispatch.
- `map_response_stream` converts provider stream data into normalized
  `ResponseEvent` values.
- `codex-rs/core/src/session/turn.rs` calls the model client session stream and
  processes `ResponseEvent::Completed`.

Layer 1 implication: `ModelRequestAdapter` should not own HTTP, websocket,
auth, retry, cancellation, telemetry, sticky turn state, or streaming
backpressure. It only owns the provider API envelope/request-body decision and
the response mapper selection.

### Usage Metadata

- `codex-rs/core/src/session/turn.rs` handles completed response usage.
- `codex-rs/core/src/session/mod.rs` records token usage via
  `record_token_usage_info`.
- Existing token usage contributors already observe recorded usage through
  `ExtensionRegistry`.

Layer 1 implication: `UsageMetadataMapper` maps bounded provider metadata into
stable usage fields before stock usage recording. It should not replace the
existing usage contributor notification path.

### Context

- `codex-rs/context-fragments/src/fragment.rs` defines contextual user
  fragments.
- `codex-rs/context-fragments/src/additional_context.rs` defines existing
  additional-context fragments.
- `codex-rs/core/src/context/contextual_user_message.rs` registers additional
  context fragments.
- `codex-rs/core/src/session/mod.rs` contains `build_initial_context`.

Layer 1 implication: additive context should reuse or bridge into the existing
fragment path. History/context selection is a separate `ContextPolicy` runtime
capability because it changes which candidate items enter final assembly.

### Tool Dispatch

- `codex-rs/core/src/tools/router.rs` owns tool dispatch.
- The dispatch path is downstream of tool-call construction and upstream of
  approval, sandbox, executor invocation, and model-visible result emission.
- Existing tool contributor/lifecycle contributor APIs are peripheral
  extension surfaces, not runtime-control middleware.

Layer 1 implication: the before hook must validate any repaired effective call
before approval/sandbox/executor dispatch. Middleware cannot bypass approval or
sandbox because those owners must receive the effective call after validation.

### App-server Construction

- `codex-rs/app-server/src/in_process.rs` defines in-process startup args and
  constructs `MessageProcessor`.
- `codex-rs/app-server/src/message_processor.rs` passes extension wiring into
  `ThreadManager`.
- `codex-rs/app-server/src/extensions.rs` builds the existing
  `ExtensionRegistry`.
- `codex-rs/ext/extension-api/src/registry.rs` supports multiple peripheral
  contributors.

Layer 1 implication: `RuntimeRegistry` remains separate from
`ExtensionRegistry`. The former is a single-active runtime control plane; the
latter remains a multi-contributor peripheral extension/observer plane.

## Phase 1 Entry Criteria

- `RuntimeRegistry` starts in a new `codex-runtime-api` crate.
- Required runtime capabilities have no-op/default implementations.
- Optional `ContextAssemblyObserver` is absent by default.
- Duplicate same-capability registration is a construction error.
- RPITIT public traits are stored internally through erased object-safe
  adapters.
- Tests compare complete boundary objects where possible.
