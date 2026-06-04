use crate::ContextAssemblyObserver;
use crate::ContextAssemblyObserverId;
use crate::ContextAssemblyObserverInput;
use crate::ContextBlock;
use crate::ContextContributor;
use crate::ContextContributorId;
use crate::ContextContributorInput;
use crate::ContextError;
use crate::ContextPolicy;
use crate::ContextPolicyDecision;
use crate::ContextPolicyId;
use crate::ContextPolicyInput;
use crate::DefaultContextContributor;
use crate::DefaultContextPolicy;
use crate::DefaultModelRequestAdapter;
use crate::DefaultToolMiddleware;
use crate::DefaultUsageMetadataMapper;
use crate::ModelApiRequest;
use crate::ModelRequestAdapter;
use crate::ModelRequestAdapterError;
use crate::ModelRequestAdapterId;
use crate::ModelRequestAdapterInput;
use crate::RuntimeCapability;
use crate::RuntimeContributorId;
use crate::RuntimeRegistryBuildError;
use crate::ToolCall;
use crate::ToolCallDecision;
use crate::ToolMiddleware;
use crate::ToolMiddlewareError;
use crate::ToolMiddlewareId;
use crate::ToolResult;
use crate::ToolResultDecision;
use crate::UsageMetadata;
use crate::UsageMetadataMapper;
use crate::UsageMetadataMapperError;
use crate::UsageMetadataMapperId;
use crate::UsageMetadataMapperInput;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

trait ErasedModelRequestAdapter: Send + Sync {
    fn id(&self) -> ModelRequestAdapterId;

    fn build_request(
        &self,
        input: ModelRequestAdapterInput,
    ) -> BoxFuture<'_, Result<ModelApiRequest, ModelRequestAdapterError>>;
}

impl<T> ErasedModelRequestAdapter for T
where
    T: ModelRequestAdapter,
{
    fn id(&self) -> ModelRequestAdapterId {
        ModelRequestAdapter::id(self)
    }

    fn build_request(
        &self,
        input: ModelRequestAdapterInput,
    ) -> BoxFuture<'_, Result<ModelApiRequest, ModelRequestAdapterError>> {
        Box::pin(ModelRequestAdapter::build_request(self, input))
    }
}

trait ErasedContextContributor: Send + Sync {
    fn id(&self) -> ContextContributorId;

    fn contribute(
        &self,
        input: ContextContributorInput,
    ) -> BoxFuture<'_, Result<Vec<ContextBlock>, ContextError>>;
}

impl<T> ErasedContextContributor for T
where
    T: ContextContributor,
{
    fn id(&self) -> ContextContributorId {
        ContextContributor::id(self)
    }

    fn contribute(
        &self,
        input: ContextContributorInput,
    ) -> BoxFuture<'_, Result<Vec<ContextBlock>, ContextError>> {
        Box::pin(ContextContributor::contribute(self, input))
    }
}

trait ErasedContextPolicy: Send + Sync {
    fn id(&self) -> ContextPolicyId;

    fn select_context(
        &self,
        input: ContextPolicyInput,
    ) -> BoxFuture<'_, Result<ContextPolicyDecision, ContextError>>;
}

impl<T> ErasedContextPolicy for T
where
    T: ContextPolicy,
{
    fn id(&self) -> ContextPolicyId {
        ContextPolicy::id(self)
    }

    fn select_context(
        &self,
        input: ContextPolicyInput,
    ) -> BoxFuture<'_, Result<ContextPolicyDecision, ContextError>> {
        Box::pin(ContextPolicy::select_context(self, input))
    }
}

trait ErasedContextAssemblyObserver: Send + Sync {
    fn id(&self) -> ContextAssemblyObserverId;

    fn observe(
        &self,
        input: ContextAssemblyObserverInput,
    ) -> BoxFuture<'_, Result<(), ContextError>>;
}

impl<T> ErasedContextAssemblyObserver for T
where
    T: ContextAssemblyObserver,
{
    fn id(&self) -> ContextAssemblyObserverId {
        ContextAssemblyObserver::id(self)
    }

    fn observe(
        &self,
        input: ContextAssemblyObserverInput,
    ) -> BoxFuture<'_, Result<(), ContextError>> {
        Box::pin(ContextAssemblyObserver::observe(self, input))
    }
}

trait ErasedToolMiddleware: Send + Sync {
    fn id(&self) -> ToolMiddlewareId;

    fn before_tool_call(
        &self,
        call: ToolCall,
    ) -> BoxFuture<'_, Result<ToolCallDecision, ToolMiddlewareError>>;

    fn after_tool_call(
        &self,
        call: ToolCall,
        result: ToolResult,
    ) -> BoxFuture<'_, Result<ToolResultDecision, ToolMiddlewareError>>;
}

