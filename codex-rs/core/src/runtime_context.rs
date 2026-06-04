use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use codex_runtime_api::ContextBlock;
use codex_runtime_api::ContextBlockSlot;
use codex_runtime_api::ContextCandidate;
use codex_runtime_api::ContextCandidateSource;
use codex_runtime_api::ContextContributorInput;
use codex_runtime_api::ContextPolicyInput;
use codex_runtime_api::RuntimeCapability;
use codex_runtime_api::RuntimeExtensionErrorInfo;
use codex_runtime_api::RuntimeExtensionPhase;
use codex_runtime_api::RuntimeRegistry;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;

const MAX_CONTEXT_BLOCK_BYTES: usize = 40_000;

#[derive(Default)]
pub(crate) struct RuntimeContextSections {
    pub(crate) developer_policy: Vec<String>,
    pub(crate) developer_capabilities: Vec<String>,
    pub(crate) contextual_user: Vec<String>,
    pub(crate) separate_developer: Vec<String>,
}

pub(crate) async fn contribute_initial_context_sections(
    registry: &RuntimeRegistry,
    turn_id: &str,
) -> CodexResult<RuntimeContextSections> {
    let blocks = registry
        .contribute_context(ContextContributorInput {
            turn_id: turn_id.to_string(),
            metadata: BTreeMap::new(),
        })
        .await
        .map_err(|error| CodexErr::InvalidRequest(error.to_string()))?;
    validate_context_blocks(registry, &blocks)?;

    Ok(blocks
        .into_iter()
        .fold(RuntimeContextSections::default(), |mut sections, block| {
            push_context_block(&mut sections, block);
            sections
        }))
}

pub(crate) async fn select_prompt_input(
    registry: &RuntimeRegistry,
    turn_id: &str,
    input: Vec<ResponseItem>,
    token_budget: Option<u32>,
) -> CodexResult<Vec<ResponseItem>> {
    let candidates = input
        .iter()
        .enumerate()
        .map(|(index, item)| context_candidate_for_item(index, input.len(), item))
        .collect::<Vec<_>>();
    let candidates_by_id = candidates
        .iter()
        .map(|candidate| (candidate.id.clone(), candidate.clone()))
        .collect::<HashMap<_, _>>();
    let selected = registry
        .select_context(ContextPolicyInput {
            candidates,
            token_budget,
        })
        .await
        .map_err(|error| CodexErr::InvalidRequest(error.to_string()))?
        .selected;

    let input_by_id = input
        .iter()
        .enumerate()
        .map(|(index, item)| (context_candidate_id(index), item.clone()))
        .collect::<HashMap<_, _>>();
    validate_context_policy_selection(turn_id, &input, &selected)?;
    let mut output = Vec::with_capacity(selected.len());
    for candidate in selected {
        let item = input_by_id.get(&candidate.id).ok_or_else(|| {
            CodexErr::InvalidRequest(format!(
                "runtime context policy selected unknown candidate id `{}` for turn `{turn_id}`",
                candidate.id
            ))
        })?;
        let original_candidate = candidates_by_id.get(&candidate.id).ok_or_else(|| {
            CodexErr::InvalidRequest(format!(
                "runtime context policy selected unknown candidate id `{}` for turn `{turn_id}`",
                candidate.id
            ))
        })?;
        output.push(response_item_for_selected_candidate(
            turn_id,
            &candidate,
            item,
            original_candidate,
        )?);
    }
    Ok(output)
}

fn push_context_block(sections: &mut RuntimeContextSections, block: ContextBlock) {
    match block.slot {
        ContextBlockSlot::DeveloperPolicy => sections.developer_policy.push(block.content),
        ContextBlockSlot::DeveloperCapabilities => {
            sections.developer_capabilities.push(block.content);
        }
        ContextBlockSlot::ContextualUser => sections.contextual_user.push(block.content),
        ContextBlockSlot::SeparateDeveloper => sections.separate_developer.push(block.content),
    }
}

