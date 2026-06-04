use codex_runtime_api::ContextBlock;
use codex_runtime_api::ContextBlockSlot;
use codex_runtime_api::ContextContributor;
use codex_runtime_api::ContextContributorId;
use codex_runtime_api::ContextContributorInput;
use codex_runtime_api::ContextError;
use codex_runtime_api::ContextPolicy;
use codex_runtime_api::ContextPolicyDecision;
use codex_runtime_api::ContextPolicyId;
use codex_runtime_api::ContextPolicyInput;
use codex_runtime_api::ModelApiKind;
use codex_runtime_api::ModelApiRequest;
use codex_runtime_api::ModelRequestAdapter;
use codex_runtime_api::ModelRequestAdapterError;
use codex_runtime_api::ModelRequestAdapterId;
use codex_runtime_api::ModelRequestAdapterInput;
use codex_runtime_api::ProtocolResponseMapperKind;
use codex_runtime_api::RuntimeRegistry;
use codex_runtime_api::ToolCall;
use codex_runtime_api::ToolCallDecision;
use codex_runtime_api::ToolMiddleware;
use codex_runtime_api::ToolMiddlewareError;
use codex_runtime_api::ToolMiddlewareId;
use codex_runtime_api::ToolResult;
use codex_runtime_api::ToolResultDecision;
use codex_runtime_api::UsageMetadata;
use codex_runtime_api::UsageMetadataMapper;
use codex_runtime_api::UsageMetadataMapperError;
use codex_runtime_api::UsageMetadataMapperId;
use codex_runtime_api::UsageMetadataMapperInput;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::collections::BTreeMap;

struct CookbookModelRequestAdapter;

impl ModelRequestAdapter for CookbookModelRequestAdapter {
    fn id(&self) -> ModelRequestAdapterId {
        ModelRequestAdapterId::new("cookbook.model_request_adapter")
    }

    async fn build_request(
        &self,
        input: ModelRequestAdapterInput,
    ) -> Result<ModelApiRequest, ModelRequestAdapterError> {
        let mut body = input.body;
        body["metadata"] = json!({ "cookbook": true });
        Ok(ModelApiRequest {
            api_kind: ModelApiKind::Responses,
            endpoint_path: "responses".to_string(),
            body,
            response_mapper: ProtocolResponseMapperKind::Responses,
        })
    }
}

struct CookbookContextContributor;

impl ContextContributor for CookbookContextContributor {
    fn id(&self) -> ContextContributorId {
        ContextContributorId::new("cookbook.context_contributor")
    }

    async fn contribute(
        &self,
        _input: ContextContributorInput,
    ) -> Result<Vec<ContextBlock>, ContextError> {
        Ok(vec![ContextBlock {
            id: "cookbook-prefix".to_string(),
            slot: ContextBlockSlot::ContextualUser,
            content: "cache-prefix: stable project context".to_string(),
            source: "cookbook".to_string(),
            metadata: BTreeMap::new(),
        }])
    }
}

struct CookbookContextPolicy;

impl ContextPolicy for CookbookContextPolicy {
    fn id(&self) -> ContextPolicyId {
        ContextPolicyId::new("cookbook.context_policy")
    }

    async fn select_context(
        &self,
        mut input: ContextPolicyInput,
    ) -> Result<ContextPolicyDecision, ContextError> {
        if let Some(first) = input.candidates.first_mut() {
            first.content = "context-summary: old trace".to_string();
        }
        Ok(ContextPolicyDecision {
            selected: input.candidates,
        })
    }
}

struct CookbookToolMiddleware;

impl ToolMiddleware for CookbookToolMiddleware {
    fn id(&self) -> ToolMiddlewareId {
        ToolMiddlewareId::new("cookbook.tool_middleware")
    }

