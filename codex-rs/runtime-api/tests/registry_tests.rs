use codex_runtime_api::*;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;

#[test]
fn default_registry_uses_no_op_required_capabilities_and_no_observer() {
    let registry = RuntimeRegistry::default();

    assert_eq!(
        registry.model_request_adapter_id(),
        ModelRequestAdapterId::new("codex.default.model_request_adapter")
    );
    assert_eq!(
        registry.context_contributor_id(),
        ContextContributorId::new("codex.default.context_contributor")
    );
    assert_eq!(
        registry.context_policy_id(),
        ContextPolicyId::new("codex.default.context_policy")
    );
    assert_eq!(registry.context_assembly_observer_id(), None);
    assert_eq!(
        registry.tool_middleware_id(),
        ToolMiddlewareId::new("codex.default.tool_middleware")
    );
    assert_eq!(
        registry.usage_metadata_mapper_id(),
        UsageMetadataMapperId::new("codex.default.usage_metadata_mapper")
    );
}

#[test]
fn duplicate_same_capability_registration_returns_structured_error() {
    let mut builder = RuntimeRegistry::builder();
    builder
        .model_request_adapter(FakeModelRequestAdapter("first"))
        .unwrap();

    let actual = match builder.model_request_adapter(FakeModelRequestAdapter("second")) {
        Ok(_) => panic!("duplicate model request adapter registration should fail"),
        Err(error) => error,
    };

    let expected = RuntimeRegistryBuildError::DuplicateCapability {
        capability: RuntimeCapability::ModelRequestAdapter,
        existing_contributor_id: RuntimeContributorId::new("first"),
        attempted_contributor_id: RuntimeContributorId::new("second"),
    };
    assert_eq!(actual, expected);
}

#[test]
fn duplicate_registration_error_maps_to_runtime_extension_error_info() {
    let error = RuntimeRegistryBuildError::DuplicateCapability {
        capability: RuntimeCapability::ContextPolicy,
        existing_contributor_id: RuntimeContributorId::new("first.policy"),
        attempted_contributor_id: RuntimeContributorId::new("second.policy"),
    };

    let actual = RuntimeExtensionErrorInfo::from(error);

    let expected = RuntimeExtensionErrorInfo {
        capability: RuntimeCapability::ContextPolicy,
        contributor_id: RuntimeContributorId::new("second.policy"),
        phase: RuntimeExtensionPhase::Registration,
        what_happened: "runtime capability already has implementation `first.policy`".to_string(),
        why_likely: "the builder registered the same runtime capability more than once".to_string(),
        how_to_fix: "register only one active implementation for this runtime capability"
            .to_string(),
        docs_anchor: Some("runtime-registry-duplicate-capability".to_string()),
    };
    assert_eq!(actual, expected);
}

#[test]
fn all_runtime_capabilities_can_coexist_with_one_active_implementation_each() {
    let mut builder = RuntimeRegistry::builder();
    builder
        .model_request_adapter(FakeModelRequestAdapter("model"))
        .unwrap()
        .context_contributor(FakeContextContributor)
        .unwrap()
        .context_policy(FakeContextPolicy)
        .unwrap()
        .context_assembly_observer(FakeContextObserver::default())
        .unwrap()
        .tool_middleware(FakeToolMiddleware)
        .unwrap()
        .usage_metadata_mapper(FakeUsageMapper)
        .unwrap();

    let registry = builder.build();

    assert_eq!(
        registry.model_request_adapter_id(),
        ModelRequestAdapterId::new("model")
    );
    assert_eq!(
        registry.context_contributor_id(),
        ContextContributorId::new("fake.context_contributor")
    );
    assert_eq!(
        registry.context_policy_id(),
        ContextPolicyId::new("fake.context_policy")
    );
    assert_eq!(
        registry.context_assembly_observer_id(),
        Some(ContextAssemblyObserverId::new("fake.context_observer"))
    );
    assert_eq!(
        registry.tool_middleware_id(),
        ToolMiddlewareId::new("fake.tool_middleware")
    );
    assert_eq!(
        registry.usage_metadata_mapper_id(),
        UsageMetadataMapperId::new("fake.usage_mapper")
    );
}

