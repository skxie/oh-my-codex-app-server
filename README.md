# Open Codex App-Server Foundation

This fork turns the Codex app-server into a reusable Layer 1 foundation for
custom agent harness backends.

The goal is to let downstream backends reuse Codex's thread, turn, tool,
sandbox, approval, event, and persistence machinery while installing their own
runtime behavior through narrow backend extension seams.

Upstream Codex already exposes a strong app-server protocol, MCP/plugin
integration, dynamic tool callbacks, and client-facing session APIs. Those
surfaces are enough for building clients and external tools, but they do not
make the internal model request, context assembly, tool-call repair, usage
mapping, or in-process app-server embedding path available as stable backend
extension points. A custom harness that needs those controls would otherwise
have to patch scattered core runtime code or move runtime policy into the
client.

This fork adds a small Layer 1 between upstream Codex and downstream harnesses:
it keeps app-server ownership of execution, but exposes selected runtime control
points through stable Rust APIs. The fork stays general-purpose:
DeepSeek-specific, Claude-specific, memory-product, or UI behavior belongs in
downstream Layer 2 and Layer 3 projects.

## Architecture

```mermaid
flowchart LR
  Upstream["Official openai/codex"]

  subgraph L1["Layer 1: this fork"]
    direction TB
    AppServer["Codex app-server\nthreads / turns / tools / sandbox"]
    RuntimeRegistry["RuntimeRegistry"]
    RuntimeApi["codex-runtime-api"]
    Adapters["runtime adapters"]

    AppServer --> RuntimeRegistry
    RuntimeRegistry --> RuntimeApi
    RuntimeRegistry --> Adapters
  end

  subgraph L2["Layer 2: custom harness backend"]
    Backend["backend server"]
    RuntimeImpls["model / context / tool / usage implementations"]
  end

  subgraph L3["Layer 3: application clients"]
    Clients["TUI / desktop / mobile / web"]
  end

  Upstream -->|"periodic rebase"| L1
  RuntimeImpls --> RuntimeRegistry
  Backend --> RuntimeImpls
  Clients --> Backend
```

Codex app-server remains the owner of thread lifecycle, turn execution,
approval, sandbox, tool routing, event emission, and persistence. Layer 2 code
can opt into the new runtime surfaces to adapt model request bodies, contribute
and select context, observe final provider-bound input, repair tool calls, and
normalize usage metadata.

## What Changes

| Area                | Upstream Codex                                                                   | This Layer 1 fork                                                                                                         |
| ------------------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| App-server protocol | Client-facing JSON-RPC for threads, turns, events, config, approvals, and tools. | Keeps the same app-server ownership model and adds SDK-friendly embedding.                                                |
| Tool extension      | MCP, plugins, and dynamic tool callbacks.                                        | Adds backend tool middleware for validation, repair, blocking, and result normalization before/after app-server dispatch. |
| Model provider path | Codex-owned request construction and transport.                                  | Adds request-body-level model adaptation while Codex still owns transport, auth, retries, and streaming.                  |
| Context path        | Codex-owned prompt/context assembly and history selection.                       | Adds bounded context contribution, context policy, and final provider-bound input observation.                            |
| Usage metadata      | Provider usage is handled inside Codex runtime paths.                            | Adds normalized usage/cache/reasoning metadata mapping for downstream harness logic.                                      |
| Embedding           | App-server is primarily consumed as a Codex runtime.                             | Adds `codex-app-server-sdk` so Layer 2 can start an in-process app-server with a custom runtime registry.                 |

- `codex-runtime-api`: stable boundary types and traits for runtime extension
  capabilities.
- `RuntimeRegistry`: the composition point for one active implementation per
  runtime capability.
- `codex-app-server-sdk`: an embedding path for building a Layer 2 app-server
  that still uses the existing Codex app-server runtime.
- Runtime take-effect tests and CI gates that prove custom context, model
  request, tool repair, and usage behavior flow through app-server.

With those additions, Layer 2 can implement capabilities that are awkward or
not cleanly possible against upstream Codex alone: provider-specific request
shaping for DeepSeek or Claude, cache-first context policy, memory/retrieval
insertion, final-context diagnostics, malformed tool-call repair, usage/cache
metadata normalization, and product-specific backend policy while keeping the
TUI, desktop, mobile, or web client thin.

For the detailed SDK path, see
[Building a Layer 2 app-server with the SDK](./docs/layer2-app-server-sdk.md).

## Upstream Codex

This fork is based on [OpenAI Codex](https://github.com/openai/codex), a local
coding agent that can run in your terminal, IDE, or desktop app.

To install upstream Codex CLI on Mac or Linux:

```shell
curl -fsSL https://chatgpt.com/codex/install.sh | sh
```

To install upstream Codex CLI on Windows:

```powershell
powershell -ExecutionPolicy ByPass -c "irm https://chatgpt.com/codex/install.ps1 | iex"
```

Codex CLI can also be installed with npm or Homebrew:

```shell
npm install -g @openai/codex
brew install --cask codex
```

Run `codex` to get started with the CLI, or run `codex app` for the desktop app
experience.

## Documentation

- [Layer 2 app-server SDK](./docs/layer2-app-server-sdk.md)
- [Upstream Codex documentation](https://developers.openai.com/codex)
- [Contributing](./docs/contributing.md)
- [Installing and building](./docs/install.md)

This repository is licensed under the [Apache-2.0 License](LICENSE).
