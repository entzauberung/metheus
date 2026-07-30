use crate::project;
use crate::validator_contract::VerificationMode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const TASK_CONTRACT_VERSION: &str = "task-contract-v1";

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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskContract {
    pub version: String,
    pub task_id: String,
    pub parent_task_id: Option<String>,
    pub depth: u32,
    pub node_type: TaskNodeType,
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

pub fn compile_subtask(
    subtask: &project::Subtask,
    parent_task_id: Option<&str>,
    depth: u32,
) -> TaskContract {
    let verification_modes = subtask
        .acceptance_criteria
        .iter()
        .map(|criterion| crate::validator_registry::validators_for(criterion)[0].mode)
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
    let budget = crate::task_complexity::estimate_budget(subtask, complexity);
    let mut contract = TaskContract {
        version: TASK_CONTRACT_VERSION.to_string(),
        task_id: subtask.id.clone(),
        parent_task_id: parent_task_id.map(str::to_string),
        depth,
        node_type: TaskNodeType::Subtask,
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

    #[test]
    fn contract_fingerprint_is_comparable() {
        let mut task = project::Subtask::default();
        task.id = "task-1".into();
        task.title = "Small change".into();
        task.goal = "Update one file".into();
        task.allowed_file_paths = vec!["src/lib.rs".into()];
        task.acceptance_criteria = vec!["file exists".into()];
        let first = compile_subtask(&task, None, 0);
        let second = compile_subtask(&task, None, 0);
        assert!(contract_is_stable(&first, &second));
    }
}