impl<T> ErasedToolMiddleware for T
where
    T: ToolMiddleware,
{
    fn id(&self) -> ToolMiddlewareId {
        ToolMiddleware::id(self)
    }

    fn before_tool_call(
        &self,
        call: ToolCall,
    ) -> BoxFuture<'_, Result<ToolCallDecision, ToolMiddlewareError>> {
        Box::pin(ToolMiddleware::before_tool_call(self, call))
    }

    fn after_tool_call(
        &self,
        call: ToolCall,
        result: ToolResult,
    ) -> BoxFuture<'_, Result<ToolResultDecision, ToolMiddlewareError>> {
        Box::pin(ToolMiddleware::after_tool_call(self, call, result))
    }
}

trait ErasedUsageMetadataMapper: Send + Sync {
    fn id(&self) -> UsageMetadataMapperId;

    fn map_usage_metadata(
        &self,
        input: UsageMetadataMapperInput,
    ) -> BoxFuture<'_, Result<Option<UsageMetadata>, UsageMetadataMapperError>>;
}

impl<T> ErasedUsageMetadataMapper for T
where
    T: UsageMetadataMapper,
{
    fn id(&self) -> UsageMetadataMapperId {
        UsageMetadataMapper::id(self)
    }

    fn map_usage_metadata(
        &self,
        input: UsageMetadataMapperInput,
    ) -> BoxFuture<'_, Result<Option<UsageMetadata>, UsageMetadataMapperError>> {
        Box::pin(UsageMetadataMapper::map_usage_metadata(self, input))
    }
}

#[derive(Clone)]
pub struct RuntimeRegistry {
    model_request_adapter: Arc<dyn ErasedModelRequestAdapter>,
    context_contributor: Arc<dyn ErasedContextContributor>,
    context_policy: Arc<dyn ErasedContextPolicy>,
    context_assembly_observer: Option<Arc<dyn ErasedContextAssemblyObserver>>,
    tool_middleware: Arc<dyn ErasedToolMiddleware>,
    usage_metadata_mapper: Arc<dyn ErasedUsageMetadataMapper>,
}

impl fmt::Debug for RuntimeRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeRegistry")
            .field("model_request_adapter", &self.model_request_adapter_id())
            .field("context_contributor", &self.context_contributor_id())
            .field("context_policy", &self.context_policy_id())
            .field(
                "context_assembly_observer",
                &self.context_assembly_observer_id(),
            )
            .field("tool_middleware", &self.tool_middleware_id())
            .field("usage_metadata_mapper", &self.usage_metadata_mapper_id())
            .finish()
    }
}

impl RuntimeRegistry {
    pub fn builder() -> RuntimeRegistryBuilder {
        RuntimeRegistryBuilder::default()
    }

    pub fn model_request_adapter_id(&self) -> ModelRequestAdapterId {
        self.model_request_adapter.id()
    }

    pub fn context_contributor_id(&self) -> ContextContributorId {
        self.context_contributor.id()
    }

    pub fn context_policy_id(&self) -> ContextPolicyId {
        self.context_policy.id()
    }

    pub fn context_assembly_observer_id(&self) -> Option<ContextAssemblyObserverId> {
        self.context_assembly_observer
            .as_ref()
            .map(|observer| observer.id())
    }

    pub fn tool_middleware_id(&self) -> ToolMiddlewareId {
        self.tool_middleware.id()
    }

    pub fn usage_metadata_mapper_id(&self) -> UsageMetadataMapperId {
        self.usage_metadata_mapper.id()
    }

    pub async fn build_model_request(
        &self,
        input: ModelRequestAdapterInput,
    ) -> Result<ModelApiRequest, ModelRequestAdapterError> {
        self.model_request_adapter.build_request(input).await
    }

    pub async fn contribute_context(
        &self,
        input: ContextContributorInput,
    ) -> Result<Vec<ContextBlock>, ContextError> {
        self.context_contributor.contribute(input).await
    }

    pub async fn select_context(
        &self,
        input: ContextPolicyInput,
    ) -> Result<ContextPolicyDecision, ContextError> {
        self.context_policy.select_context(input).await
    }

    pub async fn observe_context(
        &self,
        input: ContextAssemblyObserverInput,
    ) -> Result<(), ContextError> {
        if let Some(observer) = &self.context_assembly_observer {
            observer.observe(input).await
        } else {
            Ok(())
        }
    }

    pub async fn before_tool_call(
        &self,
        call: ToolCall,
    ) -> Result<ToolCallDecision, ToolMiddlewareError> {
        self.tool_middleware.before_tool_call(call).await
    }

    pub async fn after_tool_call(
        &self,
        call: ToolCall,
        result: ToolResult,
    ) -> Result<ToolResultDecision, ToolMiddlewareError> {
        self.tool_middleware.after_tool_call(call, result).await
    }

    pub async fn map_usage_metadata(
        &self,
        input: UsageMetadataMapperInput,
    ) -> Result<Option<UsageMetadata>, UsageMetadataMapperError> {
        self.usage_metadata_mapper.map_usage_metadata(input).await
    }
}

impl Default for RuntimeRegistry {
    fn default() -> Self {
        RuntimeRegistry {
            model_request_adapter: Arc::new(DefaultModelRequestAdapter),
            context_contributor: Arc::new(DefaultContextContributor),
            context_policy: Arc::new(DefaultContextPolicy),
            context_assembly_observer: None,
            tool_middleware: Arc::new(DefaultToolMiddleware),
            usage_metadata_mapper: Arc::new(DefaultUsageMetadataMapper),
        }
    }
}

