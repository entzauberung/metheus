use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_RECENT_MODEL_CALLS: usize = 500;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ModelCallPurpose {
    Decision,
    Review,
    Execution,
    Recovery,
    Replan,
    Constitution,
    HumanTriggered,
    MilestoneGeneration,
    MilestoneCheck,
    MidStageGeneration,
    MidStageCheck,
    ExecutionPlanGeneration,
    ExecutionPlanCheck,
    TaskCalibration,
    EvidenceSupplement,
    VisionReview,
    SchemaRepair,
    ConstitutionSummary,
    ConstitutionCompression,
    PreflightCheck,
    VersionPlanGeneration,
    ExistingProjectAnalysis,
    Discussion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelCallContext {
    #[serde(default)]
    pub project_name: String,
    #[serde(default)]
    pub milestone_id: String,
    #[serde(default)]
    pub stage_id: String,
    #[serde(default)]
    pub task_id: String,
    #[serde(default)]
    pub purpose: Option<ModelCallPurpose>,
    #[serde(default)]
    pub decision_id: String,
    #[serde(default)]
    pub action_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ProviderUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelCallMetadata {
    pub call_id: String,
    pub context: ModelCallContext,
    pub model: String,
    pub provider_response_id: String,
    pub started_at: String,
    pub ended_at: String,
    pub elapsed_ms: u64,
    pub usage: Option<ProviderUsage>,
    pub failure_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelCallResponse {
    pub content: String,
    pub metadata: ModelCallMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelCallRecord {
    pub call_id: String,
    pub task_id: String,
    pub stage_id: String,
    #[serde(default)]
    pub milestone_id: String,
    pub purpose: Option<ModelCallPurpose>,
    pub model: String,
    #[serde(default)]
    pub provider: String,
    pub started_at: String,
    pub ended_at: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub elapsed_ms: Option<u64>,
    pub cache_hit: bool,
    pub produced_change: bool,
    pub produced_evidence: bool,
    pub produced_plan: bool,
    pub no_progress: bool,
    pub failure_kind: String,
    #[serde(default)]
    pub decision_id: String,
    #[serde(default)]
    pub action_id: String,
    #[serde(default)]
    pub provider_response_id: String,
    #[serde(default)]
    pub produced_contract: bool,
    #[serde(default)]
    pub produced_fact: bool,
    #[serde(default)]
    pub duplicate_reason: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ModelCallOutcome {
    pub produced_change: bool,
    pub produced_evidence: bool,
    pub produced_plan: bool,
    pub produced_contract: bool,
    pub produced_fact: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TokenCostSummary {
    pub calls: u32,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    #[serde(default)]
    pub known_input_tokens: u64,
    #[serde(default)]
    pub known_output_tokens: u64,
    #[serde(default)]
    pub known_total_tokens: u64,
    #[serde(default)]
    pub usage_known_calls: u32,
    #[serde(default)]
    pub usage_unknown_calls: u32,
    pub effective_calls: u32,
    pub no_progress_calls: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CostGroupSummary {
    pub key: String,
    pub summary: TokenCostSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ArchivedModelCallRecord {
    pub call_id: String,
    pub task_id: String,
    pub stage_id: String,
    #[serde(default)]
    pub milestone_id: String,
    pub purpose: Option<ModelCallPurpose>,
    #[serde(default)]
    pub provider: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub no_progress: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CostLedger {
    #[serde(default)]
    pub calls: Vec<ModelCallRecord>,
    #[serde(default)]
    pub archived_calls: Vec<ArchivedModelCallRecord>,
    #[serde(default)]
    pub project_summary: TokenCostSummary,
    #[serde(default)]
    pub soft_budget_level: String,
}

impl CostLedger {
    pub fn record(&mut self, call: ModelCallRecord) {
        self.record_without_rebuild(call);
        self.rebuild_summaries();
    }

    fn record_without_rebuild(&mut self, call: ModelCallRecord) {
        if let Some(existing) = self
            .calls
            .iter_mut()
            .find(|existing| existing.call_id == call.call_id)
        {
            merge_record(existing, &call);
            return;
        }
        if let Some(existing) = self
            .archived_calls
            .iter_mut()
            .find(|existing| existing.call_id == call.call_id)
        {
            merge_archived_record(existing, &call);
            return;
        }
        self.calls.push(call);
        self.archive_excess();
    }

    fn archive_excess(&mut self) {
        if self.calls.len() <= MAX_RECENT_MODEL_CALLS {
            return;
        }
        let excess = self.calls.len() - MAX_RECENT_MODEL_CALLS;
        let archived = self.calls.drain(0..excess).collect::<Vec<_>>();
        for call in archived {
            self.merge_archived(ArchivedModelCallRecord::from(&call));
        }
    }

    fn merge_archived(&mut self, incoming: ArchivedModelCallRecord) {
        if let Some(existing) = self
            .calls
            .iter_mut()
            .find(|existing| existing.call_id == incoming.call_id)
        {
            merge_record_from_archive(existing, &incoming);
            return;
        }
        if let Some(existing) = self
            .archived_calls
            .iter_mut()
            .find(|existing| existing.call_id == incoming.call_id)
        {
            merge_archive(existing, &incoming);
            return;
        }
        self.archived_calls.push(incoming);
    }

    pub fn merge_from(&mut self, other: &CostLedger) {
        for call in &other.archived_calls {
            self.merge_archived(call.clone());
        }
        for call in &other.calls {
            self.record_without_rebuild(call.clone());
        }
        self.archive_excess();
        self.rebuild_summaries();
    }

    pub fn rebuild_summaries(&mut self) {
        self.archive_excess();
        self.project_summary = summarize_entries(self.summary_entries().into_iter());
    }

    pub fn summary_for_task(&self, task_id: &str) -> TokenCostSummary {
        summarize_entries(
            self.summary_entries()
                .into_iter()
                .filter(|call| call.task_id == task_id),
        )
    }

    pub fn summary_for_stage(&self, stage_id: &str) -> TokenCostSummary {
        summarize_entries(
            self.summary_entries()
                .into_iter()
                .filter(|call| call.stage_id == stage_id),
        )
    }

    pub fn summaries_by_provider(&self) -> Vec<CostGroupSummary> {
        group_summaries(self.summary_entries(), |entry| entry.provider.to_string())
    }

    pub fn summaries_by_purpose(&self) -> Vec<CostGroupSummary> {
        group_summaries(self.summary_entries(), |entry| {
            entry
                .purpose
                .map(|purpose| format!("{:?}", purpose))
                .unwrap_or_else(|| "HistoricalUnknown".to_string())
        })
    }

    pub fn mark_outcome(&mut self, call_id: &str, outcome: ModelCallOutcome) -> bool {
        if let Some(call) = self.calls.iter_mut().find(|call| call.call_id == call_id) {
            apply_outcome(call, outcome);
            self.rebuild_summaries();
            return true;
        }
        if let Some(call) = self
            .archived_calls
            .iter_mut()
            .find(|call| call.call_id == call_id)
        {
            if outcome_made_progress(outcome) {
                call.no_progress = false;
            }
            self.rebuild_summaries();
            return true;
        }
        false
    }

    fn summary_entries(&self) -> Vec<CostSummaryEntry<'_>> {
        self.calls
            .iter()
            .map(CostSummaryEntry::from)
            .chain(self.archived_calls.iter().map(CostSummaryEntry::from))
            .collect()
    }
}

impl ModelCallContext {
    pub fn for_project(project: &crate::project::Project, purpose: ModelCallPurpose) -> Self {
        let stage_id = crate::plan_scope::PlanScope::resolve(project)
            .map(|scope| scope.target_id(project).to_string())
            .unwrap_or_default();
        Self {
            project_name: project.name.clone(),
            milestone_id: project.current_milestone_id.clone(),
            stage_id,
            task_id: crate::task_tree::select_current_leaf(project)
                .ok()
                .flatten()
                .map(|address| address.task_id)
                .unwrap_or_default(),
            purpose: Some(purpose),
            decision_id: project.task_control.last_decision_id.clone(),
            action_id: project.task_control.active_action_id.clone(),
        }
    }
}

impl ModelCallRecord {
    fn provider_name(&self) -> &str {
        if !self.provider.is_empty() {
            &self.provider
        } else if self.purpose == Some(ModelCallPurpose::Execution) && !self.model.is_empty() {
            &self.model
        } else {
            "历史/未知"
        }
    }
}

impl ArchivedModelCallRecord {
    fn provider_name(&self) -> &str {
        if self.provider.is_empty() {
            "历史/未知"
        } else {
            &self.provider
        }
    }
}

impl From<&ModelCallRecord> for ArchivedModelCallRecord {
    fn from(call: &ModelCallRecord) -> Self {
        Self {
            call_id: call.call_id.clone(),
            task_id: call.task_id.clone(),
            stage_id: call.stage_id.clone(),
            milestone_id: call.milestone_id.clone(),
            purpose: call.purpose,
            provider: call.provider_name().to_string(),
            input_tokens: call.input_tokens,
            output_tokens: call.output_tokens,
            total_tokens: call.total_tokens,
            no_progress: call.no_progress,
        }
    }
}

#[derive(Clone, Copy)]
struct CostSummaryEntry<'a> {
    task_id: &'a str,
    stage_id: &'a str,
    provider: &'a str,
    purpose: Option<ModelCallPurpose>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    no_progress: bool,
}

impl<'a> From<&'a ModelCallRecord> for CostSummaryEntry<'a> {
    fn from(call: &'a ModelCallRecord) -> Self {
        Self {
            task_id: &call.task_id,
            stage_id: &call.stage_id,
            provider: call.provider_name(),
            purpose: call.purpose,
            input_tokens: call.input_tokens,
            output_tokens: call.output_tokens,
            total_tokens: call.total_tokens,
            no_progress: call.no_progress,
        }
    }
}

impl<'a> From<&'a ArchivedModelCallRecord> for CostSummaryEntry<'a> {
    fn from(call: &'a ArchivedModelCallRecord) -> Self {
        Self {
            task_id: &call.task_id,
            stage_id: &call.stage_id,
            provider: call.provider_name(),
            purpose: call.purpose,
            input_tokens: call.input_tokens,
            output_tokens: call.output_tokens,
            total_tokens: call.total_tokens,
            no_progress: call.no_progress,
        }
    }
}

fn merge_archived_record(existing: &mut ArchivedModelCallRecord, incoming: &ModelCallRecord) {
    if existing.task_id.is_empty() {
        existing.task_id = incoming.task_id.clone();
    }
    if existing.stage_id.is_empty() {
        existing.stage_id = incoming.stage_id.clone();
    }
    if existing.milestone_id.is_empty() {
        existing.milestone_id = incoming.milestone_id.clone();
    }
    if existing.purpose.is_none() {
        existing.purpose = incoming.purpose;
    }
    if existing.provider.is_empty() || existing.provider == "历史/未知" {
        existing.provider = incoming.provider_name().to_string();
    }
    existing.input_tokens = existing.input_tokens.or(incoming.input_tokens);
    existing.output_tokens = existing.output_tokens.or(incoming.output_tokens);
    existing.total_tokens = existing.total_tokens.or(incoming.total_tokens);
    existing.no_progress &= incoming.no_progress;
}

fn merge_record_from_archive(existing: &mut ModelCallRecord, incoming: &ArchivedModelCallRecord) {
    if existing.task_id.is_empty() {
        existing.task_id = incoming.task_id.clone();
    }
    if existing.stage_id.is_empty() {
        existing.stage_id = incoming.stage_id.clone();
    }
    if existing.milestone_id.is_empty() {
        existing.milestone_id = incoming.milestone_id.clone();
    }
    if existing.purpose.is_none() {
        existing.purpose = incoming.purpose;
    }
    if existing.provider.is_empty() && incoming.provider != "历史/未知" {
        existing.provider = incoming.provider.clone();
    }
    existing.input_tokens = existing.input_tokens.or(incoming.input_tokens);
    existing.output_tokens = existing.output_tokens.or(incoming.output_tokens);
    existing.total_tokens = existing.total_tokens.or(incoming.total_tokens);
    existing.no_progress &= incoming.no_progress;
}

fn merge_archive(existing: &mut ArchivedModelCallRecord, incoming: &ArchivedModelCallRecord) {
    if existing.task_id.is_empty() {
        existing.task_id = incoming.task_id.clone();
    }
    if existing.stage_id.is_empty() {
        existing.stage_id = incoming.stage_id.clone();
    }
    if existing.milestone_id.is_empty() {
        existing.milestone_id = incoming.milestone_id.clone();
    }
    if existing.purpose.is_none() {
        existing.purpose = incoming.purpose;
    }
    if existing.provider.is_empty() || existing.provider == "历史/未知" {
        existing.provider = incoming.provider.clone();
    }
    existing.input_tokens = existing.input_tokens.or(incoming.input_tokens);
    existing.output_tokens = existing.output_tokens.or(incoming.output_tokens);
    existing.total_tokens = existing.total_tokens.or(incoming.total_tokens);
    existing.no_progress &= incoming.no_progress;
}

fn merge_record(existing: &mut ModelCallRecord, incoming: &ModelCallRecord) {
    existing.produced_change |= incoming.produced_change;
    existing.produced_evidence |= incoming.produced_evidence;
    existing.produced_plan |= incoming.produced_plan;
    existing.produced_contract |= incoming.produced_contract;
    existing.produced_fact |= incoming.produced_fact;
    existing.no_progress &= incoming.no_progress;
    if existing.input_tokens.is_none() {
        existing.input_tokens = incoming.input_tokens;
    }
    if existing.output_tokens.is_none() {
        existing.output_tokens = incoming.output_tokens;
    }
    if existing.total_tokens.is_none() {
        existing.total_tokens = incoming.total_tokens;
    }
    if existing.ended_at.is_empty() {
        existing.ended_at = incoming.ended_at.clone();
    }
    if existing.failure_kind.is_empty() {
        existing.failure_kind = incoming.failure_kind.clone();
    }
    if existing.duplicate_reason.is_empty() {
        existing.duplicate_reason = incoming.duplicate_reason.clone();
    }
}

pub fn record_from_metadata(ledger: &mut CostLedger, metadata: &ModelCallMetadata) {
    let usage = metadata.usage.as_ref();
    ledger.record(ModelCallRecord {
        call_id: metadata.call_id.clone(),
        task_id: metadata.context.task_id.clone(),
        stage_id: metadata.context.stage_id.clone(),
        milestone_id: metadata.context.milestone_id.clone(),
        purpose: metadata.context.purpose,
        model: metadata.model.clone(),
        provider: "OpenAI Compatible".to_string(),
        started_at: metadata.started_at.clone(),
        ended_at: metadata.ended_at.clone(),
        input_tokens: usage.and_then(|usage| usage.input_tokens),
        output_tokens: usage.and_then(|usage| usage.output_tokens),
        total_tokens: usage.and_then(|usage| usage.total_tokens),
        elapsed_ms: Some(metadata.elapsed_ms),
        cache_hit: usage
            .and_then(|usage| usage.cached_input_tokens)
            .is_some_and(|tokens| tokens > 0),
        no_progress: true,
        failure_kind: metadata.failure_kind.clone(),
        decision_id: metadata.context.decision_id.clone(),
        action_id: metadata.context.action_id.clone(),
        provider_response_id: metadata.provider_response_id.clone(),
        ..Default::default()
    });
}

#[allow(clippy::too_many_arguments)]
pub fn record_execution_call(
    ledger: &mut CostLedger,
    call_id: &str,
    context: &ModelCallContext,
    provider: &str,
    model: &str,
    started_at: String,
    ended_at: String,
    elapsed_ms: u64,
    usage: Option<&ProviderUsage>,
    produced_change: bool,
    failure_kind: &str,
) {
    ledger.record(ModelCallRecord {
        call_id: call_id.to_string(),
        task_id: context.task_id.clone(),
        stage_id: context.stage_id.clone(),
        milestone_id: context.milestone_id.clone(),
        purpose: Some(ModelCallPurpose::Execution),
        model: model.to_string(),
        provider: provider.to_string(),
        started_at,
        ended_at,
        input_tokens: usage.and_then(|usage| usage.input_tokens),
        output_tokens: usage.and_then(|usage| usage.output_tokens),
        total_tokens: usage.and_then(|usage| usage.total_tokens),
        elapsed_ms: Some(elapsed_ms),
        cache_hit: false,
        produced_change,
        no_progress: !produced_change,
        failure_kind: failure_kind.to_string(),
        ..Default::default()
    });
}

#[allow(clippy::too_many_arguments)]
pub fn record_execution_call_best_effort(
    project_name: &str,
    call_id: &str,
    context: &ModelCallContext,
    provider: &str,
    model: &str,
    started_at: String,
    elapsed_ms: u64,
    usage: Option<&ProviderUsage>,
    produced_change: bool,
    failure_kind: &str,
) {
    let _ = crate::mutate_project_for_control(project_name, |project| {
        record_execution_call(
            &mut project.cost_ledger,
            call_id,
            context,
            provider,
            model,
            started_at,
            chrono::Utc::now().to_rfc3339(),
            elapsed_ms,
            usage,
            produced_change,
            failure_kind,
        );
        Ok(((), true))
    });
}

pub fn record_metadata_best_effort(metadata: &ModelCallMetadata) {
    if metadata.context.project_name.is_empty() {
        return;
    }
    let Ok(mut project) = crate::load_project(&metadata.context.project_name) else {
        return;
    };
    record_from_metadata(&mut project.cost_ledger, metadata);
    let _ = crate::save_project(&project);
}

pub fn mark_call_outcome_best_effort(project_name: &str, call_id: &str, outcome: ModelCallOutcome) {
    let Ok(mut project) = crate::load_project(project_name) else {
        return;
    };
    if !project.cost_ledger.mark_outcome(call_id, outcome) {
        return;
    }
    let _ = crate::save_project(&project);
}

fn outcome_made_progress(outcome: ModelCallOutcome) -> bool {
    outcome.produced_change
        || outcome.produced_evidence
        || outcome.produced_plan
        || outcome.produced_contract
        || outcome.produced_fact
}

fn apply_outcome(call: &mut ModelCallRecord, outcome: ModelCallOutcome) {
    call.produced_change |= outcome.produced_change;
    call.produced_evidence |= outcome.produced_evidence;
    call.produced_plan |= outcome.produced_plan;
    call.produced_contract |= outcome.produced_contract;
    call.produced_fact |= outcome.produced_fact;
    call.no_progress = !(call.produced_change
        || call.produced_evidence
        || call.produced_plan
        || call.produced_contract
        || call.produced_fact);
}

#[derive(Default)]
struct SummaryAccumulator {
    calls: u32,
    input_all_known: bool,
    output_all_known: bool,
    total_all_known: bool,
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    known_input_tokens: u64,
    known_output_tokens: u64,
    known_total_tokens: u64,
    usage_known_calls: u32,
    usage_unknown_calls: u32,
    effective_calls: u32,
    no_progress_calls: u32,
}

impl SummaryAccumulator {
    fn new() -> Self {
        Self {
            input_all_known: true,
            output_all_known: true,
            total_all_known: true,
            ..Self::default()
        }
    }

    fn push(&mut self, entry: CostSummaryEntry<'_>) {
        self.calls = self.calls.saturating_add(1);
        match entry.input_tokens {
            Some(tokens) => {
                self.input_tokens = self.input_tokens.saturating_add(tokens);
                self.known_input_tokens = self.known_input_tokens.saturating_add(tokens);
            }
            None => self.input_all_known = false,
        }
        match entry.output_tokens {
            Some(tokens) => {
                self.output_tokens = self.output_tokens.saturating_add(tokens);
                self.known_output_tokens = self.known_output_tokens.saturating_add(tokens);
            }
            None => self.output_all_known = false,
        }
        let resolved_total = entry.total_tokens.or_else(|| {
            entry
                .input_tokens
                .zip(entry.output_tokens)
                .map(|(input, output)| input.saturating_add(output))
        });
        match resolved_total {
            Some(tokens) => {
                self.total_tokens = self.total_tokens.saturating_add(tokens);
                self.known_total_tokens = self.known_total_tokens.saturating_add(tokens);
                self.usage_known_calls = self.usage_known_calls.saturating_add(1);
            }
            None => {
                self.total_all_known = false;
                self.usage_unknown_calls = self.usage_unknown_calls.saturating_add(1);
            }
        }
        if entry.no_progress {
            self.no_progress_calls = self.no_progress_calls.saturating_add(1);
        } else {
            self.effective_calls = self.effective_calls.saturating_add(1);
        }
    }

    fn finish(self) -> TokenCostSummary {
        TokenCostSummary {
            calls: self.calls,
            input_tokens: self.input_all_known.then_some(self.input_tokens),
            output_tokens: self.output_all_known.then_some(self.output_tokens),
            total_tokens: self.total_all_known.then_some(self.total_tokens),
            known_input_tokens: self.known_input_tokens,
            known_output_tokens: self.known_output_tokens,
            known_total_tokens: self.known_total_tokens,
            usage_known_calls: self.usage_known_calls,
            usage_unknown_calls: self.usage_unknown_calls,
            effective_calls: self.effective_calls,
            no_progress_calls: self.no_progress_calls,
        }
    }
}

fn summarize_entries<'a>(entries: impl Iterator<Item = CostSummaryEntry<'a>>) -> TokenCostSummary {
    let mut summary = SummaryAccumulator::new();
    for entry in entries {
        summary.push(entry);
    }
    summary.finish()
}

fn group_summaries<'a>(
    entries: Vec<CostSummaryEntry<'a>>,
    key: impl Fn(CostSummaryEntry<'a>) -> String,
) -> Vec<CostGroupSummary> {
    let mut groups = BTreeMap::<String, SummaryAccumulator>::new();
    for entry in entries {
        groups
            .entry(key(entry))
            .or_insert_with(SummaryAccumulator::new)
            .push(entry);
    }
    groups
        .into_iter()
        .map(|(key, summary)| CostGroupSummary {
            key,
            summary: summary.finish(),
        })
        .collect()
}

pub fn summarize(calls: &[ModelCallRecord]) -> TokenCostSummary {
    summarize_entries(calls.iter().map(CostSummaryEntry::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_plan_uses_milestone_identity_for_stage_costs() {
        let mut project = crate::project::Project::new("quick-cost");
        project.workload_profile = Some(
            crate::workload_policy::classify(
                crate::project::WorkloadSignals {
                    has_frontend: true,
                    has_backend: false,
                    has_persistence: false,
                    has_auth_or_roles: false,
                    external_integration_count: 0,
                    independent_domain_count: 1,
                    deliverable_count: 2,
                    high_risk: false,
                },
                None,
                0,
            )
            .unwrap(),
        );
        project.current_milestone_id = "milestone-1".to_string();
        project.milestones.push(crate::project::Milestone {
            id: "milestone-1".to_string(),
            mode: crate::project::StageMode::Quick,
            ..Default::default()
        });

        let context = ModelCallContext::for_project(&project, ModelCallPurpose::Execution);
        assert_eq!(context.milestone_id, "milestone-1");
        assert_eq!(context.stage_id, "milestone-1");
    }

    #[test]
    fn missing_provider_usage_remains_unknown() {
        let summary = summarize(&[ModelCallRecord::default()]);
        assert_eq!(summary.total_tokens, None);
        assert_eq!(summary.known_total_tokens, 0);
        assert_eq!(summary.usage_known_calls, 0);
        assert_eq!(summary.usage_unknown_calls, 1);
    }

    #[test]
    fn runtime_fix_grok_execution_records_usage_and_task_context() {
        let mut ledger = CostLedger::default();
        record_execution_call(
            &mut ledger,
            "execution-1",
            &ModelCallContext {
                project_name: "project".to_string(),
                milestone_id: "milestone".to_string(),
                stage_id: "stage".to_string(),
                task_id: "leaf".to_string(),
                ..Default::default()
            },
            "Grok Build",
            "grok-code-fast-1",
            "2026-08-03T00:00:00Z".to_string(),
            "2026-08-03T00:00:01Z".to_string(),
            1_000,
            Some(&ProviderUsage {
                input_tokens: Some(12),
                output_tokens: Some(5),
                total_tokens: Some(17),
                cached_input_tokens: None,
            }),
            true,
            "",
        );

        assert_eq!(ledger.calls.len(), 1);
        let call = &ledger.calls[0];
        assert_eq!(call.provider, "Grok Build");
        assert_eq!(call.model, "grok-code-fast-1");
        assert_eq!(call.task_id, "leaf");
        assert_eq!(call.total_tokens, Some(17));
        assert_eq!(call.elapsed_ms, Some(1_000));
        assert_eq!(ledger.summary_for_task("leaf").known_total_tokens, 17);
    }

    #[test]
    fn runtime_fix_grok_execution_without_usage_stays_unknown() {
        let mut ledger = CostLedger::default();
        record_execution_call(
            &mut ledger,
            "execution-2",
            &ModelCallContext {
                task_id: "leaf".to_string(),
                ..Default::default()
            },
            "Grok Build",
            "grok-code-fast-1",
            String::new(),
            String::new(),
            25,
            None,
            false,
            "",
        );

        assert_eq!(ledger.calls[0].total_tokens, None);
        assert_eq!(ledger.project_summary.usage_unknown_calls, 1);
        assert_eq!(ledger.calls[0].elapsed_ms, Some(25));
    }

    #[test]
    fn mixed_usage_preserves_known_totals_and_coverage() {
        let summary = summarize(&[
            ModelCallRecord {
                input_tokens: Some(6),
                output_tokens: Some(4),
                total_tokens: Some(10),
                ..Default::default()
            },
            ModelCallRecord::default(),
        ]);
        assert_eq!(summary.total_tokens, None);
        assert_eq!(summary.known_input_tokens, 6);
        assert_eq!(summary.known_output_tokens, 4);
        assert_eq!(summary.known_total_tokens, 10);
        assert_eq!(summary.usage_known_calls, 1);
        assert_eq!(summary.usage_unknown_calls, 1);
    }

    #[test]
    fn useful_and_no_progress_calls_are_distinguished() {
        let mut ledger = CostLedger::default();
        ledger.record(ModelCallRecord {
            call_id: "1".into(),
            no_progress: true,
            total_tokens: Some(10),
            input_tokens: Some(6),
            output_tokens: Some(4),
            ..Default::default()
        });
        assert_eq!(ledger.project_summary.effective_calls, 0);
        assert_eq!(ledger.project_summary.no_progress_calls, 1);
    }

    #[test]
    fn duplicate_call_ids_merge_without_double_counting() {
        let mut ledger = CostLedger::default();
        ledger.record(ModelCallRecord {
            call_id: "same-request".into(),
            no_progress: true,
            total_tokens: Some(10),
            ..Default::default()
        });
        ledger.record(ModelCallRecord {
            call_id: "same-request".into(),
            no_progress: false,
            produced_evidence: true,
            ..Default::default()
        });
        assert_eq!(ledger.calls.len(), 1);
        assert_eq!(ledger.project_summary.calls, 1);
        assert!(ledger.calls[0].produced_evidence);
        assert!(!ledger.calls[0].no_progress);
        assert_eq!(ledger.calls[0].total_tokens, Some(10));
    }

    #[test]
    fn failed_metadata_is_no_progress_until_an_outcome_is_recorded() {
        let mut ledger = CostLedger::default();
        record_from_metadata(
            &mut ledger,
            &ModelCallMetadata {
                call_id: "failed-request".into(),
                failure_kind: "StreamFailed".into(),
                ..Default::default()
            },
        );
        assert_eq!(ledger.calls.len(), 1);
        assert!(ledger.calls[0].no_progress);
        assert_eq!(ledger.project_summary.no_progress_calls, 1);
        assert_eq!(ledger.project_summary.effective_calls, 0);
    }

    #[test]
    fn the_501st_call_archives_the_oldest_without_changing_lifetime_totals() {
        let mut ledger = CostLedger::default();
        for index in 0..=MAX_RECENT_MODEL_CALLS {
            ledger.record(ModelCallRecord {
                call_id: format!("call-{index}"),
                task_id: "task".to_string(),
                stage_id: "stage".to_string(),
                purpose: Some(ModelCallPurpose::Review),
                provider: if index == 0 {
                    "Archived Provider".to_string()
                } else {
                    "Recent Provider".to_string()
                },
                input_tokens: Some(1),
                output_tokens: Some(1),
                total_tokens: Some(2),
                ..Default::default()
            });
        }

        assert_eq!(ledger.calls.len(), MAX_RECENT_MODEL_CALLS);
        assert_eq!(ledger.archived_calls.len(), 1);
        assert_eq!(ledger.project_summary.calls, 501);
        assert_eq!(ledger.project_summary.known_total_tokens, 1002);
        assert_eq!(ledger.summary_for_task("task").calls, 501);
        assert_eq!(
            ledger
                .summaries_by_provider()
                .iter()
                .find(|group| group.key == "Archived Provider")
                .unwrap()
                .summary
                .calls,
            1
        );
        assert_eq!(ledger.summaries_by_purpose()[0].summary.calls, 501);
    }

    #[test]
    fn recent_and_archived_merges_are_idempotent() {
        let mut source = CostLedger::default();
        for index in 0..=MAX_RECENT_MODEL_CALLS {
            source.record(ModelCallRecord {
                call_id: format!("merge-{index}"),
                total_tokens: Some(1),
                ..Default::default()
            });
        }
        let stale_oldest = CostLedger {
            calls: vec![ModelCallRecord {
                call_id: "merge-0".to_string(),
                total_tokens: Some(1),
                ..Default::default()
            }],
            ..Default::default()
        };

        source.merge_from(&stale_oldest);
        source.merge_from(&stale_oldest);
        assert_eq!(source.calls.len(), MAX_RECENT_MODEL_CALLS);
        assert_eq!(source.archived_calls.len(), 1);
        assert_eq!(source.project_summary.calls, 501);
        assert_eq!(source.project_summary.known_total_tokens, 501);
    }

    #[test]
    fn archived_call_outcome_can_still_be_updated() {
        let mut ledger = CostLedger::default();
        for index in 0..=MAX_RECENT_MODEL_CALLS {
            ledger.record(ModelCallRecord {
                call_id: format!("outcome-{index}"),
                no_progress: true,
                ..Default::default()
            });
        }
        assert!(ledger.mark_outcome(
            "outcome-0",
            ModelCallOutcome {
                produced_evidence: true,
                ..Default::default()
            }
        ));
        assert!(!ledger.archived_calls[0].no_progress);
        assert_eq!(ledger.project_summary.effective_calls, 1);
        assert_eq!(ledger.project_summary.no_progress_calls, 500);
    }

    #[test]
    fn old_cost_json_loads_and_rebuilds_new_summaries() {
        let mut ledger = CostLedger::default();
        ledger.record(ModelCallRecord {
            call_id: "old-execution".to_string(),
            purpose: Some(ModelCallPurpose::Execution),
            model: "Claude Code".to_string(),
            total_tokens: Some(7),
            ..Default::default()
        });
        let mut value = serde_json::to_value(&ledger).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("archived_calls");
        object["calls"][0]
            .as_object_mut()
            .unwrap()
            .remove("provider");
        let summary = object["project_summary"].as_object_mut().unwrap();
        for field in [
            "known_input_tokens",
            "known_output_tokens",
            "known_total_tokens",
            "usage_known_calls",
            "usage_unknown_calls",
        ] {
            summary.remove(field);
        }

        let mut restored: CostLedger = serde_json::from_value(value).unwrap();
        restored.rebuild_summaries();
        assert_eq!(restored.project_summary.known_total_tokens, 7);
        assert_eq!(restored.project_summary.usage_known_calls, 1);
        assert_eq!(restored.summaries_by_provider()[0].key, "Claude Code");
    }
}
