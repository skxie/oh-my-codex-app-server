use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::protocol::TokenUsage;
use codex_runtime_api::RawProviderMetadata;
use codex_runtime_api::RuntimeCapability;
use codex_runtime_api::RuntimeExtensionErrorInfo;
use codex_runtime_api::RuntimeExtensionPhase;
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

    mapped
        .map(token_usage_from_usage_metadata)
        .transpose()
        .map_err(|message| {
            CodexErr::InvalidRequest(
                RuntimeExtensionErrorInfo::new(
                    RuntimeCapability::UsageMetadataMapper,
                    registry.usage_metadata_mapper_id().to_string(),
                    RuntimeExtensionPhase::UsageMapping,
                    message,
                    "return token counts that fit the app-server TokenUsage event fields",
                )
                .to_string(),
            )
        })
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

fn token_usage_from_usage_metadata(metadata: UsageMetadata) -> Result<TokenUsage, String> {
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

fn to_i64(value: u64, field: &str) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| format!("returned {field} value that exceeds i64: {value}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_runtime_api::UsageMetadataMapper;
    use codex_runtime_api::UsageMetadataMapperError;
    use codex_runtime_api::UsageMetadataMapperId;
    use pretty_assertions::assert_eq;

    struct FailingUsageMapper;

    impl UsageMetadataMapper for FailingUsageMapper {
        fn id(&self) -> UsageMetadataMapperId {
            UsageMetadataMapperId::new("test.failing_usage_mapper")
        }

        async fn map_usage_metadata(
            &self,
            _input: UsageMetadataMapperInput,
        ) -> Result<Option<UsageMetadata>, UsageMetadataMapperError> {
            Err(RuntimeExtensionErrorInfo::new(
                RuntimeCapability::UsageMetadataMapper,
                "test.failing_usage_mapper",
                RuntimeExtensionPhase::UsageMapping,
                "provider metadata was missing cache counters",
                "return fallback usage or provider cache counters",
            ))
        }
    }

    struct OverflowUsageMapper;

    impl UsageMetadataMapper for OverflowUsageMapper {
        fn id(&self) -> UsageMetadataMapperId {
            UsageMetadataMapperId::new("test.overflow_usage_mapper")
        }

        async fn map_usage_metadata(
            &self,
            _input: UsageMetadataMapperInput,
        ) -> Result<Option<UsageMetadata>, UsageMetadataMapperError> {
            Ok(Some(UsageMetadata {
                prompt_tokens: u64::MAX,
                completion_tokens: 0,
                cached_prompt_tokens: None,
                cache_miss_prompt_tokens: None,
                reasoning_tokens: None,
            }))
        }
    }

    fn token_usage() -> TokenUsage {
        TokenUsage {
            input_tokens: 1,
            cached_input_tokens: 0,
            output_tokens: 2,
            reasoning_output_tokens: 0,
            total_tokens: 3,
        }
    }

    #[tokio::test]
    async fn usage_mapper_failure_surfaces_runtime_extension_info() {
        let mut builder = RuntimeRegistry::builder();
        builder
            .usage_metadata_mapper(FailingUsageMapper)
            .expect("register failing usage mapper");

        let err = map_token_usage(
            &builder.build(),
            &token_usage(),
            /*raw_provider_metadata*/ None,
        )
        .await
        .expect_err("failing usage mapper should surface");

        match err {
            CodexErr::InvalidRequest(message) => assert_eq!(
                message,
                "UsageMetadataMapper `test.failing_usage_mapper` failed during UsageMapping: provider metadata was missing cache counters. Fix: return fallback usage or provider cache counters"
            ),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    async fn usage_mapper_invalid_output_surfaces_runtime_extension_info() {
        let mut builder = RuntimeRegistry::builder();
        builder
            .usage_metadata_mapper(OverflowUsageMapper)
            .expect("register overflow usage mapper");

        let err = map_token_usage(
            &builder.build(),
            &token_usage(),
            /*raw_provider_metadata*/ None,
        )
        .await
        .expect_err("overflow usage mapper output should surface");

        match err {
            CodexErr::InvalidRequest(message) => assert_eq!(
                message,
                "UsageMetadataMapper `test.overflow_usage_mapper` failed during UsageMapping: returned prompt_tokens value that exceeds i64: 18446744073709551615. Fix: return token counts that fit the app-server TokenUsage event fields"
            ),
            other => panic!("unexpected error: {other}"),
        }
    }
}
