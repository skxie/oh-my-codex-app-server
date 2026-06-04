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
    pub why_likely: String,
    pub how_to_fix: String,
    pub docs_anchor: Option<String>,
}

impl RuntimeExtensionErrorInfo {
    pub fn new(
        capability: RuntimeCapability,
        contributor_id: impl Into<RuntimeContributorId>,
        phase: RuntimeExtensionPhase,
        what_happened: impl Into<String>,
        why_likely: impl Into<String>,
        how_to_fix: impl Into<String>,
        docs_anchor: Option<&str>,
    ) -> Self {
        Self {
            capability,
            contributor_id: contributor_id.into(),
            phase,
            what_happened: what_happened.into(),
            why_likely: why_likely.into(),
            how_to_fix: how_to_fix.into(),
            docs_anchor: docs_anchor.map(ToString::to_string),
        }
    }
}

impl std::fmt::Display for RuntimeExtensionErrorInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} `{}` failed during {:?}: {}. Likely cause: {}. Fix: {}",
            self.capability,
            self.contributor_id,
            self.phase,
            self.what_happened,
            self.why_likely,
            self.how_to_fix
        )?;
        if let Some(docs_anchor) = &self.docs_anchor {
            write!(f, ". Docs: {docs_anchor}")?;
        }
        Ok(())
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

impl RuntimeRegistryBuildError {
    pub fn into_error_info(self) -> RuntimeExtensionErrorInfo {
        match self {
            RuntimeRegistryBuildError::DuplicateCapability {
                capability,
                existing_contributor_id,
                attempted_contributor_id,
            } => RuntimeExtensionErrorInfo::new(
                capability,
                attempted_contributor_id,
                RuntimeExtensionPhase::Registration,
                format!(
                    "runtime capability already has implementation `{existing_contributor_id}`"
                ),
                "the builder registered the same runtime capability more than once",
                "register only one active implementation for this runtime capability",
                Some("runtime-registry-duplicate-capability"),
            ),
        }
    }
}

impl From<RuntimeRegistryBuildError> for RuntimeExtensionErrorInfo {
    fn from(value: RuntimeRegistryBuildError) -> Self {
        value.into_error_info()
    }
}
