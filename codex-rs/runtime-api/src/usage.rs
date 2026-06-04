use crate::UsageMetadataMapperError;
use crate::UsageMetadataMapperId;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct RawProviderMetadata {
    pub values: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct UsageMetadata {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_prompt_tokens: Option<u64>,
    pub cache_miss_prompt_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct UsageMetadataMapperInput {
    pub raw_provider_metadata: RawProviderMetadata,
    pub fallback_usage: Option<UsageMetadata>,
}

/// Maps bounded provider metadata into stable usage/cache/reasoning fields.
pub trait UsageMetadataMapper: Send + Sync + 'static {
    fn id(&self) -> UsageMetadataMapperId;

    fn map_usage_metadata(
        &self,
        input: UsageMetadataMapperInput,
    ) -> impl std::future::Future<Output = Result<Option<UsageMetadata>, UsageMetadataMapperError>> + Send;
}

pub struct DefaultUsageMetadataMapper;

impl UsageMetadataMapper for DefaultUsageMetadataMapper {
    fn id(&self) -> UsageMetadataMapperId {
        UsageMetadataMapperId::new("codex.default.usage_metadata_mapper")
    }

    async fn map_usage_metadata(
        &self,
        input: UsageMetadataMapperInput,
    ) -> Result<Option<UsageMetadata>, UsageMetadataMapperError> {
        Ok(input.fallback_usage)
    }
}
