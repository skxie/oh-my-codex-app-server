use crate::ContextAssemblyObserverId;
use crate::ContextContributorId;
use crate::ContextError;
use crate::ContextPolicyId;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum ContextBlockSlot {
    DeveloperPolicy,
    DeveloperCapabilities,
    ContextualUser,
    SeparateDeveloper,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ContextBlock {
    pub id: String,
    pub slot: ContextBlockSlot,
    pub content: String,
    pub source: String,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ContextContributorInput {
    pub turn_id: String,
    pub metadata: BTreeMap<String, String>,
}

/// Adds stable context blocks before context policy selects final candidates.
///
/// Implementations own their internal cache, memory, retrieval, or project
/// context composition. The registry exposes one active contributor.
pub trait ContextContributor: Send + Sync + 'static {
    fn id(&self) -> ContextContributorId;

    fn contribute(
        &self,
        input: ContextContributorInput,
    ) -> impl std::future::Future<Output = Result<Vec<ContextBlock>, ContextError>> + Send;
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum ContextCandidateSource {
    CurrentUserInput,
    History,
    ToolCallResultPair,
    Contributor,
    ClientAdditionalContext,
    Environment,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ContextCandidate {
    pub id: String,
    pub source: ContextCandidateSource,
    pub content: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ContextPolicyInput {
    pub candidates: Vec<ContextCandidate>,
    pub token_budget: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ContextPolicyDecision {
    pub selected: Vec<ContextCandidate>,
}

/// Selects or replaces context/history candidates before final prompt assembly.
///
/// Implementations must preserve semantic invariants, such as current user
/// input and tool-call/result pairing, before returning a decision.
pub trait ContextPolicy: Send + Sync + 'static {
    fn id(&self) -> ContextPolicyId;

    fn select_context(
        &self,
        input: ContextPolicyInput,
    ) -> impl std::future::Future<Output = Result<ContextPolicyDecision, ContextError>> + Send;
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ContextAssemblyObserverInput {
    pub provider_bound_input: Value,
    pub metadata: BTreeMap<String, String>,
}

/// Observes the final provider-bound model input without mutating it.
pub trait ContextAssemblyObserver: Send + Sync + 'static {
    fn id(&self) -> ContextAssemblyObserverId;

    fn observe(
        &self,
        input: ContextAssemblyObserverInput,
    ) -> impl std::future::Future<Output = Result<(), ContextError>> + Send;
}

pub struct DefaultContextContributor;

impl ContextContributor for DefaultContextContributor {
    fn id(&self) -> ContextContributorId {
        ContextContributorId::new("codex.default.context_contributor")
    }

    async fn contribute(
        &self,
        _input: ContextContributorInput,
    ) -> Result<Vec<ContextBlock>, ContextError> {
        Ok(Vec::new())
    }
}

pub struct DefaultContextPolicy;

impl ContextPolicy for DefaultContextPolicy {
    fn id(&self) -> ContextPolicyId {
        ContextPolicyId::new("codex.default.context_policy")
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