fn validate_context_blocks(registry: &RuntimeRegistry, blocks: &[ContextBlock]) -> CodexResult<()> {
    for block in blocks {
        let size = block.content.len();
        if size > MAX_CONTEXT_BLOCK_BYTES {
            return Err(CodexErr::InvalidRequest(
                RuntimeExtensionErrorInfo::new(
                    RuntimeCapability::ContextContributor,
                    registry.context_contributor_id().to_string(),
                    RuntimeExtensionPhase::ContextContribution,
                    format!(
                        "context block `{}` is {size} bytes, exceeding {MAX_CONTEXT_BLOCK_BYTES} byte limit",
                        block.id
                    ),
                    "the contributor returned a block larger than the bounded prompt-fragment limit",
                    "split, summarize, or omit large contributor context blocks before prompt assembly",
                    Some("context-contributor-oversized-block"),
                )
                .to_string(),
            ));
        }
    }
    Ok(())
}

fn context_candidate_for_item(index: usize, total: usize, item: &ResponseItem) -> ContextCandidate {
    ContextCandidate {
        id: context_candidate_id(index),
        source: context_candidate_source(index, total, item),
        content: context_candidate_content(item),
    }
}

fn context_candidate_id(index: usize) -> String {
    format!("item-{index}")
}

fn context_candidate_source(
    index: usize,
    total: usize,
    item: &ResponseItem,
) -> ContextCandidateSource {
    match item {
        ResponseItem::Message { role, .. } if role == "user" && index + 1 == total => {
            ContextCandidateSource::CurrentUserInput
        }
        ResponseItem::FunctionCall { .. }
        | ResponseItem::CustomToolCall { .. }
        | ResponseItem::ToolSearchCall { .. }
        | ResponseItem::FunctionCallOutput { .. }
        | ResponseItem::CustomToolCallOutput { .. }
        | ResponseItem::ToolSearchOutput { .. } => ContextCandidateSource::ToolCallResultPair,
        _ => ContextCandidateSource::History,
    }
}