    async fn before_tool_call(
        &self,
        call: ToolCall,
    ) -> Result<ToolCallDecision, ToolMiddlewareError> {
        if call.arguments == json!("{malformed") {
            Ok(ToolCallDecision::Repair {
                repaired_arguments: json!({ "fixed": true }),
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

struct CookbookUsageMetadataMapper;

impl UsageMetadataMapper for CookbookUsageMetadataMapper {
    fn id(&self) -> UsageMetadataMapperId {
        UsageMetadataMapperId::new("cookbook.usage_metadata_mapper")
    }

    async fn map_usage_metadata(
        &self,
        input: UsageMetadataMapperInput,
    ) -> Result<Option<UsageMetadata>, UsageMetadataMapperError> {
        let Some(cache_hit) = input.raw_provider_metadata.values.get("cache_hit") else {
            return Ok(input.fallback_usage);
        };
        Ok(Some(UsageMetadata {
            prompt_tokens: 100,
            completion_tokens: 25,
            cached_prompt_tokens: cache_hit.as_u64(),
            cache_miss_prompt_tokens: Some(20),
            reasoning_tokens: Some(7),
        }))
    }
}

#[tokio::test]
async fn model_request_adapter() {
    let mut registry = RuntimeRegistry::builder();
    registry
        .model_request_adapter(CookbookModelRequestAdapter)
        .expect("register cookbook model adapter");
    let request = registry
        .build()
        .build_model_request(ModelRequestAdapterInput {
            provider: "openai".to_string(),
            model: "gpt-test".to_string(),
            api_kind: ModelApiKind::Responses,
            body: json!({ "model": "gpt-test" }),
            instructions: String::new(),
            input: json!([]),
            tools: json!([]),
            parallel_tool_calls: true,
            metadata: BTreeMap::new(),
        })
        .await
        .expect("build cookbook request");

    assert_eq!(request.body["metadata"], json!({ "cookbook": true }));
}

#[tokio::test]
async fn context_contributor_and_policy() {
    let mut registry = RuntimeRegistry::builder();
    registry
        .context_contributor(CookbookContextContributor)
        .expect("register cookbook contributor")
        .context_policy(CookbookContextPolicy)
        .expect("register cookbook policy");
    let registry = registry.build();

    let blocks = registry
        .contribute_context(ContextContributorInput {
            turn_id: "turn-1".to_string(),
            metadata: BTreeMap::new(),
        })
        .await
        .expect("contribute context");
    let decision = registry
        .select_context(ContextPolicyInput {
            candidates: vec![codex_runtime_api::ContextCandidate {
                id: "item-0".to_string(),
                source: codex_runtime_api::ContextCandidateSource::History,
                content: "old trace".to_string(),
            }],
            token_budget: Some(4096),
        })
        .await
        .expect("select context");

    assert_eq!(blocks[0].content, "cache-prefix: stable project context");
    assert_eq!(decision.selected[0].content, "context-summary: old trace");
}

#[tokio::test]
async fn tool_middleware() {
    let mut registry = RuntimeRegistry::builder();
    registry
        .tool_middleware(CookbookToolMiddleware)
        .expect("register cookbook tool middleware");
    let decision = registry
        .build()
        .before_tool_call(ToolCall {
            call_id: "call-1".to_string(),
            tool_name: "shell".to_string(),
            source: codex_runtime_api::ToolCallSource::Model,
            arguments: json!("{malformed"),
        })
        .await
        .expect("repair tool call");

    assert_eq!(
        decision,
        ToolCallDecision::Repair {
            repaired_arguments: json!({ "fixed": true }),
        }
    );
}

#[tokio::test]
async fn usage_metadata_mapper() {
    let mut registry = RuntimeRegistry::builder();
    registry
        .usage_metadata_mapper(CookbookUsageMetadataMapper)
        .expect("register cookbook usage mapper");
    let mut raw = codex_runtime_api::RawProviderMetadata {
        values: BTreeMap::new(),
    };
    raw.values.insert("cache_hit".to_string(), json!(80));

    let usage = registry
        .build()
        .map_usage_metadata(UsageMetadataMapperInput {
            raw_provider_metadata: raw,
            fallback_usage: None,
        })
        .await
        .expect("map usage")
        .expect("usage should be mapped");

    assert_eq!(
        usage,
        UsageMetadata {
            prompt_tokens: 100,
            completion_tokens: 25,
            cached_prompt_tokens: Some(80),
            cache_miss_prompt_tokens: Some(20),
            reasoning_tokens: Some(7),
        }
    );
}
