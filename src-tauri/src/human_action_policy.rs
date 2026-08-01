use crate::project::{AcceptanceStatus, Project, RecoveryPhase, SubtaskStatus};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HumanTerminalAction {
    ConfirmActualPass,
    AcceptDeviation,
    SkipTask,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HumanActionPolicyConfig {
    pub action: HumanTerminalAction,
    pub allowed_statuses: Vec<String>,
    pub requires_successful_execution: bool,
    pub requires_acceptance_selection: bool,
    pub requires_dependency_check: bool,
    pub destructive: bool,
    pub requires_preview: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HumanActionDecision {
    pub action: HumanTerminalAction,
    pub allowed: bool,
    pub denial_reason: String,
    pub destructive: bool,
    pub requires_preview: bool,
    pub actionable_criteria: Vec<u32>,
    pub dependency_check: String,
}

pub fn config(action: HumanTerminalAction) -> HumanActionPolicyConfig {
    match action {
        HumanTerminalAction::ConfirmActualPass => HumanActionPolicyConfig {
            action,
            allowed_statuses: vec![
                "AwaitingConfirmation".to_string(),
                "WaitingHuman".to_string(),
            ],
            requires_successful_execution: true,
            requires_acceptance_selection: false,
            requires_dependency_check: false,
            destructive: false,
            requires_preview: false,
        },
        HumanTerminalAction::AcceptDeviation => HumanActionPolicyConfig {
            action,
            allowed_statuses: vec![
                "AwaitingConfirmation".to_string(),
                "WaitingHuman".to_string(),
            ],
            requires_successful_execution: true,
            requires_acceptance_selection: true,
            requires_dependency_check: false,
            destructive: false,
            requires_preview: false,
        },
        HumanTerminalAction::SkipTask => HumanActionPolicyConfig {
            action,
            allowed_statuses: vec!["WaitingHuman".to_string()],
            requires_successful_execution: false,
            requires_acceptance_selection: false,
            requires_dependency_check: true,
            destructive: true,
            requires_preview: true,
        },
    }
}

pub fn evaluate(
    project: &Project,
    task_id: &str,
    action: HumanTerminalAction,
) -> HumanActionDecision {
    let policy = config(action);
    let denied = |reason: String, actionable_criteria: Vec<u32>, dependency_check: String| {
        HumanActionDecision {
            action,
            allowed: false,
            denial_reason: reason,
            destructive: policy.destructive,
            requires_preview: policy.requires_preview,
            actionable_criteria,
            dependency_check,
        }
    };
    let allowed = |actionable_criteria: Vec<u32>, dependency_check: String| HumanActionDecision {
        action,
        allowed: true,
        denial_reason: String::new(),
        destructive: policy.destructive,
        requires_preview: policy.requires_preview,
        actionable_criteria,
        dependency_check,
    };

    let task = match crate::task_tree::find_task(project, task_id) {
        Ok(Some(task)) => task,
        Ok(None) => {
            return denied(
                format!("任务节点不存在：{}", task_id),
                Vec::new(),
                String::new(),
            )
        }
        Err(error) => return denied(error, Vec::new(), String::new()),
    };
    if !task.child_tasks.is_empty() {
        return denied(
            "人工终态动作只能作用于叶子任务".to_string(),
            Vec::new(),
            String::new(),
        );
    }
    if crate::task_tree::is_terminal(&task.status) {
        return denied(
            "已进入终态的任务不能再次执行人工终态动作".to_string(),
            Vec::new(),
            String::new(),
        );
    }

    let recovery_bound = project
        .workflow_state
        .recovery_state
        .as_ref()
        .zip(project.execution_session.as_ref())
        .is_some_and(|(recovery, session)| {
            recovery.phase == RecoveryPhase::WaitingHuman
                && recovery.subtask_id == task_id
                && session.subtask_id == task_id
        });
    let current_task = crate::task_tree::select_current_leaf(project)
        .ok()
        .flatten()
        .is_some_and(|address| address.task_id == task_id);
    if !current_task && !recovery_bound {
        return denied(
            "只能操作当前叶子或当前人工恢复会话绑定的任务".to_string(),
            Vec::new(),
            String::new(),
        );
    }

    if action == HumanTerminalAction::SkipTask {
        if !recovery_bound {
            return denied(
                "跳过任务只能在当前人工恢复边界执行".to_string(),
                Vec::new(),
                String::new(),
            );
        }
        return match validate_skip_dependencies(project, task_id) {
            Ok(check) => allowed(Vec::new(), check),
            Err(reason) => denied(reason, Vec::new(), String::new()),
        };
    }

    if task
        .execution_result
        .as_ref()
        .is_none_or(|result| !result.success)
    {
        return denied(
            "执行引擎没有成功完成任务，不能使用人工通过或接受偏差".to_string(),
            Vec::new(),
            String::new(),
        );
    }
    if task.status != SubtaskStatus::AwaitingConfirmation && !recovery_bound {
        return denied(
            "任务尚未进入验证或人工恢复边界".to_string(),
            Vec::new(),
            String::new(),
        );
    }

    let actionable_criteria = task
        .acceptance_ledger
        .iter()
        .filter(|item| {
            matches!(
                item.status,
                AcceptanceStatus::Unknown | AcceptanceStatus::Unsatisfied
            )
        })
        .map(|item| item.criterion_index)
        .collect::<Vec<_>>();
    if action == HumanTerminalAction::AcceptDeviation && actionable_criteria.is_empty() {
        return denied(
            "当前没有可接受偏差的未满足或证据不足验收项".to_string(),
            actionable_criteria,
            String::new(),
        );
    }
    allowed(actionable_criteria, String::new())
}

pub fn authorize(
    project: &Project,
    task_id: &str,
    action: HumanTerminalAction,
    requested_criteria: &[u32],
    reason: &str,
) -> Result<HumanActionDecision, String> {
    let decision = evaluate(project, task_id, action);
    if !decision.allowed {
        return Err(decision.denial_reason.clone());
    }
    if reason.trim().is_empty() {
        return Err("人工终态动作必须填写依据".to_string());
    }
    if action == HumanTerminalAction::AcceptDeviation {
        if requested_criteria.is_empty() {
            return Err("接受偏差必须选择至少一个验收项".to_string());
        }
        if requested_criteria
            .iter()
            .any(|index| !decision.actionable_criteria.contains(index))
        {
            return Err("只能接受当前未满足或证据不足的验收项".to_string());
        }
    } else if !requested_criteria.is_empty() {
        return Err("当前人工动作不能携带验收项选择".to_string());
    }
    Ok(decision)
}

pub fn execution_result_fingerprint(task: &crate::project::Subtask) -> Result<String, String> {
    let result = task
        .execution_result
        .as_ref()
        .ok_or_else(|| "任务缺少执行结果，无法生成审计指纹".to_string())?;
    let bytes =
        serde_json::to_vec(result).map_err(|error| format!("执行结果指纹生成失败：{}", error))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

pub fn validate_recorded_human_acceptance(
    project: &Project,
    task: &crate::project::Subtask,
) -> Result<(), String> {
    let verification = task
        .human_verification
        .as_ref()
        .filter(|verification| {
            matches!(
                verification.resolution,
                crate::project::HumanResolution::ConfirmActualPass
                    | crate::project::HumanResolution::AcceptDeviation
            )
        })
        .ok_or_else(|| "任务缺少人工通过或接受偏差记录".to_string())?;
    if verification.verification_kind != crate::project::VerificationKind::HumanOverride
        || verification.verification_reason.trim().is_empty()
    {
        return Err("人工终态记录缺少明确依据".to_string());
    }
    if !matches!(
        verification.action_source.as_str(),
        "task_control" | "recovery"
    ) {
        return Err("人工终态记录缺少可信后端动作来源".to_string());
    }
    if verification.project_revision == 0
        || verification.project_revision > project.workflow_state.data_revision
        || verification.task_tree_revision != project.task_control.tree_revision
    {
        return Err("人工终态记录的项目或任务树修订已失效".to_string());
    }
    let fingerprint = execution_result_fingerprint(task)?;
    if verification.execution_result_fingerprint != fingerprint {
        return Err("人工终态记录绑定的执行结果已变化".to_string());
    }
    if task
        .execution_result
        .as_ref()
        .is_none_or(|result| !result.success)
    {
        return Err("人工终态记录没有成功执行结果".to_string());
    }
    if verification.resolution == crate::project::HumanResolution::AcceptDeviation {
        if verification.accepted_criteria.is_empty()
            || verification.accepted_criteria.iter().any(|index| {
                !task.acceptance_ledger.iter().any(|item| {
                    item.criterion_index == *index
                        && item.status == AcceptanceStatus::AcceptedDeviation
                })
            })
        {
            return Err("接受偏差记录与验收项事实不一致".to_string());
        }
    } else if !verification.accepted_criteria.is_empty() {
        return Err("确认实际通过记录不能携带偏差验收项".to_string());
    }
    Ok(())
}

fn validate_skip_dependencies(project: &Project, task_id: &str) -> Result<String, String> {
    let address = crate::task_tree::locate_task(project, task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", task_id))?;
    let leaves = crate::task_tree::leaf_addresses_in_scope(
        project,
        &address.milestone_id,
        &address.mid_stage_id,
    )?;
    let current_index = leaves
        .iter()
        .position(|candidate| candidate.task_id == task_id)
        .ok_or_else(|| "无法定位要跳过的叶子任务".to_string())?;
    let remaining = leaves
        .iter()
        .skip(current_index + 1)
        .filter_map(|candidate| {
            crate::task_tree::find_task(project, &candidate.task_id)
                .ok()
                .flatten()
                .filter(|task| task.status == SubtaskStatus::Pending)
        })
        .collect::<Vec<_>>();
    let hard_dependents = remaining
        .iter()
        .filter(|task| {
            task.depends_on
                .iter()
                .any(|dependency| dependency == task_id)
        })
        .map(|task| task.title.clone())
        .collect::<Vec<_>>();
    if !hard_dependents.is_empty() {
        return Err(format!(
            "后续任务存在硬依赖，不能跳过：{}",
            hard_dependents.join("、")
        ));
    }
    if remaining
        .iter()
        .any(|task| task.depends_on.is_empty() && task.dependency_notes.trim().is_empty())
    {
        return Err("旧计划没有显式依赖契约，无法证明跳过安全；请先重新校准后续任务".to_string());
    }
    Ok(if remaining.is_empty() {
        "没有后续任务".to_string()
    } else {
        "后续任务显式声明不依赖当前任务".to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{
        AcceptanceLedgerItem, ExecutionResult, Milestone, MilestoneStatus, StageMode, Subtask,
    };

    fn project_with_tasks(tasks: Vec<Subtask>) -> Project {
        let mut project = Project::new("human-policy");
        project.current_milestone_id = "m".to_string();
        project.milestones.push(Milestone {
            id: "m".to_string(),
            version: "v0.1".to_string(),
            title: "M".to_string(),
            description: String::new(),
            tech_stack: String::new(),
            status: MilestoneStatus::InProgress,
            mode: StageMode::Quick,
            mid_stages: Vec::new(),
            subtasks: tasks,
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
        project
    }

    fn task(id: &str, status: SubtaskStatus, success: Option<bool>) -> Subtask {
        Subtask {
            id: id.to_string(),
            title: id.to_string(),
            status,
            execution_result: success.map(|success| ExecutionResult {
                success,
                ..Default::default()
            }),
            acceptance_criteria: vec!["criterion".to_string()],
            acceptance_ledger: vec![AcceptanceLedgerItem {
                criterion_index: 1,
                criterion: "criterion".to_string(),
                status: AcceptanceStatus::Unknown,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn phase1_human_action_safety_unexecuted_and_failed_tasks_cannot_accept_deviation() {
        for success in [None, Some(false)] {
            let project = project_with_tasks(vec![task(
                "leaf",
                SubtaskStatus::AwaitingConfirmation,
                success,
            )]);
            let decision = evaluate(&project, "leaf", HumanTerminalAction::AcceptDeviation);
            assert!(!decision.allowed);
            assert!(decision.denial_reason.contains("没有成功完成"));
        }
    }

    #[test]
    fn phase1_human_action_safety_successful_leaf_accepts_only_unresolved_criteria() {
        let project = project_with_tasks(vec![task(
            "leaf",
            SubtaskStatus::AwaitingConfirmation,
            Some(true),
        )]);
        let decision = authorize(
            &project,
            "leaf",
            HumanTerminalAction::AcceptDeviation,
            &[1],
            "known deviation",
        )
        .expect("successful current leaf should be eligible");
        assert_eq!(decision.actionable_criteria, vec![1]);
    }

    #[test]
    fn phase1_human_action_safety_parent_and_future_leaf_are_read_only() {
        let parent = Subtask {
            id: "parent".to_string(),
            title: "parent".to_string(),
            status: SubtaskStatus::AwaitingConfirmation,
            execution_result: Some(ExecutionResult {
                success: true,
                ..Default::default()
            }),
            child_tasks: vec![task("current", SubtaskStatus::Pending, None)],
            ..Default::default()
        };
        let future = task("future", SubtaskStatus::AwaitingConfirmation, Some(true));
        let project = project_with_tasks(vec![parent, future]);
        assert!(!evaluate(&project, "parent", HumanTerminalAction::AcceptDeviation).allowed);
        assert!(!evaluate(&project, "future", HumanTerminalAction::AcceptDeviation).allowed);
    }

    fn bind_waiting_recovery(project: &mut Project, task_id: &str) {
        project.workflow_state.recovery_state = Some(crate::project::RecoveryState {
            phase: RecoveryPhase::WaitingHuman,
            subtask_id: task_id.to_string(),
            ..Default::default()
        });
        project.execution_session = Some(crate::project::ExecutionSession {
            active: true,
            milestone_id: "m".to_string(),
            subtask_id: task_id.to_string(),
            ..Default::default()
        });
    }

    #[test]
    fn phase1_human_action_safety_skip_requires_explicit_dependency_contract() {
        let skipped = task("current", SubtaskStatus::Rejected, Some(false));
        let legacy = task("next", SubtaskStatus::Pending, None);
        let mut project = project_with_tasks(vec![skipped.clone(), legacy]);
        bind_waiting_recovery(&mut project, "current");
        assert!(!evaluate(&project, "current", HumanTerminalAction::SkipTask).allowed);

        let dependent = Subtask {
            depends_on: vec!["current".to_string()],
            ..task("next", SubtaskStatus::Pending, None)
        };
        let mut project = project_with_tasks(vec![skipped.clone(), dependent]);
        bind_waiting_recovery(&mut project, "current");
        assert!(!evaluate(&project, "current", HumanTerminalAction::SkipTask).allowed);

        let independent = Subtask {
            dependency_notes: "明确不依赖当前任务".to_string(),
            ..task("next", SubtaskStatus::Pending, None)
        };
        let mut project = project_with_tasks(vec![skipped, independent]);
        bind_waiting_recovery(&mut project, "current");
        let decision = evaluate(&project, "current", HumanTerminalAction::SkipTask);
        assert!(decision.allowed);
        assert!(decision.destructive);
        assert!(decision.requires_preview);
    }
}
