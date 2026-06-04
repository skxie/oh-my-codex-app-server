# QA Report: Layer 1 Runtime Error Contract

Date: 2026-06-04
Scope: Codex Layer 1 runtime extension implementation after SDK startup ergonomics
Mode: code/test QA, no browser target

## Summary

QA found 2 error-contract gaps during the current PLAN.md implementation audit and fixed both:

1. `ContextPolicy` invalid decisions from app-server validation returned plain `InvalidRequest` strings instead of `RuntimeExtensionErrorInfo`.
2. `ToolMiddleware` invalid repair output for schema-specific tool payloads returned a plain tool error instead of `RuntimeExtensionErrorInfo`.

Both fixes keep app-server ownership unchanged. Context policy still only selects or rewrites allowed candidates, and tool middleware still runs before approval/executor with app-server preserving call identity, approval, sandbox, and executor ownership.

## Verification

Passed:

- `just test -p codex-core runtime_context`
- `just test -p codex-core runtime_tool_middleware_invalid_repair_returns_runtime_error runtime_tool_middleware_before_timeout_returns_runtime_error runtime_tool_middleware_after_timeout_returns_runtime_error runtime_tool_middleware_failure_surfaces_before_executor runtime_tool_middleware_repairs_arguments_before_executor runtime_tool_middleware_blocks_before_executor runtime_tool_middleware_normalizes_result_after_executor`
- `just verify-layer1-adapters`
- `just tthw-layer1`
- `just fix -p codex-core`
- `git diff --check`

`just verify-layer1-adapters` now includes 20 focused `codex-core` runtime tests in the model/context/tool/usage section, including the new invalid-repair gate.

## External Review Notes

Claude CLI was available at `/Users/xie/.local/bin/claude`, but authentication was missing in this environment, so no Claude CLI review output was used as evidence.

Collab reviewer subagents could not be started because the session agent thread limit was already reached by stale shutdown agents, and the alternate Claude Agent tool reported no available agent types. No external reviewer findings were counted as passing evidence in this report.

## Health

Before: Layer 1 take-effect gates passed, but two invalid-output paths did not consistently surface actionable runtime extension diagnostics.

After: invalid context policy decisions and invalid tool repair outputs surface `RuntimeExtensionErrorInfo` with capability, contributor id, phase, likely cause, fix guidance, and docs anchors.
