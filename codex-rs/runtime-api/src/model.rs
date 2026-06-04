use crate::ModelRequestAdapterError;
use crate::ModelRequestAdapterId;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum ModelApiKind {
    Responses,
    ChatCompletions,
    Messages,
    Custom(String),
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ModelRequestAdapterInput {
    pub provider: String,
    pub model: String,
    pub api_kind: ModelApiKind,
    pub body: Value,
    pub instructions: String,
    pub input: Value,
    pub tools: Value,
    pub parallel_tool_calls: bool,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ModelApiRequest {
    pub api_kind: ModelApiKind,
    pub endpoint_path: String,
    pub body: Value,
    pub response_mapper: ProtocolResponseMapperKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum ProtocolResponseMapperKind {
    Responses,
    ChatCompletions,
    Messages,
    Custom(String),
}

/// Builds the provider API request envelope from normalized Codex model input.
///
/// Implementations choose API family and request body shape. They must not own
/// transport, authentication, retry, cancellation, telemetry, or stream
/// backpressure.
pub trait ModelRequestAdapter: Send + Sync + 'static {
    fn id(&self) -> ModelRequestAdapterId;

    fn build_request(
        &self,
        input: ModelRequestAdapterInput,
    ) -> impl std::future::Future<Output = Result<ModelApiRequest, ModelRequestAdapterError>> + Send;
}

pub struct DefaultModelRequestAdapter;

impl ModelRequestAdapter for DefaultModelRequestAdapter {
    fn id(&self) -> ModelRequestAdapterId {
        ModelRequestAdapterId::new("codex.default.model_request_adapter")
    }

    async fn build_request(
        &self,
        input: ModelRequestAdapterInput,
    ) -> Result<ModelApiRequest, ModelRequestAdapterError> {
        Ok(ModelApiRequest {
            api_kind: ModelApiKind::Responses,
            endpoint_path: "responses".to_string(),
            body: input.body,
            response_mapper: ProtocolResponseMapperKind::Responses,
        })
    }
}