#[test]
fn rpitit_model_adapter_dispatches_through_registry_erasure() {
    let mut builder = RuntimeRegistry::builder();
    builder
        .model_request_adapter(ChatCompletionsAdapter)
        .unwrap();
    let registry = builder.build();

    let actual = tokio_test::block_on(registry.build_model_request(ModelRequestAdapterInput {
        provider: "fake-provider".to_string(),
        model: "fake-model".to_string(),
        api_kind: ModelApiKind::ChatCompletions,
        body: serde_json::json!({
            "model": "fake-model",
            "messages": [
                { "role": "user", "content": "hello" }
            ],
            "tools": [],
            "parallel_tool_calls": true,
        }),
        instructions: "Be brief.".to_string(),
        input: serde_json::json!([
            { "role": "user", "content": "hello" }
        ]),
        tools: serde_json::json!([]),
        parallel_tool_calls: true,
        metadata: BTreeMap::new(),
    }))
    .unwrap();

    let expected = ModelApiRequest {
        api_kind: ModelApiKind::ChatCompletions,
        endpoint_path: "chat/completions".to_string(),
        body: serde_json::json!({
            "model": "fake-model",
            "messages": [
                { "role": "user", "content": "hello" }
            ],
            "tools": [],
            "parallel_tool_calls": true,
        }),
        response_mapper: ProtocolResponseMapperKind::ChatCompletions,
    };
    assert_eq!(actual, expected);
}

#[test]
fn tool_repair_decision_builds_effective_call_without_changing_identity() {
    let original = ToolCall {
        call_id: "call-1".to_string(),
        tool_name: "fake_tool".to_string(),
        source: ToolCallSource::Model,
        arguments: serde_json::json!({ "path": 123 }),
    };
    let repaired_arguments = serde_json::json!({ "path": "/tmp/file.txt" });
    let decision = ToolCallDecision::Repair {
        repaired_arguments: repaired_arguments.clone(),
    };

    let actual = decision.effective_call(&original);

    let expected = Some(ToolCall {
        call_id: "call-1".to_string(),
        tool_name: "fake_tool".to_string(),
        source: ToolCallSource::Model,
        arguments: repaired_arguments,
    });
    assert_eq!(actual, expected);
}

#[test]
fn tool_repair_decision_builds_repair_record_from_original_call() {
    let original = ToolCall {
        call_id: "call-1".to_string(),
        tool_name: "fake_tool".to_string(),
        source: ToolCallSource::Model,
        arguments: serde_json::json!({ "path": 123 }),
    };
    let repaired_arguments = serde_json::json!({ "path": "/tmp/file.txt" });
    let decision = ToolCallDecision::Repair {
        repaired_arguments: repaired_arguments.clone(),
    };

    let actual = decision.repair_record(&original);

    let expected = Some(ToolCallRepairRecord {
        call_id: "call-1".to_string(),
        tool_name: "fake_tool".to_string(),
        original_arguments: serde_json::json!({ "path": 123 }),
        repaired_arguments,
    });
    assert_eq!(actual, expected);
}

#[test]
fn runtime_extension_error_info_compares_as_a_whole_object() {
    let actual = RuntimeExtensionErrorInfo::new(
        RuntimeCapability::ToolMiddleware,
        "fake.tool_middleware",
        RuntimeExtensionPhase::ToolBeforeCall,
        "middleware returned repaired arguments that are not valid JSON for fake_tool",
        "the middleware produced arguments that no longer match the target tool schema",
        "return a JSON object matching the tool schema, or block the call",
        Some("tool-middleware-argument-repair"),
    );

    let expected = RuntimeExtensionErrorInfo {
        capability: RuntimeCapability::ToolMiddleware,
        contributor_id: RuntimeContributorId::new("fake.tool_middleware"),
        phase: RuntimeExtensionPhase::ToolBeforeCall,
        what_happened:
            "middleware returned repaired arguments that are not valid JSON for fake_tool"
                .to_string(),
        why_likely: "the middleware produced arguments that no longer match the target tool schema"
            .to_string(),
        how_to_fix: "return a JSON object matching the tool schema, or block the call".to_string(),
        docs_anchor: Some("tool-middleware-argument-repair".to_string()),
    };
    assert_eq!(actual, expected);
}

