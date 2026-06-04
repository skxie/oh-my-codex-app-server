# QA Report: Layer 1 Runtime Extensions

Date: 2026-06-03
Scope: Codex Layer 1 runtime extension implementation
Mode: code/test QA, no browser target

## Summary

QA found 3 guardrail gaps from the engineering review and fixed all 3:

1. ToolMiddleware before/after calls could wait indefinitely.
2. ContextContributor blocks had no explicit size cap before prompt assembly.
3. RawProviderMetadata had no explicit size cap before UsageMetadataMapper.

Fix commit: f8cb35690a Add runtime extension guardrails

## Verification

Passed:

- `just test -p codex-core runtime_tool_middleware_before_timeout_returns_runtime_error runtime_tool_middleware_after_timeout_returns_runtime_error runtime_context_contributor_rejects_oversized_block usage_mapper_rejects_oversized_raw_provider_metadata`
- `just tthw-layer1`
- `just verify-layer1-adapters`
- `git diff --check`
- `just fix -p codex-core`

Claude CLI review was attempted three times but did not return a report within the available execution window; no Claude CLI findings were applied.

## Health

Before: implementation passed take-effect fixtures but missed three planned guardrails.
After: take-effect fixtures still pass, and each guardrail has an atomic failure test.

PR summary: QA fixed 3 guardrail issues, health score 8/10 -> 9/10.
