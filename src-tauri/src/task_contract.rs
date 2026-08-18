use crate::project;
use crate::validator_contract::VerificationMode;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const TASK_CONTRACT_VERSION: &str = "task-contract-v2";
const LEGACY_TASK_CONTRACT_VERSION: &str = "task-contract-v1";
pub(crate) const DEFAULT_MAX_WALL_CLOCK_SECS: u64 = 600;
const ABSOLUTE_MAX_WALL_CLOCK_SECS: u64 = 20 * 60;

fn default_max_wall_clock_secs() -> u64 {
    DEFAULT_MAX_WALL_CLOCK_SECS
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum TaskNodeType {
    Milestone,
    MidStage,
    #[default]
    Subtask,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum TaskComplexity {
    #[default]
    Small,
    Medium,
    Large,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum TaskRiskLevel {
    #[default]
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TaskArtifactConstraint {
    #[serde(default)]
    pub expected_files: Vec<String>,
    #[serde(default)]
    pub expected_identifiers: Vec<String>,
    #[serde(default)]
    pub completion_facts: Vec<String>,
    #[serde(default)]
    pub expected_artifacts: Vec<String>,
    #[serde(default)]
    pub related_symbols: Vec<String>,
    #[serde(default)]
    pub read_file_paths: Vec<String>,
    #[serde(default)]
    pub write_file_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TaskBudgetSummary {
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub estimated_model_calls: u32,
    #[serde(default)]
    pub estimated_input_tokens: u64,
    #[serde(default)]
    pub estimated_output_tokens: u64,
    pub max_executor_turns: u32,
    pub max_transport_retries: u32,
    pub max_doom_loop_retries: u32,
    /// Contract-level budget for the complete execution attempt. This is not
    /// reset by provider/API retries or continuation attempts.
    #[serde(default = "default_max_wall_clock_secs")]
    pub max_wall_clock_secs: u64,
}

impl TaskBudgetSummary {
    pub(crate) fn bounded_wall_clock_secs(&self) -> u64 {
        let requested = if self.max_wall_clock_secs == 0 {
            DEFAULT_MAX_WALL_CLOCK_SECS
        } else {
            self.max_wall_clock_secs
        };
        requested.min(ABSOLUTE_MAX_WALL_CLOCK_SECS)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionStopReason {
    Cancelled,
    WallClockExceeded,
    ResourceHardStop,
}

/// Shared cancellation/deadline state for an entire execution attempt.
/// Resource sampling may promote the decision while the attempt is running;
/// callers still own the actual child/future termination.
#[derive(Clone)]
pub(crate) struct ExecutionGuard {
    cancellation: Arc<AtomicBool>,
    resource_decision: Arc<AtomicU8>,
    resource_observation: Arc<Mutex<project::ResourceObservationSummary>>,
    deadline: Instant,
}

impl ExecutionGuard {
    pub(crate) fn new(
        budget: &TaskBudgetSummary,
        resource_decision: crate::runtime_resource::ResourceDecision,
    ) -> Self {
        Self {
            cancellation: Arc::new(AtomicBool::new(false)),
            resource_decision: Arc::new(AtomicU8::new(resource_decision_code(resource_decision))),
            resource_observation: Arc::new(Mutex::new(
                project::ResourceObservationSummary::default(),
            )),
            deadline: Instant::now() + Duration::from_secs(budget.bounded_wall_clock_secs()),
        }
    }

    pub(crate) fn cancellation(&self) -> Arc<AtomicBool> {
        self.cancellation.clone()
    }

    pub(crate) fn request_cancel(&self) {
        self.cancellation.store(true, Ordering::Release);
    }

    pub(crate) fn set_resource_decision(
        &self,
        decision: crate::runtime_resource::ResourceDecision,
    ) {
        let next = resource_decision_code(decision);
        loop {
            let current = self.resource_decision.load(Ordering::Acquire);
            if current
                == resource_decision_code(crate::runtime_resource::ResourceDecision::HardStop)
            {
                break;
            }
            if self
                .resource_decision
                .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break;
            }
        }
        if decision == crate::runtime_resource::ResourceDecision::HardStop {
            self.request_cancel();
        }
    }

    pub(crate) fn set_resource_observation(
        &self,
        observation: project::ResourceObservationSummary,
    ) {
        if let Ok(mut current) = self.resource_observation.lock() {
            if current.state == project::ResourceObservationState::HardStop
                && observation.state != project::ResourceObservationState::HardStop
            {
                return;
            }
            *current = observation;
        }
    }

    pub(crate) fn resource_observation(&self) -> project::ResourceObservationSummary {
        self.resource_observation
            .lock()
            .map(|observation| observation.clone())
            .unwrap_or_default()
    }

    pub(crate) fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    pub(crate) fn stop_reason(&self) -> Option<ExecutionStopReason> {
        if resource_decision_from_code(self.resource_decision.load(Ordering::Acquire))
            == crate::runtime_resource::ResourceDecision::HardStop
        {
            return Some(ExecutionStopReason::ResourceHardStop);
        }
        if self.remaining().is_zero() {
            return Some(ExecutionStopReason::WallClockExceeded);
        }
        if self.cancellation.load(Ordering::Acquire) {
            return Some(ExecutionStopReason::Cancelled);
        }
        None
    }
}

fn resource_decision_code(decision: crate::runtime_resource::ResourceDecision) -> u8 {
    match decision {
        crate::runtime_resource::ResourceDecision::Unknown => 0,
        crate::runtime_resource::ResourceDecision::Continue => 1,
        crate::runtime_resource::ResourceDecision::Warning => 2,
        crate::runtime_resource::ResourceDecision::HardStop => 3,
    }
}

fn resource_decision_from_code(code: u8) -> crate::runtime_resource::ResourceDecision {
    match code {
        1 => crate::runtime_resource::ResourceDecision::Continue,
        2 => crate::runtime_resource::ResourceDecision::Warning,
        3 => crate::runtime_resource::ResourceDecision::HardStop,
        _ => crate::runtime_resource::ResourceDecision::Unknown,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskContract {
    pub version: String,
    pub task_id: String,
    pub parent_task_id: Option<String>,
    pub depth: u32,
    pub node_type: TaskNodeType,
    pub workload_scale: project::WorkloadScale,
    pub workload_profile_fingerprint: String,
    pub max_split_depth: u32,
    pub title: String,
    pub goal: String,
    #[serde(default)]
    pub allowed_file_paths: Vec<String>,
    #[serde(default)]
    pub new_file_paths: Vec<String>,
    #[serde(default)]
    pub evidence_files: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria_meta: Vec<crate::provability::AcceptanceCriterion>,
    #[serde(default)]
    pub verification_modes: Vec<VerificationMode>,
    #[serde(default)]
    pub stop_rules: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub complexity: TaskComplexity,
    pub risk: TaskRiskLevel,
    pub artifacts: TaskArtifactConstraint,
    pub budget: TaskBudgetSummary,
    pub recommended_executor: String,
    pub plan_source: String,
    #[serde(default)]
    pub split_basis: String,
    #[serde(default)]
    pub estimated_complexity_reduction: u32,
    #[serde(default)]
    pub independently_verifiable: bool,
    #[serde(default)]
    pub future_parallel_safe: bool,
    pub compiled_at: String,
    pub fingerprint: String,
}

/// `contract_snapshot` is a rebuildable cache, but the surrounding task is not.
/// Only the known v1 cache may be discarded during project deserialization. Current
/// and unknown versions stay strict so damaged data cannot be silently accepted.
pub fn deserialize_contract_snapshot<'de, D>(
    deserializer: D,
) -> Result<Option<TaskContract>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(value) = Option::<serde_json::Value>::deserialize(deserializer)? else {
        return Ok(None);
    };
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| D::Error::custom("任务合同快照缺少字符串 version 字段"))?;
    match version {
        LEGACY_TASK_CONTRACT_VERSION => Ok(None),
        TASK_CONTRACT_VERSION => serde_json::from_value(value)
            .map(Some)
            .map_err(|error| D::Error::custom(format!("任务合同 v2 快照损坏：{error}"))),
        other => Err(D::Error::custom(format!(
            "不支持的任务合同快照版本：{other}"
        ))),
    }
}

pub fn compile_subtask(
    subtask: &project::Subtask,
    parent_task_id: Option<&str>,
    depth: u32,
    workload: &project::WorkloadProfile,
) -> TaskContract {
    let acceptance_criteria_meta = crate::provability::normalize_metadata(
        &subtask.acceptance_criteria,
        &subtask.acceptance_criteria_meta,
    );
    let verification_modes = acceptance_criteria_meta
        .iter()
        .map(|criterion| criterion.provability.verification_mode())
        .collect::<Vec<_>>();
    let artifacts = TaskArtifactConstraint {
        expected_files: subtask
            .new_file_paths
            .iter()
            .chain(subtask.allowed_file_paths.iter())
            .cloned()
            .collect(),
        expected_identifiers: subtask.required_identifiers.clone(),
        completion_facts: vec![format!("task:{} completed", subtask.id)],
        expected_artifacts: subtask.expected_artifacts.clone(),
        related_symbols: subtask.related_symbols.clone(),
        read_file_paths: if subtask.read_file_paths.is_empty() {
            subtask.evidence_files.clone()
        } else {
            subtask.read_file_paths.clone()
        },
        write_file_paths: if subtask.write_file_paths.is_empty() {
            subtask
                .allowed_file_paths
                .iter()
                .chain(subtask.new_file_paths.iter())
                .cloned()
                .collect()
        } else {
            subtask.write_file_paths.clone()
        },
    };
    let complexity = crate::task_complexity::estimate_complexity(subtask);
    let risk = crate::task_complexity::estimate_risk(subtask, complexity);
    let budget = crate::task_complexity::estimate_budget(subtask, complexity, workload);
    let mut contract = TaskContract {
        version: TASK_CONTRACT_VERSION.to_string(),
        task_id: subtask.id.clone(),
        parent_task_id: parent_task_id.map(str::to_string),
        depth,
        node_type: TaskNodeType::Subtask,
        workload_scale: workload.scale,
        workload_profile_fingerprint: workload.fingerprint.clone(),
        max_split_depth: workload.max_split_depth,
        title: subtask.title.clone(),
        goal: if subtask.goal.trim().is_empty() {
            subtask.prompt.clone()
        } else {
            subtask.goal.clone()
        },
        allowed_file_paths: subtask.allowed_file_paths.clone(),
        new_file_paths: subtask.new_file_paths.clone(),
        evidence_files: subtask.evidence_files.clone(),
        acceptance_criteria: subtask.acceptance_criteria.clone(),
        acceptance_criteria_meta,
        verification_modes,
        stop_rules: subtask.stop_rules.clone(),
        dependencies: subtask.depends_on.clone(),
        complexity,
        risk,
        artifacts,
        budget,
        recommended_executor: "serial_pipeline".to_string(),
        plan_source: "legacy_subtask".to_string(),
        split_basis: subtask.split_basis.clone(),
        estimated_complexity_reduction: 0,
        independently_verifiable: subtask.independently_verifiable,
        future_parallel_safe: subtask.future_parallel_safe,
        compiled_at: chrono::Utc::now().to_rfc3339(),
        fingerprint: String::new(),
    };
    contract.fingerprint = fingerprint(&contract);
    contract
}

fn fingerprint(contract: &TaskContract) -> String {
    let mut copy = contract.clone();
    copy.fingerprint.clear();
    copy.compiled_at.clear();
    let bytes = serde_json::to_vec(&copy).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

pub fn refresh_fingerprint(contract: &mut TaskContract) {
    contract.fingerprint = fingerprint(contract);
}

pub fn contract_is_stable(left: &TaskContract, right: &TaskContract) -> bool {
    left.fingerprint == right.fingerprint
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_with_facts() -> project::Subtask {
        project::Subtask {
            id: "task-1".into(),
            title: "Small change".into(),
            goal: "Update one file".into(),
            allowed_file_paths: vec!["src/lib.rs".into()],
            acceptance_criteria: vec!["file exists".into()],
            confirmation_notes: Some("keep this human decision".into()),
            ..Default::default()
        }
    }

    #[test]
    fn contract_fingerprint_is_comparable() {
        let task = task_with_facts();
        let workload = crate::workload_policy::test_profile(project::WorkloadScale::Micro);
        let first = compile_subtask(&task, None, 0, &workload);
        let second = compile_subtask(&task, None, 0, &workload);
        assert!(contract_is_stable(&first, &second));
        assert_eq!(first.workload_scale, project::WorkloadScale::Micro);
        assert_eq!(first.max_split_depth, 0);
        assert_eq!(first.budget.max_executor_turns, 4);
    }

    #[test]
    fn adaptive_execution_contract_known_v1_snapshot_is_invalidated_without_task_loss() {
        let task = task_with_facts();
        let mut value = serde_json::to_value(&task).unwrap();
        value["contract_snapshot"] = serde_json::json!({
            "version": "task-contract-v1",
            "task_id": "task-1"
        });

        let restored: project::Subtask = serde_json::from_value(value).unwrap();
        assert!(restored.contract_snapshot.is_none());
        assert_eq!(restored.id, task.id);
        assert_eq!(restored.status, task.status);
        assert_eq!(restored.acceptance_criteria, task.acceptance_criteria);
        assert_eq!(restored.confirmation_notes, task.confirmation_notes);
    }

    #[test]
    fn adaptive_execution_contract_v2_snapshot_roundtrips_strictly() {
        let mut task = task_with_facts();
        let workload = crate::workload_policy::test_profile(project::WorkloadScale::Micro);
        task.contract_snapshot = Some(compile_subtask(&task, None, 0, &workload));

        let encoded = serde_json::to_string(&task).unwrap();
        let restored: project::Subtask = serde_json::from_str(&encoded).unwrap();
        let contract = restored.contract_snapshot.unwrap();
        assert_eq!(contract.version, "task-contract-v2");
        assert_eq!(contract.workload_profile_fingerprint, workload.fingerprint);
    }

    #[test]
    fn legacy_budget_without_wall_clock_gets_a_bounded_default() {
        let budget: TaskBudgetSummary = serde_json::from_value(serde_json::json!({
            "level": "small",
            "estimated_model_calls": 1,
            "estimated_input_tokens": 100,
            "estimated_output_tokens": 100,
            "max_executor_turns": 4,
            "max_transport_retries": 0,
            "max_doom_loop_retries": 0
        }))
        .unwrap();
        assert_eq!(budget.max_wall_clock_secs, DEFAULT_MAX_WALL_CLOCK_SECS);
        assert_eq!(
            budget.bounded_wall_clock_secs(),
            DEFAULT_MAX_WALL_CLOCK_SECS
        );
    }

    #[test]
    fn resource_hard_stop_has_priority_over_generic_cancellation() {
        let budget = TaskBudgetSummary {
            max_wall_clock_secs: 600,
            ..TaskBudgetSummary::default()
        };
        let guard =
            ExecutionGuard::new(&budget, crate::runtime_resource::ResourceDecision::HardStop);
        assert_eq!(
            guard.stop_reason(),
            Some(ExecutionStopReason::ResourceHardStop)
        );
        guard.request_cancel();
        assert_eq!(
            guard.stop_reason(),
            Some(ExecutionStopReason::ResourceHardStop)
        );
    }

    #[test]
    fn resource_guard_retains_latest_observation_for_finalization() {
        let budget = TaskBudgetSummary::default();
        let guard =
            ExecutionGuard::new(&budget, crate::runtime_resource::ResourceDecision::Unknown);
        guard.set_resource_observation(project::ResourceObservationSummary {
            state: project::ResourceObservationState::Warning,
            headroom_bytes: Some(10),
            ..Default::default()
        });
        assert_eq!(
            guard.resource_observation().state,
            project::ResourceObservationState::Warning
        );
        assert_eq!(guard.resource_observation().headroom_bytes, Some(10));
    }

    #[test]
    fn resource_hard_stop_cannot_be_downgraded_by_a_later_sample() {
        let budget = TaskBudgetSummary::default();
        let guard =
            ExecutionGuard::new(&budget, crate::runtime_resource::ResourceDecision::Unknown);
        guard.set_resource_observation(project::ResourceObservationSummary {
            state: project::ResourceObservationState::HardStop,
            sampled_at: Some("hard-stop".to_string()),
            ..Default::default()
        });
        guard.set_resource_decision(crate::runtime_resource::ResourceDecision::HardStop);
        guard.set_resource_observation(project::ResourceObservationSummary {
            state: project::ResourceObservationState::Warning,
            sampled_at: Some("later-warning".to_string()),
            ..Default::default()
        });
        guard.set_resource_decision(crate::runtime_resource::ResourceDecision::Warning);
        assert_eq!(
            guard.stop_reason(),
            Some(ExecutionStopReason::ResourceHardStop)
        );
        assert_eq!(
            guard.resource_observation().state,
            project::ResourceObservationState::HardStop
        );
        assert_eq!(
            guard.resource_observation().sampled_at.as_deref(),
            Some("hard-stop")
        );
    }

    #[test]
    fn adaptive_execution_contract_unknown_snapshot_version_is_rejected() {
        let mut value = serde_json::to_value(task_with_facts()).unwrap();
        value["contract_snapshot"] = serde_json::json!({
            "version": "task-contract-v999",
            "task_id": "task-1"
        });

        let error = serde_json::from_value::<project::Subtask>(value).unwrap_err();
        assert!(error.to_string().contains("不支持的任务合同快照版本"));
    }

    #[test]
    fn adaptive_execution_contract_damaged_v2_snapshot_is_rejected() {
        let mut value = serde_json::to_value(task_with_facts()).unwrap();
        value["contract_snapshot"] = serde_json::json!({
            "version": "task-contract-v2",
            "task_id": "task-1"
        });

        assert!(serde_json::from_value::<project::Subtask>(value).is_err());
    }
}
