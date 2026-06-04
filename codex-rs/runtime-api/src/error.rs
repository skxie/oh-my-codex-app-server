use crate::RuntimeContributorId;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum RuntimeCapability {
    ModelRequestAdapter,
    ProtocolResponseMapper,
    ContextContributor,
    ContextPolicy,
    ContextAssemblyObserver,
    ToolMiddleware,
    UsageMetadataMapper,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum RuntimeExtensionPhase {
    Registration,
    ModelRequest,
    ProtocolResponseMapping,
    ContextContribution,
    ContextSelection,
    ContextObservation,
    ToolBeforeCall,
    ToolAfterCall,
    UsageMapping,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RuntimeExtensionErrorInfo {
    pub capability: RuntimeCapability,
    pub contributor_id: RuntimeContributorId,
    pub phase: RuntimeExtensionPhase,
    pub what_happened: String,
    pub how_to_fix: String,
}

impl RuntimeExtensionErrorInfo {
    pub fn new(
        capability: RuntimeCapability,
        contributor_id: impl Into<RuntimeContributorId>,
        phase: RuntimeExtensionPhase,
        what_happened: impl Into<String>,
        how_to_fix: impl Into<String>,
    ) -> Self {
        Self {
            capability,
            contributor_id: contributor_id.into(),
            phase,
            what_happened: what_happened.into(),
            how_to_fix: how_to_fix.into(),
        }
    }
}

impl std::fmt::Display for RuntimeExtensionErrorInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} failed during {:?}: {}. Fix: {}",
            self.capability, self.phase, self.what_happened, self.how_to_fix
        )
    }
}

impl std::error::Error for RuntimeExtensionErrorInfo {}

pub type ModelRequestAdapterError = RuntimeExtensionErrorInfo;
pub type ContextError = RuntimeExtensionErrorInfo;
pub type ToolMiddlewareError = RuntimeExtensionErrorInfo;
pub type UsageMetadataMapperError = RuntimeExtensionErrorInfo;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RuntimeRegistryBuildError {
    #[error(
        "runtime capability {capability:?} already has implementation {existing_contributor_id}; attempted to register {attempted_contributor_id}"
    )]
    DuplicateCapability {
        capability: RuntimeCapability,
        existing_contributor_id: RuntimeContributorId,
        attempted_contributor_id: RuntimeContributorId,
    },
}