struct FakeModelRequestAdapter(&'static str);

impl ModelRequestAdapter for FakeModelRequestAdapter {
    fn id(&self) -> ModelRequestAdapterId {
        ModelRequestAdapterId::new(self.0)
    }

    async fn build_request(
        &self,
        input: ModelRequestAdapterInput,
    ) -> Result<ModelApiRequest, ModelRequestAdapterError> {
        Ok(ModelApiRequest {
            api_kind: input.api_kind,
            endpoint_path: "custom".to_string(),
            body: serde_json::json!({ "model": input.model }),
            response_mapper: ProtocolResponseMapperKind::Custom("fake".to_string()),
        })
    }
}

struct ChatCompletionsAdapter;

impl ModelRequestAdapter for ChatCompletionsAdapter {
    fn id(&self) -> ModelRequestAdapterId {
        ModelRequestAdapterId::new("fake.chat_completions_adapter")
    }

    async fn build_request(
        &self,
        input: ModelRequestAdapterInput,
    ) -> Result<ModelApiRequest, ModelRequestAdapterError> {
        Ok(ModelApiRequest {
            api_kind: ModelApiKind::ChatCompletions,
            endpoint_path: "chat/completions".to_string(),
            body: serde_json::json!({
                "model": input.model,
                "messages": input.input,
                "tools": input.tools,
                "parallel_tool_calls": input.parallel_tool_calls,
            }),
            response_mapper: ProtocolResponseMapperKind::ChatCompletions,
        })
    }
}

struct FakeContextContributor;

impl ContextContributor for FakeContextContributor {
    fn id(&self) -> ContextContributorId {
        ContextContributorId::new("fake.context_contributor")
    }

    async fn contribute(
        &self,
        _input: ContextContributorInput,
    ) -> Result<Vec<ContextBlock>, ContextError> {
        Ok(vec![ContextBlock {
            id: "stable-prefix".to_string(),
            slot: ContextBlockSlot::DeveloperPolicy,
            content: "cache prefix".to_string(),
            source: "test".to_string(),
            metadata: BTreeMap::new(),
        }])
    }
}

struct FakeContextPolicy;

impl ContextPolicy for FakeContextPolicy {
    fn id(&self) -> ContextPolicyId {
        ContextPolicyId::new("fake.context_policy")
    }

    async fn select_context(
        &self,
        input: ContextPolicyInput,
    ) -> Result<ContextPolicyDecision, ContextError> {
        Ok(ContextPolicyDecision {
            selected: input.candidates,
        })
    }
}

#[derive(Default)]
struct FakeContextObserver {
    observed: Arc<Mutex<Vec<ContextAssemblyObserverInput>>>,
}

impl ContextAssemblyObserver for FakeContextObserver {
    fn id(&self) -> ContextAssemblyObserverId {
        ContextAssemblyObserverId::new("fake.context_observer")
    }

    async fn observe(&self, input: ContextAssemblyObserverInput) -> Result<(), ContextError> {
        if let Ok(mut observed) = self.observed.lock() {
            observed.push(input);
        }
        Ok(())
    }
}

struct FakeToolMiddleware;

impl ToolMiddleware for FakeToolMiddleware {
    fn id(&self) -> ToolMiddlewareId {
        ToolMiddlewareId::new("fake.tool_middleware")
    }

    async fn before_tool_call(
        &self,
        _call: ToolCall,
    ) -> Result<ToolCallDecision, ToolMiddlewareError> {
        Ok(ToolCallDecision::Continue)
    }

    async fn after_tool_call(
        &self,
        _call: ToolCall,
        _result: ToolResult,
    ) -> Result<ToolResultDecision, ToolMiddlewareError> {
        Ok(ToolResultDecision::Preserve)
    }
}

struct FakeUsageMapper;

impl UsageMetadataMapper for FakeUsageMapper {
    fn id(&self) -> UsageMetadataMapperId {
        UsageMetadataMapperId::new("fake.usage_mapper")
    }

    async fn map_usage_metadata(
        &self,
        input: UsageMetadataMapperInput,
    ) -> Result<Option<UsageMetadata>, UsageMetadataMapperError> {
        Ok(input.fallback_usage)
    }
}
