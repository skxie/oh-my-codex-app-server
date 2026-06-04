use crate::ToolMiddlewareError;
use crate::ToolMiddlewareId;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum ToolCallSource {
    Model,
    Client,
    Runtime,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ToolCall {
    pub call_id: String,
    pub tool_name: String,
    pub source: ToolCallSource,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ToolCallRepairRecord {
    pub call_id: String,
    pub tool_name: String,
    pub original_arguments: Value,
    pub repaired_arguments: Value,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum ToolCallDecision {
    Continue,
    Repair { repaired_arguments: Value },
    Block { reason: String },
}

impl ToolCallDecision {
    pub fn effective_call(&self, original_call: &ToolCall) -> Option<ToolCall> {
        match self {
            ToolCallDecision::Continue => Some(original_call.clone()),
            ToolCallDecision::Repair { repaired_arguments } => Some(ToolCall {
                arguments: repaired_arguments.clone(),
                ..original_call.clone()
            }),
            ToolCallDecision::Block { reason: _ } => None,
        }
    }

    pub fn repair_record(&self, original_call: &ToolCall) -> Option<ToolCallRepairRecord> {
        match self {
            ToolCallDecision::Repair { repaired_arguments } => Some(ToolCallRepairRecord {
                call_id: original_call.call_id.clone(),
                tool_name: original_call.tool_name.clone(),
                original_arguments: original_call.arguments.clone(),
                repaired_arguments: repaired_arguments.clone(),
            }),
            ToolCallDecision::Continue | ToolCallDecision::Block { reason: _ } => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum ToolResultStatus {
    Success,
    Failed,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ToolResult {
    pub call_id: String,
    pub status: ToolResultStatus,
    pub output: Value,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum ToolResultDecision {
    Preserve,
    Replace(ToolResult),
}

/// Runs runtime-control decisions around tool dispatch.
///
/// Implementations may repair, block, or normalize calls/results. They must not
/// bypass approval, sandbox, executor ownership, or change call identity.
pub trait ToolMiddleware: Send + Sync + 'static {
    fn id(&self) -> ToolMiddlewareId;

    fn before_tool_call(
        &self,
        call: ToolCall,
    ) -> impl std::future::Future<Output = Result<ToolCallDecision, ToolMiddlewareError>> + Send;

    fn after_tool_call(
        &self,
        call: ToolCall,
        result: ToolResult,
    ) -> impl std::future::Future<Output = Result<ToolResultDecision, ToolMiddlewareError>> + Send;
}

pub struct DefaultToolMiddleware;

impl ToolMiddleware for DefaultToolMiddleware {
    fn id(&self) -> ToolMiddlewareId {
        ToolMiddlewareId::new("codex.default.tool_middleware")
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