fn context_candidate_content(item: &ResponseItem) -> String {
    match item {
        ResponseItem::Message { content, .. } => content
            .iter()
            .filter_map(content_item_text)
            .collect::<Vec<_>>()
            .join("\n"),
        ResponseItem::FunctionCall {
            name, arguments, ..
        } => {
            format!("{name}({arguments})")
        }
        ResponseItem::FunctionCallOutput { output, .. }
        | ResponseItem::CustomToolCallOutput { output, .. } => function_output_text(output),
        ResponseItem::CustomToolCall { name, input, .. } => {
            format!("{name}({input})")
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn content_item_text(item: &ContentItem) -> Option<String> {
    match item {
        ContentItem::InputText { text } | ContentItem::OutputText { text } => Some(text.clone()),
        ContentItem::InputImage { .. } => None,
    }
}

fn function_output_text(output: &FunctionCallOutputPayload) -> String {
    output.body.to_text().unwrap_or_default()
}

fn response_item_for_selected_candidate(
    turn_id: &str,
    candidate: &ContextCandidate,
    original: &ResponseItem,
    original_candidate: &ContextCandidate,
) -> CodexResult<ResponseItem> {
    candidate_index_from_id(&candidate.id, turn_id)?;
    if candidate.source != original_candidate.source {
        return Err(CodexErr::InvalidRequest(format!(
            "runtime context policy changed candidate `{}` source for turn `{turn_id}`",
            candidate.id
        )));
    }
    if candidate.content == original_candidate.content {
        return Ok(original.clone());
    }
    match candidate.source {
        ContextCandidateSource::History => Ok(ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: candidate.content.clone(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }),
        ContextCandidateSource::CurrentUserInput => Err(CodexErr::InvalidRequest(format!(
            "runtime context policy cannot rewrite current user input candidate `{}` for turn `{turn_id}`",
            candidate.id
        ))),
        ContextCandidateSource::ToolCallResultPair => Err(CodexErr::InvalidRequest(format!(
            "runtime context policy cannot rewrite tool call/result candidate `{}` for turn `{turn_id}`",
            candidate.id
        ))),
        ContextCandidateSource::Contributor
        | ContextCandidateSource::ClientAdditionalContext
        | ContextCandidateSource::Environment => Err(CodexErr::InvalidRequest(format!(
            "runtime context policy cannot rewrite non-history candidate `{}` for turn `{turn_id}`",
            candidate.id
        ))),
    }
}

fn validate_context_policy_selection(
    turn_id: &str,
    input: &[ResponseItem],
    selected: &[ContextCandidate],
) -> CodexResult<()> {
    let selected_ids = selected
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect::<HashSet<_>>();
    for (index, item) in input.iter().enumerate() {
        let id = context_candidate_id(index);
        if matches!(
            context_candidate_source(index, input.len(), item),
            ContextCandidateSource::CurrentUserInput
        ) && !selected_ids.contains(id.as_str())
        {
            return Err(CodexErr::InvalidRequest(format!(
                "runtime context policy removed current user input candidate `{id}` for turn `{turn_id}`"
            )));
        }
    }

    let selected_pairs = selected_tool_pair_keys(input, &selected_ids);
    for pair in tool_pair_keys(input) {
        if selected_pairs.contains(&pair)
            && !tool_pair_is_fully_selected(input, &selected_ids, &pair)
        {
            return Err(CodexErr::InvalidRequest(format!(
                "runtime context policy selected only part of tool call/result pair `{}` for turn `{turn_id}`",
                pair.1
            )));
        }
    }
    Ok(())
}

fn candidate_index_from_id(id: &str, turn_id: &str) -> CodexResult<usize> {
    id.strip_prefix("item-")
        .and_then(|index| index.parse::<usize>().ok())
        .ok_or_else(|| {
            CodexErr::InvalidRequest(format!(
                "runtime context policy selected invalid candidate id `{id}` for turn `{turn_id}`"
            ))
        })
}

fn selected_tool_pair_keys(
    input: &[ResponseItem],
    selected_ids: &HashSet<&str>,
) -> HashSet<(ToolPairKind, String)> {
    input
        .iter()
        .enumerate()
        .filter(|(index, _)| selected_ids.contains(context_candidate_id(*index).as_str()))
        .filter_map(|(_, item)| tool_pair_key(item))
        .collect()
}

fn tool_pair_keys(input: &[ResponseItem]) -> HashSet<(ToolPairKind, String)> {
    input.iter().filter_map(tool_pair_key).collect()
}

fn tool_pair_is_fully_selected(
    input: &[ResponseItem],
    selected_ids: &HashSet<&str>,
    pair: &(ToolPairKind, String),
) -> bool {
    input.iter().enumerate().all(|(index, item)| {
        tool_pair_key(item).as_ref() != Some(pair)
            || selected_ids.contains(context_candidate_id(index).as_str())
    })
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ToolPairKind {
    Function,
    Custom,
    Search,
}

fn tool_pair_key(item: &ResponseItem) -> Option<(ToolPairKind, String)> {
    match item {
        ResponseItem::FunctionCall { call_id, .. }
        | ResponseItem::FunctionCallOutput { call_id, .. } => {
            Some((ToolPairKind::Function, call_id.clone()))
        }
        ResponseItem::CustomToolCall { call_id, .. }
        | ResponseItem::CustomToolCallOutput { call_id, .. } => {
            Some((ToolPairKind::Custom, call_id.clone()))
        }
        ResponseItem::ToolSearchCall {
            call_id: Some(call_id),
            ..
        }
        | ResponseItem::ToolSearchOutput {
            call_id: Some(call_id),
            ..
        } => Some((ToolPairKind::Search, call_id.clone())),
        ResponseItem::ToolSearchCall { call_id: None, .. }
        | ResponseItem::ToolSearchOutput { call_id: None, .. }
        | ResponseItem::Message { .. }
        | ResponseItem::AdditionalTools { .. }
        | ResponseItem::AgentMessage { .. }
        | ResponseItem::Reasoning { .. }
        | ResponseItem::LocalShellCall { .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::ImageGenerationCall { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::CompactionTrigger { .. }
        | ResponseItem::ContextCompaction { .. }
        | ResponseItem::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_runtime_api::ContextContributor;
    use codex_runtime_api::ContextContributorId;
    use codex_runtime_api::ContextError;
    use codex_runtime_api::ContextPolicy;
    use codex_runtime_api::ContextPolicyDecision;
    use codex_runtime_api::ContextPolicyId;
    use codex_runtime_api::RuntimeCapability;
    use codex_runtime_api::RuntimeExtensionErrorInfo;
    use codex_runtime_api::RuntimeExtensionPhase;
    use pretty_assertions::assert_eq;

    struct SelectSecondCandidatePolicy;

    impl ContextPolicy for SelectSecondCandidatePolicy {
        fn id(&self) -> ContextPolicyId {
            ContextPolicyId::new("test.select_second")
        }

        async fn select_context(
            &self,
            input: ContextPolicyInput,
        ) -> std::result::Result<ContextPolicyDecision, ContextError> {
            Ok(ContextPolicyDecision {
                selected: vec![input.candidates[1].clone()],
            })
        }
    }

    struct SummaryReplacementPolicy;

    impl ContextPolicy for SummaryReplacementPolicy {
        fn id(&self) -> ContextPolicyId {
            ContextPolicyId::new("test.summary_replacement")
        }

        async fn select_context(
            &self,
            input: ContextPolicyInput,
        ) -> std::result::Result<ContextPolicyDecision, ContextError> {
            let mut selected = input.candidates;
            selected[0].content = "context-summary: old parser trace".to_string();
            Ok(ContextPolicyDecision { selected })
        }
    }

    struct OrphanToolResultPolicy;
    struct OversizedContextContributor;

    impl ContextPolicy for OrphanToolResultPolicy {
        fn id(&self) -> ContextPolicyId {
            ContextPolicyId::new("test.orphan_tool_result")
        }

        async fn select_context(
            &self,
            input: ContextPolicyInput,
        ) -> std::result::Result<ContextPolicyDecision, ContextError> {
            Ok(ContextPolicyDecision {
                selected: vec![input.candidates[1].clone(), input.candidates[2].clone()],
            })
        }
    }

    impl ContextContributor for OversizedContextContributor {
        fn id(&self) -> ContextContributorId {
            ContextContributorId::new("test.oversized_context_contributor")
        }

        async fn contribute(
            &self,
            _input: ContextContributorInput,
        ) -> std::result::Result<Vec<ContextBlock>, ContextError> {
            Ok(vec![ContextBlock {
                id: "too-large".to_string(),
                slot: ContextBlockSlot::DeveloperPolicy,
                content: "x".repeat(MAX_CONTEXT_BLOCK_BYTES + 1),
                source: "test".to_string(),
                metadata: BTreeMap::new(),
            }])
        }
    }

    fn user_message(text: &str) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: text.to_string(),
            }],
            phase: None,
        }
    }

    #[tokio::test]
    async fn runtime_context_policy_selects_prompt_items_without_rewriting_them() {
        let mut builder = RuntimeRegistry::builder();
        builder
            .context_policy(SelectSecondCandidatePolicy)
            .expect("register context policy");
        let input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "drop me".to_string(),
                }],
                phase: None,
            },
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "keep me".to_string(),
                }],
                phase: None,
            },
        ];

        let actual = select_prompt_input(
            &builder.build(),
            "turn-test",
            input.clone(),
            /*token_budget*/ None,
        )
        .await
        .expect("select prompt input");

        assert_eq!(actual, vec![input[1].clone()]);
    }

    #[tokio::test]
    async fn runtime_context_default_policy_preserves_stock_prompt_items() {
        let input = vec![user_message("history"), user_message("current")];

        let actual = select_prompt_input(
            &RuntimeRegistry::default(),
            "turn-test",
            input.clone(),
            /*token_budget*/ None,
        )
        .await
        .expect("select prompt input");

        assert_eq!(actual, input);
    }

    #[tokio::test]
    async fn runtime_context_policy_replaces_old_history_with_summary() {
        let mut builder = RuntimeRegistry::builder();
        builder
            .context_policy(SummaryReplacementPolicy)
            .expect("register context policy");
        let input = vec![
            user_message("OLD LONG TRACE SHOULD BE REPLACED"),
            user_message("Fix the parser test."),
        ];

        let actual = select_prompt_input(
            &builder.build(),
            "turn-test",
            input,
            /*token_budget*/ None,
        )
        .await
        .expect("select prompt input");

        assert_eq!(
            actual,
            vec![
                user_message("context-summary: old parser trace"),
                user_message("Fix the parser test."),
            ]
        );
    }

    #[tokio::test]
    async fn runtime_context_policy_rejects_orphaned_tool_result_pair() {
        let mut builder = RuntimeRegistry::builder();
        builder
            .context_policy(OrphanToolResultPolicy)
            .expect("register context policy");
        let input = vec![
            ResponseItem::FunctionCall {
                id: None,
                name: "shell".to_string(),
                namespace: None,
                arguments: "{\"cmd\":\"test\"}".to_string(),
                call_id: "call-1".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call-1".to_string(),
                output: FunctionCallOutputPayload::from_text("ok".to_string()),
            },
            user_message("Fix the parser test."),
        ];

        let err = select_prompt_input(
            &builder.build(),
            "turn-test",
            input,
            /*token_budget*/ None,
        )
        .await
        .expect_err("orphaned tool result should fail");

        match err {
            CodexErr::InvalidRequest(message) => assert_eq!(
                message,
                "runtime context policy selected only part of tool call/result pair `call-1` for turn `turn-test`"
            ),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    async fn runtime_context_contributor_rejects_oversized_block() {
        let mut builder = RuntimeRegistry::builder();
        builder
            .context_contributor(OversizedContextContributor)
            .expect("register oversized context contributor");

        let err = match contribute_initial_context_sections(&builder.build(), "turn-test").await {
            Ok(_) => panic!("oversized context block should fail"),
            Err(err) => err,
        };

        match err {
            CodexErr::InvalidRequest(message) => assert_eq!(
                message,
                "ContextContributor `test.oversized_context_contributor` failed during ContextContribution: context block `too-large` is 40001 bytes, exceeding 40000 byte limit. Likely cause: the contributor returned a block larger than the bounded prompt-fragment limit. Fix: split, summarize, or omit large contributor context blocks before prompt assembly. Docs: context-contributor-oversized-block"
            ),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    async fn runtime_context_policy_error_surfaces_runtime_extension_info() {
        struct FailingPolicy;

        impl ContextPolicy for FailingPolicy {
            fn id(&self) -> ContextPolicyId {
                ContextPolicyId::new("test.failing_policy")
            }

            async fn select_context(
                &self,
                _input: ContextPolicyInput,
            ) -> std::result::Result<ContextPolicyDecision, ContextError> {
                Err(RuntimeExtensionErrorInfo::new(
                    RuntimeCapability::ContextPolicy,
                    "test.failing_policy",
                    RuntimeExtensionPhase::ContextSelection,
                    "policy rejected candidate graph",
                    "the policy rejected or produced an invalid context candidate graph",
                    "keep current user input and complete tool/result pairs",
                    Some("context-policy-selection-error"),
                ))
            }
        }

        let mut builder = RuntimeRegistry::builder();
        builder
            .context_policy(FailingPolicy)
            .expect("register context policy");

        let err = select_prompt_input(
            &builder.build(),
            "turn-test",
            vec![user_message("Fix the parser test.")],
            /*token_budget*/ None,
        )
        .await
        .expect_err("failing policy should surface");

        match err {
            CodexErr::InvalidRequest(message) => assert_eq!(
                message,
                "ContextPolicy `test.failing_policy` failed during ContextSelection: policy rejected candidate graph. Likely cause: the policy rejected or produced an invalid context candidate graph. Fix: keep current user input and complete tool/result pairs. Docs: context-policy-selection-error"
            ),
            other => panic!("unexpected error: {other}"),
        }
    }
}
