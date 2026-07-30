use crate::control_snapshot::{build, TaskControlSnapshot};
use crate::project;
use crate::task_control::{TaskControlMode, TASK_CONTROL_ALGORITHM_VERSION};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct TaskControlActionRequest {
    pub action: String,
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub criterion_indexes: Vec<u32>,
    #[serde(default)]
    pub expected_tree_revision: Option<u64>,
    #[serde(default)]
    pub decision_id: String,
    #[serde(default)]
    pub action_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TaskControlActionResult {
    pub snapshot: TaskControlSnapshot,
    pub job_started: bool,
    pub queued: bool,
    pub action_id: String,
    pub project_revision: u64,
    pub snapshot_version: String,
}

fn parse_mode(mode: &str) -> Result<TaskControlMode, String> {
    match mode {
        "Legacy" | "legacy" => Ok(TaskControlMode::Legacy),
        "Shadow" | "shadow" => Ok(TaskControlMode::Shadow),
        "SerialTakeover" | "serial_takeover" => Ok(TaskControlMode::SerialTakeover),
        _ => Err(format!("未知控制模式：{}", mode)),
    }
}

#[tauri::command]
pub(crate) async fn get_task_control_snapshot(
    project_name: String,
) -> Result<TaskControlSnapshot, String> {
    build(&crate::load_project(&project_name)?)
}

#[tauri::command]
pub(crate) async fn set_task_control_mode(
    project_name: String,
    mode: String,
    expected_revision: u64,
) -> Result<project::Project, String> {
    let mut project = crate::load_project(&project_name)?;
    ensure_revision(&project, expected_revision)?;
    let mode = parse_mode(&mode)?;
    if mode == TaskControlMode::SerialTakeover
        && project
            .execution_session
            .as_ref()
            .is_some_and(|session| session.active)
    {
        return Err("当前任务正在执行，不能切换串行接管模式".to_string());
    }
    project.task_control.mode = mode;
    project.task_control.algorithm_version = TASK_CONTROL_ALGORITHM_VERSION.to_string();
    project.task_control.control_source = match mode {
        TaskControlMode::Legacy => "legacy_workflow",
        TaskControlMode::Shadow => "shadow_controller",
        TaskControlMode::SerialTakeover => "task_controller",
    }
    .to_string();
    project.workflow_state.data_revision += 1;
    project.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
    crate::save_and_reload_project(&project)
}

#[tauri::command]
pub(crate) async fn apply_task_control_action(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
    mut request: TaskControlActionRequest,
) -> Result<TaskControlActionResult, String> {
    let mut project = crate::load_project(&project_name)?;
    let action = request.action.trim().to_ascii_lowercase();
    let task_id = requested_task_id(&project, &request)?;
    let base_action_id = manual_action_id(&project, &request, &task_id)?;
    if request.action_id.is_empty() {
        request.action_id = base_action_id.clone();
    }

    match action.as_str() {
        "pause" => {
            if project
                .workflow_state
                .autopilot_state
                .as_ref()
                .is_some_and(|state| state.run_status == project::AutopilotRunStatus::Paused)
            {
                return action_result(&project, false, false, base_action_id);
            }
            ensure_request_revisions(&project, &request)?;
            if let Some(state) = project.workflow_state.autopilot_state.as_mut() {
                state.run_status = project::AutopilotRunStatus::Paused;
                state.last_action = "任务控制中心暂停后续派发".to_string();
                state.last_action_at = chrono::Utc::now().to_rfc3339();
            } else {
                return Err("自动驾驶状态不存在，无法暂停".to_string());
            }
            append_event(
                &mut project,
                "info",
                project::ExecutionEventType::SystemAdvance,
                request.clone(),
            );
            persist_manual_lifecycle_action(&mut project, &base_action_id, "pause", true);
        }
        "resume" => {
            if project
                .workflow_state
                .autopilot_state
                .as_ref()
                .is_some_and(|state| state.run_status == project::AutopilotRunStatus::Running)
            {
                let started = state
                    .autopilot_runtime
                    .start_if_active(state.pipeline_state.clone(), project_name.clone())
                    .await?;
                return action_result(&project, started, false, base_action_id);
            }
            ensure_request_revisions(&project, &request)?;
            let mut resumed =
                crate::commands::workflow::autopilot_resume_state(project_name.clone()).await?;
            append_event(
                &mut resumed,
                "info",
                project::ExecutionEventType::SystemAdvance,
                request.clone(),
            );
            persist_manual_lifecycle_action(&mut resumed, &base_action_id, "resume", true);
            crate::save_project(&resumed)?;
            state
                .autopilot_runtime
                .start(state.pipeline_state.clone(), project_name)
                .await?;
            return action_result(&resumed, true, false, base_action_id);
        }
        "stop" => {
            if !project.workflow_state.autopilot_active
                && project
                    .workflow_state
                    .autopilot_state
                    .as_ref()
                    .is_some_and(|state| !state.active)
            {
                return action_result(&project, false, false, base_action_id);
            }
            ensure_request_revisions(&project, &request)?;
            project.workflow_state.autopilot_active = false;
            if let Some(state) = project.workflow_state.autopilot_state.as_mut() {
                state.active = false;
                state.run_status = project::AutopilotRunStatus::ErrorStopped;
                state.recovery_action = project::AutopilotRecoveryAction::WaitHumanDecision;
                state.last_action = "任务控制中心停止自动派发".to_string();
                state.last_action_at = chrono::Utc::now().to_rfc3339();
            }
            append_event(
                &mut project,
                "info",
                project::ExecutionEventType::SystemAdvance,
                request.clone(),
            );
            persist_manual_lifecycle_action(&mut project, &base_action_id, "stop", true);
        }
        "revalidate" | "validate" | "split" | "recompile" | "accept_deviation" => {
            let action_plan = manual_executor_actions(&project, &request, &task_id)?;
            let final_action_id = format!(
                "{}-{}",
                base_action_id,
                action_plan
                    .last()
                    .map(|(kind, _)| kind.as_str())
                    .unwrap_or("none")
            );
            if project.task_control.last_completed_action_id == final_action_id {
                return action_result(&project, false, false, final_action_id);
            }
            if project
                .task_control
                .active_action_id
                .starts_with(&base_action_id)
            {
                return action_result(
                    &project,
                    false,
                    true,
                    project.task_control.active_action_id.clone(),
                );
            }
            ensure_request_revisions(&project, &request)?;
            let mut last_result = None;
            for (kind, criterion_indexes) in action_plan {
                let latest = crate::load_project(&project_name)?;
                let address = crate::task_tree::locate_task(&latest, &task_id)?
                    .ok_or_else(|| format!("任务节点不存在：{}", task_id))?;
                let task = crate::task_tree::find_task(&latest, &task_id)?
                    .ok_or_else(|| format!("任务节点不存在：{}", task_id))?;
                let contract = crate::task_compiler::compile(
                    task,
                    address.ancestor_task_ids.last().map(String::as_str),
                    address.depth,
                )
                .contract;
                let action_id = format!("{}-{}", base_action_id, kind.as_str());
                last_result = Some(
                    crate::control_action_executor::execute(
                        state.pipeline_state.clone(),
                        project_name.clone(),
                        crate::control_action_executor::ControlActionRequest {
                            action_id,
                            action: kind,
                            task_id: task_id.clone(),
                            decision_id: request.decision_id.clone(),
                            expected_project_revision: Some(latest.workflow_state.data_revision),
                            expected_tree_revision: Some(latest.task_control.tree_revision),
                            contract_fingerprint: contract.fingerprint,
                            criterion_indexes,
                            reason: request.reason.clone(),
                            source: project::OperationSource::User,
                        },
                    )
                    .await?,
                );
            }
            let latest = crate::load_project(&project_name)?;
            let result = last_result.ok_or_else(|| "没有可执行的人工控制动作".to_string())?;
            return action_result(&latest, false, result.queued, result.action_id);
        }
        _ => return Err(format!("未知任务控制动作：{}", request.action)),
    }
    project.workflow_state.data_revision += 1;
    project.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
    crate::save_project(&project)?;
    action_result(&project, false, false, base_action_id)
}

fn requested_task_id(
    project: &project::Project,
    request: &TaskControlActionRequest,
) -> Result<String, String> {
    if let Some(task_id) = request.task_id.as_deref().filter(|value| !value.is_empty()) {
        return Ok(task_id.to_string());
    }
    if matches!(
        request.action.trim().to_ascii_lowercase().as_str(),
        "pause" | "resume" | "stop"
    ) {
        return Ok(String::new());
    }
    let current = build(project)?.current_task_id;
    if current.is_empty() {
        Err("当前没有可控制任务".to_string())
    } else {
        Ok(current)
    }
}

fn manual_action_id(
    project: &project::Project,
    request: &TaskControlActionRequest,
    task_id: &str,
) -> Result<String, String> {
    if !request.action_id.trim().is_empty() {
        return Ok(request.action_id.clone());
    }
    let bytes = serde_json::to_vec(&(
        request.action.trim().to_ascii_lowercase(),
        task_id,
        request
            .expected_revision
            .unwrap_or(project.workflow_state.data_revision),
        request
            .expected_tree_revision
            .unwrap_or(project.task_control.tree_revision),
        &request.criterion_indexes,
        request.reason.trim(),
    ))
    .map_err(|error| format!("人工控制动作指纹生成失败：{}", error))?;
    Ok(format!("manual-{:x}", Sha256::digest(bytes)))
}

fn manual_executor_actions(
    project: &project::Project,
    request: &TaskControlActionRequest,
    task_id: &str,
) -> Result<Vec<(crate::control_action::ControlActionKind, Vec<u32>)>, String> {
    let action = request.action.trim().to_ascii_lowercase();
    if action == "split" {
        return Ok(vec![(
            crate::control_action::ControlActionKind::Split,
            Vec::new(),
        )]);
    }
    if action == "recompile" {
        return Ok(vec![(
            crate::control_action::ControlActionKind::Recompile,
            Vec::new(),
        )]);
    }
    if action == "accept_deviation" {
        return Ok(vec![(
            crate::control_action::ControlActionKind::AcceptDeviation,
            request.criterion_indexes.clone(),
        )]);
    }
    let task = crate::task_tree::find_task(project, task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", task_id))?;
    let targets = crate::acceptance::revalidation_target_indexes(task, &request.criterion_indexes)?;
    if targets.is_empty() {
        return Err("当前任务没有需要重新验证的验收项".to_string());
    }
    let mut deterministic = Vec::new();
    let mut automated = Vec::new();
    let mut semantic = Vec::new();
    for index in targets {
        match crate::validator_registry::verification_mode_for(task, index) {
            crate::validator_contract::VerificationMode::Deterministic => deterministic.push(index),
            crate::validator_contract::VerificationMode::AutomatedTest => automated.push(index),
            crate::validator_contract::VerificationMode::SemanticReview
            | crate::validator_contract::VerificationMode::HumanReview => semantic.push(index),
        }
    }
    let mut actions = Vec::new();
    if !deterministic.is_empty() {
        actions.push((
            crate::control_action::ControlActionKind::LocalValidate,
            deterministic,
        ));
    }
    if !automated.is_empty() {
        actions.push((
            crate::control_action::ControlActionKind::AutomatedValidate,
            automated,
        ));
    }
    if !semantic.is_empty() {
        actions.push((
            crate::control_action::ControlActionKind::TargetedValidate,
            semantic,
        ));
    }
    Ok(actions)
}

fn ensure_request_revisions(
    project: &project::Project,
    request: &TaskControlActionRequest,
) -> Result<(), String> {
    if let Some(expected) = request.expected_revision {
        ensure_revision(project, expected)?;
    }
    if let Some(expected) = request.expected_tree_revision {
        if project.task_control.tree_revision != expected {
            return Err(format!(
                "任务树修订冲突：请求={}，磁盘={}",
                expected, project.task_control.tree_revision
            ));
        }
    }
    Ok(())
}

fn persist_manual_lifecycle_action(
    project: &mut project::Project,
    action_id: &str,
    action_kind: &str,
    made_progress: bool,
) {
    project.task_control.last_completed_action_id = action_id.to_string();
    project.task_control.last_completed_action_kind = action_kind.to_string();
    project.task_control.last_completed_action_task_id.clear();
    project.task_control.last_action_result = format!("人工控制动作 {} 已完成", action_kind);
    project.task_control.last_action_made_progress = made_progress;
    project.task_control.last_action_at = Some(chrono::Utc::now().to_rfc3339());
}

fn action_result(
    project: &project::Project,
    job_started: bool,
    queued: bool,
    action_id: String,
) -> Result<TaskControlActionResult, String> {
    Ok(TaskControlActionResult {
        snapshot: build(project)?,
        job_started,
        queued,
        action_id,
        project_revision: project.workflow_state.data_revision,
        snapshot_version: project.task_control.snapshot_version.clone(),
    })
}

pub(crate) fn split_task(project: &mut project::Project, task_id: &str) -> Result<(), String> {
    let changed = {
        let task = crate::task_tree::find_task_mut(project, task_id)?
            .ok_or_else(|| format!("任务节点不存在：{}", task_id))?;
        if matches!(
            task.status,
            project::SubtaskStatus::Passed
                | project::SubtaskStatus::AcceptedDeviation
                | project::SubtaskStatus::Skipped
        ) {
            return Err("已完成任务不能重新拆分".to_string());
        }
        if !task.child_tasks.is_empty() {
            false
        } else {
            let depth = task
                .contract_snapshot
                .as_ref()
                .map(|contract| contract.depth)
                .unwrap_or(0);
            let compiled = crate::task_compiler::compile(task, None, depth);
            if compiled.decision.kind != crate::task_compiler::TaskCompileDecisionKind::SplitFurther
            {
                return Err(compiled.decision.reason);
            }
            let plan = compiled
                .split_plan
                .as_ref()
                .ok_or_else(|| "任务编译器未返回安全拆分计划".to_string())?;
            let children = crate::task_compiler::materialize_child_tasks(task, depth, plan)?;
            task.contract_snapshot = Some(compiled.contract);
            task.child_tasks = children;
            true
        }
    };
    if changed {
        project.task_control.tree_revision = project.task_control.tree_revision.saturating_add(1);
    }
    Ok(())
}

pub(crate) fn recompile_task(project: &mut project::Project, task_id: &str) -> Result<(), String> {
    let address = crate::task_tree::locate_task(project, task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", task_id))?;
    let task = crate::task_tree::find_task_mut(project, task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", task_id))?;
    if matches!(
        task.status,
        project::SubtaskStatus::Passed
            | project::SubtaskStatus::AcceptedDeviation
            | project::SubtaskStatus::Skipped
    ) {
        return Err("已完成任务不能重新编译覆盖".to_string());
    }
    task.contract_snapshot = Some(crate::task_contract::compile_subtask(
        task,
        address.ancestor_task_ids.last().map(String::as_str),
        address.depth,
    ));
    Ok(())
}

pub(crate) fn accept_deviation(
    project: &mut project::Project,
    task_id: &str,
    criterion_indexes: &[u32],
    reason: &str,
) -> Result<(), String> {
    let task = crate::task_tree::find_task_mut(project, task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", task_id))?;
    if matches!(
        task.status,
        project::SubtaskStatus::Passed
            | project::SubtaskStatus::AcceptedDeviation
            | project::SubtaskStatus::Skipped
    ) {
        return Err("已完成任务不能重复接受偏差".to_string());
    }
    let mut updated = 0usize;
    let now = chrono::Utc::now().to_rfc3339();
    for item in &mut task.acceptance_ledger {
        if criterion_indexes.contains(&item.criterion_index) {
            item.status = project::AcceptanceStatus::AcceptedDeviation;
            item.evidence = format!("人工接受偏差：{}", reason);
            item.updated_at = now.clone();
            updated += 1;
        }
    }
    if updated != criterion_indexes.len() {
        return Err("接受偏差包含不存在的验收项".to_string());
    }
    task.human_verification = Some(project::HumanVerification {
        verification_kind: project::VerificationKind::HumanOverride,
        verification_reason: reason.to_string(),
        verified_at: now,
        original_test_failure: task.test_report.clone(),
        resolution: project::HumanResolution::AcceptDeviation,
        accepted_criteria: criterion_indexes.to_vec(),
        dependency_check: "偏差仅作用于当前任务合同".to_string(),
    });
    if task.acceptance_ledger.iter().all(|item| {
        matches!(
            item.status,
            project::AcceptanceStatus::Satisfied | project::AcceptanceStatus::AcceptedDeviation
        )
    }) {
        task.status = project::SubtaskStatus::AcceptedDeviation;
    }
    Ok(())
}

fn ensure_revision(project: &project::Project, expected: u64) -> Result<(), String> {
    if project.workflow_state.data_revision != expected {
        return Err(format!(
            "项目修订冲突：请求={}，磁盘={}",
            expected, project.workflow_state.data_revision
        ));
    }
    Ok(())
}

fn append_event(
    project: &mut project::Project,
    level: &str,
    event_type: project::ExecutionEventType,
    request: TaskControlActionRequest,
) {
    project
        .execution_history
        .push(project::ExecutionHistoryEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            level: level.to_string(),
            event_type,
            source: project::OperationSource::User,
            text: if request.reason.trim().is_empty() {
                format!("任务控制动作：{}", request.action)
            } else {
                request.reason
            },
            milestone_id: None,
            mid_stage_id: None,
            subtask_id: request.task_id,
            criterion_index: (request.criterion_indexes.len() == 1)
                .then(|| request.criterion_indexes[0]),
            decision_id: (!request.decision_id.is_empty()).then_some(request.decision_id),
            action_id: (!request.action_id.is_empty()).then_some(request.action_id),
            validator_id: None,
            model_call_id: None,
        });
    if project.execution_history.len() > project::MAX_EXECUTION_HISTORY {
        let excess = project.execution_history.len() - project::MAX_EXECUTION_HISTORY;
        project.execution_history.drain(0..excess);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_task() -> project::Project {
        let mut project = project::Project::new("task-control-test");
        project.milestones.push(project::Milestone {
            id: "m".to_string(),
            version: "v0.1".to_string(),
            title: "M".to_string(),
            description: String::new(),
            tech_stack: String::new(),
            status: project::MilestoneStatus::InProgress,
            mode: project::StageMode::Quick,
            mid_stages: Vec::new(),
            subtasks: vec![project::Subtask {
                id: "task".to_string(),
                title: "Task".to_string(),
                acceptance_criteria: vec![
                    "file `a` exists".to_string(),
                    "file `b` exists".to_string(),
                    "file `c` exists".to_string(),
                    "file `d` exists".to_string(),
                ],
                acceptance_ledger: vec![
                    project::AcceptanceLedgerItem {
                        criterion_index: 1,
                        criterion: "file `a` exists".to_string(),
                        ..Default::default()
                    },
                    project::AcceptanceLedgerItem {
                        criterion_index: 2,
                        criterion: "file `b` exists".to_string(),
                        ..Default::default()
                    },
                    project::AcceptanceLedgerItem {
                        criterion_index: 3,
                        criterion: "file `c` exists".to_string(),
                        ..Default::default()
                    },
                    project::AcceptanceLedgerItem {
                        criterion_index: 4,
                        criterion: "file `d` exists".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            qa_result: None,
            git_commit_hash: String::new(),
            decomposition_check: None,
            review_status: None,
            review_conclusion: None,
            approved_at: None,
            goal: String::new(),
            scope: String::new(),
            dependencies: Vec::new(),
            expected_output: String::new(),
            acceptance_criteria: Vec::new(),
        });
        project.current_milestone_id = "m".to_string();
        project
    }

    #[test]
    fn split_is_scoped_and_idempotent() {
        let mut project = project_with_task();
        split_task(&mut project, "task").unwrap();
        let first_count = project.milestones[0].subtasks[0].child_tasks.len();
        assert_eq!(first_count, 4);
        split_task(&mut project, "task").unwrap();
        assert_eq!(
            project.milestones[0].subtasks[0].child_tasks.len(),
            first_count
        );
        assert!(project.milestones[0].subtasks[0]
            .child_tasks
            .iter()
            .all(|child| child.depends_on.is_empty()));
    }

    #[test]
    fn deviation_updates_only_requested_criterion() {
        let mut project = project_with_task();
        accept_deviation(&mut project, "task", &[2], "外部依赖已接受").unwrap();
        let task = &project.milestones[0].subtasks[0];
        assert_eq!(
            task.acceptance_ledger[0].status,
            project::AcceptanceStatus::Unknown
        );
        assert_eq!(
            task.acceptance_ledger[1].status,
            project::AcceptanceStatus::AcceptedDeviation
        );
        assert_eq!(
            task.human_verification.as_ref().unwrap().accepted_criteria,
            vec![2]
        );
    }

    #[test]
    fn revalidation_routes_deterministic_and_semantic_criteria() {
        let mut project = project_with_task();
        project.milestones[0].subtasks[0].acceptance_criteria = vec![
            "file exists: `index.html`".to_string(),
            "自动化测试通过".to_string(),
            "用户可以完整完成结账流程".to_string(),
        ];
        project.milestones[0].subtasks[0].acceptance_ledger = vec![
            project::AcceptanceLedgerItem {
                criterion_index: 1,
                criterion: "file exists: `index.html`".to_string(),
                ..Default::default()
            },
            project::AcceptanceLedgerItem {
                criterion_index: 2,
                criterion: "自动化测试通过".to_string(),
                status: project::AcceptanceStatus::Unknown,
                ..Default::default()
            },
            project::AcceptanceLedgerItem {
                criterion_index: 3,
                criterion: "用户可以完整完成结账流程".to_string(),
                status: project::AcceptanceStatus::Unsatisfied,
                ..Default::default()
            },
        ];
        let request = TaskControlActionRequest {
            action: "revalidate".to_string(),
            task_id: Some("task".to_string()),
            ..TaskControlActionRequest::default()
        };

        let actions = manual_executor_actions(&project, &request, "task").unwrap();
        assert_eq!(actions.len(), 3);
        assert_eq!(
            actions[0],
            (
                crate::control_action::ControlActionKind::LocalValidate,
                vec![1]
            )
        );
        assert_eq!(
            actions[1],
            (
                crate::control_action::ControlActionKind::AutomatedValidate,
                vec![2]
            )
        );
        assert_eq!(
            actions[2],
            (
                crate::control_action::ControlActionKind::TargetedValidate,
                vec![3]
            )
        );
    }

    #[test]
    fn generated_manual_action_id_is_stable_for_the_same_request() {
        let project = project_with_task();
        let request = TaskControlActionRequest {
            action: "revalidate".to_string(),
            task_id: Some("task".to_string()),
            expected_revision: Some(4),
            expected_tree_revision: Some(2),
            criterion_indexes: vec![1, 2],
            ..TaskControlActionRequest::default()
        };

        let first = manual_action_id(&project, &request, "task").unwrap();
        let second = manual_action_id(&project, &request, "task").unwrap();
        assert_eq!(first, second);
    }
}