#[derive(Default)]
pub struct RuntimeRegistryBuilder {
    model_request_adapter: Option<Arc<dyn ErasedModelRequestAdapter>>,
    context_contributor: Option<Arc<dyn ErasedContextContributor>>,
    context_policy: Option<Arc<dyn ErasedContextPolicy>>,
    context_assembly_observer: Option<Arc<dyn ErasedContextAssemblyObserver>>,
    tool_middleware: Option<Arc<dyn ErasedToolMiddleware>>,
    usage_metadata_mapper: Option<Arc<dyn ErasedUsageMetadataMapper>>,
}

impl RuntimeRegistryBuilder {
    pub fn model_request_adapter(
        &mut self,
        adapter: impl ModelRequestAdapter,
    ) -> Result<&mut Self, RuntimeRegistryBuildError> {
        let adapter = Arc::new(adapter);
        if let Some(existing) = &self.model_request_adapter {
            return Err(duplicate_error(
                RuntimeCapability::ModelRequestAdapter,
                existing.id().to_string(),
                adapter.id().to_string(),
            ));
        }
        self.model_request_adapter = Some(adapter);
        Ok(self)
    }

    pub fn context_contributor(
        &mut self,
        contributor: impl ContextContributor,
    ) -> Result<&mut Self, RuntimeRegistryBuildError> {
        let contributor = Arc::new(contributor);
        if let Some(existing) = &self.context_contributor {
            return Err(duplicate_error(
                RuntimeCapability::ContextContributor,
                existing.id().to_string(),
                contributor.id().to_string(),
            ));
        }
        self.context_contributor = Some(contributor);
        Ok(self)
    }

    pub fn context_policy(
        &mut self,
        policy: impl ContextPolicy,
    ) -> Result<&mut Self, RuntimeRegistryBuildError> {
        let policy = Arc::new(policy);
        if let Some(existing) = &self.context_policy {
            return Err(duplicate_error(
                RuntimeCapability::ContextPolicy,
                existing.id().to_string(),
                policy.id().to_string(),
            ));
        }
        self.context_policy = Some(policy);
        Ok(self)
    }

    pub fn context_assembly_observer(
        &mut self,
        observer: impl ContextAssemblyObserver,
    ) -> Result<&mut Self, RuntimeRegistryBuildError> {
        let observer = Arc::new(observer);
        if let Some(existing) = &self.context_assembly_observer {
            return Err(duplicate_error(
                RuntimeCapability::ContextAssemblyObserver,
                existing.id().to_string(),
                observer.id().to_string(),
            ));
        }
        self.context_assembly_observer = Some(observer);
        Ok(self)
    }

    pub fn tool_middleware(
        &mut self,
        middleware: impl ToolMiddleware,
    ) -> Result<&mut Self, RuntimeRegistryBuildError> {
        let middleware = Arc::new(middleware);
        if let Some(existing) = &self.tool_middleware {
            return Err(duplicate_error(
                RuntimeCapability::ToolMiddleware,
                existing.id().to_string(),
                middleware.id().to_string(),
            ));
        }
        self.tool_middleware = Some(middleware);
        Ok(self)
    }

    pub fn usage_metadata_mapper(
        &mut self,
        mapper: impl UsageMetadataMapper,
    ) -> Result<&mut Self, RuntimeRegistryBuildError> {
        let mapper = Arc::new(mapper);
        if let Some(existing) = &self.usage_metadata_mapper {
            return Err(duplicate_error(
                RuntimeCapability::UsageMetadataMapper,
                existing.id().to_string(),
                mapper.id().to_string(),
            ));
        }
        self.usage_metadata_mapper = Some(mapper);
        Ok(self)
    }

    pub fn build(self) -> RuntimeRegistry {
        RuntimeRegistry {
            model_request_adapter: self
                .model_request_adapter
                .unwrap_or_else(|| Arc::new(DefaultModelRequestAdapter)),
            context_contributor: self
                .context_contributor
                .unwrap_or_else(|| Arc::new(DefaultContextContributor)),
            context_policy: self
                .context_policy
                .unwrap_or_else(|| Arc::new(DefaultContextPolicy)),
            context_assembly_observer: self.context_assembly_observer,
            tool_middleware: self
                .tool_middleware
                .unwrap_or_else(|| Arc::new(DefaultToolMiddleware)),
            usage_metadata_mapper: self
                .usage_metadata_mapper
                .unwrap_or_else(|| Arc::new(DefaultUsageMetadataMapper)),
        }
    }
}

fn duplicate_error(
    capability: RuntimeCapability,
    existing_contributor_id: String,
    attempted_contributor_id: String,
) -> RuntimeRegistryBuildError {
    RuntimeRegistryBuildError::DuplicateCapability {
        capability,
        existing_contributor_id: RuntimeContributorId::new(existing_contributor_id),
        attempted_contributor_id: RuntimeContributorId::new(attempted_contributor_id),
    }
}
