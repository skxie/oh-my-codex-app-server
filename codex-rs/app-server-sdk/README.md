# Codex App-Server SDK

`codex-app-server-sdk` is the thin Layer 1 embedding surface for custom agent
harness backends. It does not implement a second app-server runtime. It wraps
the existing `codex_app_server::in_process` startup contract and lets embedders
install a custom `codex_runtime_api::RuntimeRegistry`.

## Minimal Usage

Most Layer 2 embedders should start from the same production startup args used
by `codex-app-server-client`, then install only their runtime registry:

```rust
use codex_app_server_sdk::AppServerBuilder;

// client_start_args: codex_app_server_client::ClientStartArgs
let client = AppServerBuilder::from_client_start_args(client_start_args)
    .runtime_registry(runtime_registry)
    .start_client()
    .await?;
```

This preserves the app-server client's initialize, config, state, auth, and
thread-config startup behavior while letting the custom backend own Layer 1
runtime capabilities.

If the caller has already resolved the lower-level in-process startup contract,
the SDK also accepts it directly:

```rust
use codex_app_server_sdk::AppServerBuilder;

let client = AppServerBuilder::new(in_process_start_args)
    .runtime_registry(runtime_registry)
    .start_client()
    .await?;
```

`in_process_start_args` is the same `InProcessStartArgs` value that would be
passed to `codex_app_server::in_process::start`. The SDK keeps Codex app-server
in charge of:

- `MessageProcessor` startup;
- thread and turn lifecycle;
- app-server event delivery;
- approval and sandbox policy;
- tool routing and execution;
- shutdown behavior.

The runtime registry only controls the Layer 1 extension seams exposed by
`codex-runtime-api`.

Use `start()` only when the caller needs the lower-level in-process transport
handle. Most embedders should use `start_client()` so they receive the typed
`codex_app_server_client::AppServerClient` facade.

## Take-Effect Test

Run the SDK builder gate from `codex-rs`:

```bash
just test -p codex-app-server-sdk
```

The test proves a custom registry can be installed through `AppServerBuilder`
without changing unrelated in-process startup args. The full end-to-end runtime
proof still lives in the app-server golden fixture:

```bash
just tthw-layer1
```
