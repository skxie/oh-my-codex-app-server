use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::protocol::TokenUsage;
use codex_runtime_api::RawProviderMetadata;
use codex_runtime_api::RuntimeRegistry;
use codex_runtime_api::UsageMetadata;
use codex_runtime_api::UsageMetadataMapperInput;
use std::collections::HashMap;

pub(crate) async fn map_token_usage(
    registry: &RuntimeRegistry,
    token_usage: &TokenUsage,
    raw_provider_metadata: Option<&HashMap<String, serde_json::Value>>,
) -> CodexResult<Option<TokenUsage>> {
    let mapped = registry
        .map_usage_metadata(UsageMetadataMapperInput {
            raw_provider_metadata: RawProviderMetadata {
                values: raw_provider_metadata
                    .map(|values| values.clone().into_iter().collect())
                    .unwrap_or_default(),
            },
            fallback_usage: Some(usage_metadata_from_token_usage(token_usage)?),
        })
        .await
        .map_err(|error| CodexErr::InvalidRequest(error.to_string()))?;

    mapped.map(token_usage_from_usage_metadata).transpose()
}

fn usage_metadata_from_token_usage(token_usage: &TokenUsage) -> CodexResult<UsageMetadata> {
    Ok(UsageMetadata {
        prompt_tokens: to_u64(token_usage.input_tokens, "input_tokens")?,
        completion_tokens: to_u64(token_usage.output_tokens, "output_tokens")?,
        cached_prompt_tokens: Some(to_u64(
            token_usage.cached_input_tokens,
            "cached_input_tokens",
        )?),
        cache_miss_prompt_tokens: Some(to_u64(token_usage.non_cached_input(), "non_cached_input")?),
        reasoning_tokens: Some(to_u64(
            token_usage.reasoning_output_tokens,
            "reasoning_output_tokens",
        )?),
    })
}

fn token_usage_from_usage_metadata(metadata: UsageMetadata) -> CodexResult<TokenUsage> {
    let input_tokens = to_i64(metadata.prompt_tokens, "prompt_tokens")?;
    let output_tokens = to_i64(metadata.completion_tokens, "completion_tokens")?;
    let cached_input_tokens = metadata
        .cached_prompt_tokens
        .map(|value| to_i64(value, "cached_prompt_tokens"))
        .transpose()?
        .unwrap_or_default();
    let reasoning_output_tokens = metadata
        .reasoning_tokens
        .map(|value| to_i64(value, "reasoning_tokens"))
        .transpose()?
        .unwrap_or_default();

    Ok(TokenUsage {
        input_tokens,
        cached_input_tokens,
        output_tokens,
        reasoning_output_tokens,
        total_tokens: input_tokens + output_tokens + reasoning_output_tokens,
    })
}

fn to_u64(value: i64, field: &str) -> CodexResult<u64> {
    u64::try_from(value).map_err(|_| {
        CodexErr::InvalidRequest(format!(
            "runtime usage metadata mapper cannot convert negative {field}: {value}"
        ))
    })
}

fn to_i64(value: u64, field: &str) -> CodexResult<i64> {
    i64::try_from(value).map_err(|_| {
        CodexErr::InvalidRequest(format!(
            "runtime usage metadata mapper value for {field} exceeds i64: {value}"
        ))
    })
}
