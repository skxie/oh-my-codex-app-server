# Codex Runtime API

`codex-runtime-api` is the Layer 1 control-plane API for fork-owned runtime
extensions. It exposes one active implementation for each runtime capability
through `RuntimeRegistry` while keeping Codex app-server in charge of thread,
turn, transport, approval, sandbox, tool execution, and event semantics.

## Capabilities

- `ModelRequestAdapter` changes the provider API request body at the request
  envelope level. It does not own HTTP, WebSocket, auth, retry, or streaming.
- `ContextContributor` adds stable context blocks before Codex assembles model
  input.
- `ContextPolicy` selects or replaces context candidates while preserving
  current user input and tool call/result pairing.
- `ContextAssemblyObserver` optionally observes the final provider-bound input
  for diagnostics and tests.
- `ToolMiddleware` repairs, blocks, or normalizes tool calls/results without
  bypassing approval, sandbox, or executor ownership.
- `UsageMetadataMapper` maps bounded provider usage/cache/reasoning metadata
  into stable Codex token usage fields.

## Registry Shape

Each capability has a no-op/default implementation. A custom backend installs
one implementation per capability through `RuntimeRegistry::builder()`.
Duplicate registration is a construction error so a Layer 2 backend has a
single runtime control plane.

```rust
let mut builder = codex_runtime_api::RuntimeRegistry::builder();
builder.model_request_adapter(my_adapter)?;
builder.context_policy(my_policy)?;
let registry = builder.build();
```

Use `codex-app-server-sdk::AppServerBuilder` or
`codex_app_server::in_process::start` to pass the registry into app-server.

## Verification

Run the fork rebase guard from `codex-rs`:

```bash
just verify-layer1-adapters
```

Run the hello-world end-to-end fixture:

```bash
just tthw-layer1
```

Cookbook examples compile under:

```text
codex-rs/app-server/tests/suite/layer2_cookbook_examples.rs
```
