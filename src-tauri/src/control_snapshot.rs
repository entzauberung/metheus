use crate::control_scheduler::TaskControlDecision;
use crate::cost_ledger::{CostGroupSummary, ModelCallRecord, TokenCostSummary};
use crate::project::{AcceptanceLedgerItem, MidStage, Milestone, Project, Subtask};
use crate::task_compiler::compile;
use crate::task_contract::TaskContract;
use crate::task_control::{mode_label, ShadowComparisonMetrics, TaskControlMode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_SNAPSHOT_EVENTS: usize = 120;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskTreeNodeView {
    pub id: String,
    pub title: String,
    pub node_type: String,
    pub status: String,
    pub depth: u32,
    pub complexity: String,
    pub risk: String,
    pub contract_fingerprint: String,
    pub contract: Option<TaskContract>,
    pub dependencies: Vec<String>,
    pub acceptance: Vec<AcceptanceLedgerItem>,
    pub capabilities: Vec<String>,
    pub disabled_reasons: BTreeMap<String, String>,
    pub is_currently_actionable: bool,
    pub actionable_acceptance_criteria: Vec<u32>,
    pub children: Vec<TaskTreeNodeView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ControlEventView {
    pub timestamp: String,
    pub level: String,
    pub source: String,
    pub text: String,
    pub task_id: Option<String>,
    pub criterion_index: Option<u32>,
    pub decision_id: Option<String>,
    pub action_id: Option<String>,
    pub validator_id: Option<String>,
    pub model_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlActionStateView {
    pub action_id: String,
    pub kind: String,
    pub task_id: String,
    pub result: String,
    pub made_progress: bool,
    pub at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskControlSnapshot {
    pub snapshot_version: String,
    pub project_name: String,
    pub project_revision: u64,
    pub control_algorithm_version: String,
    pub control_mode: TaskControlMode,
    pub control_mode_label: String,
    pub current_milestone_id: String,
    pub current_mid_stage_id: String,
    pub current_task_id: String,
    pub task_tree_revision: u64,
    pub source_process_start_id: String,
    pub source_event_sequence: u64,
    pub source_control_action_id: Option<String>,
    pub nodes: Vec<TaskTreeNodeView>,
    pub selected_contract: Option<TaskContract>,
    pub selected_acceptance: Vec<AcceptanceLedgerItem>,
    pub decision: Option<TaskControlDecision>,
    pub shadow_comparison: ShadowComparisonMetrics,
    pub current_action: Option<ControlActionStateView>,
    pub recent_action: Option<ControlActionStateView>,
    pub control_capabilities: Vec<String>,
    pub cost: TokenCostSummary,
    pub stage_cost: TokenCostSummary,
    pub task_cost: TokenCostSummary,
    pub provider_costs: Vec<CostGroupSummary>,
    pub purpose_costs: Vec<CostGroupSummary>,
    pub cost_calls: Vec<ModelCallRecord>,
    pub events: Vec<ControlEventView>,
    pub heartbeat_at: String,
}

pub fn build(project: &Project) -> Result<TaskControlSnapshot, String> {
    build_at_event(project, "", 0)
}

pub fn build_at_event(
    project: &Project,
    process_start_id: &str,
    event_sequence: u64,
) -> Result<TaskControlSnapshot, String> {
    crate::task_tree::validate_project_tree(project)?;
    let selected_address = crate::task_tree::select_current_leaf(project)?;
    let selected_task_id = selected_address
        .as_ref()
        .map(|address| address.task_id.clone())
        .unwrap_or_default();
    let mut nodes = Vec::new();
    for milestone in &project.milestones {
        nodes.push(milestone_node(project, milestone, &selected_task_id));
    }
    let selected = crate::task_tree::find_task(project, &selected_task_id)?;
    let (selected_contract, acceptance, selected_can_split) = if let Some(task) = selected {
        let address = selected_address
            .as_ref()
            .ok_or_else(|| "控制快照缺少当前任务地址".to_string())?;
        let result = compile(
            task,
            address.ancestor_task_ids.last().map(String::as_str),
            address.depth,
        );
        (
            Some(result.contract.clone()),
            task.acceptance_ledger.clone(),
            result.decision.kind == crate::task_compiler::TaskCompileDecisionKind::SplitFurther,
        )
    } else {
        (None, Vec::new(), false)
    };
    let decision = project
        .task_control
        .last_decision
        .as_ref()
        .filter(|decision| decision.task_id == selected_task_id)
        .cloned();
    let events = project
        .execution_history
        .iter()
        .rev()
        .take(MAX_SNAPSHOT_EVENTS)
        .map(|event| ControlEventView {
            timestamp: event.timestamp.clone(),
            level: event.level.clone(),
            source: format!("{:?}", event.source),
            text: event.text.clone(),
            task_id: event.subtask_id.clone(),
            criterion_index: event.criterion_index,
            decision_id: event.decision_id.clone(),
            action_id: event.action_id.clone(),
            validator_id: event.validator_id.clone(),
            model_call_id: event.model_call_id.clone(),
        })
        .collect::<Vec<_>>();
    Ok(TaskControlSnapshot {
        snapshot_version: project.task_control.snapshot_version.clone(),
        project_name: project.name.clone(),
        project_revision: project.workflow_state.data_revision,
        control_algorithm_version: project.task_control.algorithm_version.clone(),
        control_mode: project.task_control.mode,
        control_mode_label: mode_label(project.task_control.mode).to_string(),
        current_milestone_id: project.current_milestone_id.clone(),
        current_mid_stage_id: project.current_mid_stage_id.clone(),
        current_task_id: selected_task_id.clone(),
        task_tree_revision: project.task_control.tree_revision,
        source_process_start_id: process_start_id.to_string(),
        source_event_sequence: event_sequence,
        source_control_action_id: if !project.task_control.active_action_id.is_empty() {
            Some(project.task_control.active_action_id.clone())
        } else if !project.task_control.last_completed_action_id.is_empty() {
            Some(project.task_control.last_completed_action_id.clone())
        } else {
            None
        },
        nodes,
        selected_contract,
        selected_acceptance: acceptance,
        decision,
        shadow_comparison: project.task_control.shadow_comparison.clone(),
        current_action: current_action(project),
        recent_action: recent_action(project),
        control_capabilities: control_capabilities(project, selected, selected_can_split),
        cost: project.cost_ledger.project_summary.clone(),
        stage_cost: project
            .cost_ledger
            .summary_for_stage(&project.current_mid_stage_id),
        task_cost: project.cost_ledger.summary_for_task(&selected_task_id),
        provider_costs: project.cost_ledger.summaries_by_provider(),
        purpose_costs: project.cost_ledger.summaries_by_purpose(),
        cost_calls: project
            .cost_ledger
            .calls
            .iter()
            .rev()
            .take(100)
            .cloned()
            .collect(),
        events,
        heartbeat_at: project
            .workflow_state
            .autopilot_state
            .as_ref()
            .map(|state| state.heartbeat_at.clone())
            .unwrap_or_default(),
    })
}

fn current_action(project: &Project) -> Option<ControlActionStateView> {
    (!project.task_control.active_action_id.is_empty()).then(|| ControlActionStateView {
        action_id: project.task_control.active_action_id.clone(),
        kind: project.task_control.active_action_kind.clone(),
        task_id: project.task_control.active_action_task_id.clone(),
        result: "running".to_string(),
        made_progress: false,
        at: project.task_control.last_action_at.clone(),
    })
}

fn recent_action(project: &Project) -> Option<ControlActionStateView> {
    (!project.task_control.last_completed_action_id.is_empty()).then(|| ControlActionStateView {
        action_id: project.task_control.last_completed_action_id.clone(),
        kind: project.task_control.last_completed_action_kind.clone(),
        task_id: project.task_control.last_completed_action_task_id.clone(),
        result: project.task_control.last_action_result.clone(),
        made_progress: project.task_control.last_action_made_progress,
        at: project.task_control.last_action_at.clone(),
    })
}

fn control_capabilities(
    project: &Project,
    selected: Option<&Subtask>,
    selected_can_split: bool,
) -> Vec<String> {
    let mut capabilities = Vec::new();
    if let Some(state) = project.workflow_state.autopilot_state.as_ref() {
        if state.run_status == crate::project::AutopilotRunStatus::Running {
            capabilities.push("pause".to_string());
        }
        if state.run_status == crate::project::AutopilotRunStatus::Paused
            || (state.run_status == crate::project::AutopilotRunStatus::ErrorStopped
                && matches!(
                    state.recovery_action,
                    crate::project::AutopilotRecoveryAction::None
                        | crate::project::AutopilotRecoveryAction::RetryAutopilotAdvance
                ))
        {
            capabilities.push("resume".to_string());
        }
        if state.active || project.workflow_state.autopilot_active {
            capabilities.push("stop".to_string());
        }
    }
    let Some(task) = selected else {
        return capabilities;
    };
    let session_blocks_edit = project.execution_session.as_ref().is_some_and(|session| {
        session.active
            && (session.subtask_id == task.id || session.task_path.iter().any(|id| id == &task.id))
    });
    if !crate::task_tree::is_terminal(&task.status) && task.child_tasks.is_empty() {
        capabilities.push("revalidate".to_string());
        if crate::human_action_policy::evaluate(
            project,
            &task.id,
            crate::human_action_policy::HumanTerminalAction::AcceptDeviation,
        )
        .allowed
        {
            capabilities.push("accept_deviation".to_string());
        }
    }
    if !crate::task_tree::is_terminal(&task.status) && !session_blocks_edit {
        capabilities.push("recompile".to_string());
        if task.child_tasks.is_empty() && selected_can_split {
            capabilities.push("split".to_string());
        }
    }
    capabilities
}

fn milestone_node(
    project: &Project,
    milestone: &Milestone,
    current_task_id: &str,
) -> TaskTreeNodeView {
    let mut children = milestone
        .mid_stages
        .iter()
        .map(|stage| mid_stage_node(project, stage, current_task_id))
        .collect::<Vec<_>>();
    if children.is_empty() {
        children = milestone
            .subtasks
            .iter()
            .map(|task| subtask_node(project, task, None, 1, current_task_id))
            .collect();
    }
    TaskTreeNodeView {
        id: milestone.id.clone(),
        title: milestone.title.clone(),
        node_type: "Milestone".to_string(),
        status: format!("{:?}", milestone.status),
        depth: 0,
        complexity: "stage".to_string(),
        risk: "stage".to_string(),
        contract_fingerprint: String::new(),
        contract: None,
        dependencies: milestone.dependencies.clone(),
        acceptance: Vec::new(),
        capabilities: Vec::new(),
        disabled_reasons: stage_disabled_reasons(),
        is_currently_actionable: false,
        actionable_acceptance_criteria: Vec::new(),
        children,
    }
}

fn mid_stage_node(project: &Project, stage: &MidStage, current_task_id: &str) -> TaskTreeNodeView {
    TaskTreeNodeView {
        id: stage.id.clone(),
        title: stage.title.clone(),
        node_type: "MidStage".to_string(),
        status: format!("{:?}", stage.status),
        depth: 1,
        complexity: "stage".to_string(),
        risk: "stage".to_string(),
        contract_fingerprint: String::new(),
        contract: None,
        dependencies: Vec::new(),
        acceptance: Vec::new(),
        capabilities: Vec::new(),
        disabled_reasons: stage_disabled_reasons(),
        is_currently_actionable: false,
        actionable_acceptance_criteria: Vec::new(),
        children: stage
            .subtasks
            .iter()
            .map(|task| subtask_node(project, task, Some(&stage.id), 2, current_task_id))
            .collect(),
    }
}

fn subtask_node(
    project: &Project,
    task: &Subtask,
    parent: Option<&str>,
    depth: u32,
    current_task_id: &str,
) -> TaskTreeNodeView {
    let result = compile(task, parent, depth);
    let (capabilities, disabled_reasons, actionable_acceptance_criteria) =
        node_capabilities(project, task, task.id == current_task_id, &result);
    TaskTreeNodeView {
        id: task.id.clone(),
        title: task.title.clone(),
        node_type: "Subtask".to_string(),
        status: format!("{:?}", task.status),
        depth,
        complexity: format!("{:?}", result.contract.complexity),
        risk: format!("{:?}", result.contract.risk),
        contract_fingerprint: result.contract.fingerprint.clone(),
        contract: Some(result.contract),
        dependencies: task.depends_on.clone(),
        acceptance: task.acceptance_ledger.clone(),
        is_currently_actionable: !capabilities.is_empty(),
        capabilities,
        disabled_reasons,
        actionable_acceptance_criteria,
        children: task
            .child_tasks
            .iter()
            .map(|child| subtask_node(project, child, Some(&task.id), depth + 1, current_task_id))
            .collect(),
    }
}

const NODE_ACTIONS: [&str; 8] = [
    "execute",
    "revalidate",
    "split",
    "recompile",
    "accept_deviation",
    "confirm_actual_pass",
    "skip_task",
    "git_confirm",
];

fn stage_disabled_reasons() -> BTreeMap<String, String> {
    NODE_ACTIONS
        .iter()
        .map(|action| {
            (
                (*action).to_string(),
                "阶段节点只读，不能执行叶子任务动作".to_string(),
            )
        })
        .collect()
}

fn node_capabilities(
    project: &Project,
    task: &Subtask,
    is_current: bool,
    compiled: &crate::task_compiler::TaskCompileResult,
) -> (Vec<String>, BTreeMap<String, String>, Vec<u32>) {
    let mut capabilities = Vec::new();
    let mut disabled = BTreeMap::new();
    let mut allow = |action: &str| capabilities.push(action.to_string());
    let mut deny = |action: &str, reason: &str| {
        disabled.insert(action.to_string(), reason.to_string());
    };
    if !task.child_tasks.is_empty() {
        for action in NODE_ACTIONS {
            deny(action, "父任务节点只读，人工动作只能作用于叶子任务");
        }
        apply_human_action_denials(project, task, &mut disabled);
        return (capabilities, disabled, Vec::new());
    }
    if crate::task_tree::is_terminal(&task.status) {
        for action in NODE_ACTIONS {
            deny(action, "已完成任务只读");
        }
        apply_human_action_denials(project, task, &mut disabled);
        return (capabilities, disabled, Vec::new());
    }
    if !is_current {
        for action in NODE_ACTIONS {
            deny(action, "非当前任务节点只读");
        }
        apply_human_action_denials(project, task, &mut disabled);
        return (capabilities, disabled, Vec::new());
    }
    if !project.task_control.active_action_id.is_empty() {
        let reason = format!(
            "控制动作 {} 正在执行，当前快照不可写",
            project.task_control.active_action_id
        );
        for action in NODE_ACTIONS {
            deny(action, &reason);
        }
        return (capabilities, disabled, Vec::new());
    }

    if task.status == crate::project::SubtaskStatus::Pending {
        let dependencies_satisfied = crate::task_tree::locate_task(project, &task.id)
            .ok()
            .flatten()
            .is_some_and(|address| address.dependencies_satisfied);
        if dependencies_satisfied {
            allow("execute");
        } else {
            deny("execute", "叶子任务依赖尚未满足");
        }
    } else {
        deny("execute", "执行动作只能作用于 Pending 叶子任务");
    }

    allow("revalidate");
    let session_blocks_edit = project.execution_session.as_ref().is_some_and(|session| {
        session.active
            && (session.subtask_id == task.id || session.task_path.iter().any(|id| id == &task.id))
    });
    if session_blocks_edit {
        deny("recompile", "活动执行会话绑定当前任务，不能重编译");
        deny("split", "活动执行会话绑定当前任务，不能拆分");
    } else {
        allow("recompile");
        if compiled.decision.kind == crate::task_compiler::TaskCompileDecisionKind::SplitFurther {
            allow("split");
        } else {
            deny("split", &compiled.decision.reason);
        }
    }

    let accept = crate::human_action_policy::evaluate(
        project,
        &task.id,
        crate::human_action_policy::HumanTerminalAction::AcceptDeviation,
    );
    if accept.allowed {
        allow("accept_deviation");
    } else {
        deny("accept_deviation", &accept.denial_reason);
    }
    let confirm = crate::human_action_policy::evaluate(
        project,
        &task.id,
        crate::human_action_policy::HumanTerminalAction::ConfirmActualPass,
    );
    if confirm.allowed {
        allow("confirm_actual_pass");
    } else {
        deny("confirm_actual_pass", &confirm.denial_reason);
    }
    let skip = crate::human_action_policy::evaluate(
        project,
        &task.id,
        crate::human_action_policy::HumanTerminalAction::SkipTask,
    );
    if skip.allowed {
        allow("skip_task");
    } else {
        deny("skip_task", &skip.denial_reason);
    }
    if task.status == crate::project::SubtaskStatus::AwaitingConfirmation {
        allow("git_confirm");
    } else {
        deny("git_confirm", "Git 确认只能作用于待确认叶子任务");
    }
    (capabilities, disabled, accept.actionable_criteria)
}

fn apply_human_action_denials(
    project: &Project,
    task: &Subtask,
    disabled: &mut BTreeMap<String, String>,
) {
    for (name, action) in [
        (
            "accept_deviation",
            crate::human_action_policy::HumanTerminalAction::AcceptDeviation,
        ),
        (
            "confirm_actual_pass",
            crate::human_action_policy::HumanTerminalAction::ConfirmActualPass,
        ),
        (
            "skip_task",
            crate::human_action_policy::HumanTerminalAction::SkipTask,
        ),
    ] {
        let decision = crate::human_action_policy::evaluate(project, &task.id, action);
        if !decision.allowed {
            disabled.insert(name.to_string(), decision.denial_reason);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_supports_quick_and_professional_trees() {
        let mut project = Project::new("snapshot");
        project.milestones.push(Milestone {
            id: "m".into(),
            version: "v0.1".into(),
            title: "Milestone".into(),
            description: String::new(),
            tech_stack: String::new(),
            status: crate::project::MilestoneStatus::Pending,
            mode: crate::project::StageMode::Quick,
            mid_stages: Vec::new(),
            subtasks: vec![Subtask {
                id: "task".into(),
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
        project.current_milestone_id = "m".into();
        let snapshot = build(&project).unwrap();
        assert_eq!(snapshot.nodes[0].children.len(), 1);
        assert_eq!(snapshot.nodes[0].children[0].depth, 1);
    }

    #[test]
    fn snapshot_uses_the_persisted_decision_without_regenerating_it() {
        let mut project = Project::new("snapshot-decision");
        project.milestones.push(Milestone {
            id: "m".into(),
            version: "v0.1".into(),
            title: "Milestone".into(),
            description: String::new(),
            tech_stack: String::new(),
            status: crate::project::MilestoneStatus::InProgress,
            mode: crate::project::StageMode::Quick,
            mid_stages: Vec::new(),
            subtasks: vec![Subtask {
                id: "task".into(),
                status: crate::project::SubtaskStatus::Pending,
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
        project.current_milestone_id = "m".into();
        let task = &project.milestones[0].subtasks[0];
        let compiled = compile(task, None, 1);
        let decision =
            crate::control_scheduler::decide_next_action(task, &compiled, "facts", false);
        let decision_id = decision.decision_id.clone();
        project.task_control.last_decision = Some(decision);

        assert_eq!(
            build(&project).unwrap().decision.unwrap().decision_id,
            decision_id
        );
        assert_eq!(
            build(&project).unwrap().decision.unwrap().decision_id,
            decision_id
        );
    }

    #[test]
    fn phase1_human_action_safety_nodes_use_backend_capabilities() {
        let current = Subtask {
            id: "current".into(),
            title: "Current".into(),
            status: crate::project::SubtaskStatus::AwaitingConfirmation,
            execution_result: Some(crate::project::ExecutionResult {
                success: true,
                ..Default::default()
            }),
            acceptance_criteria: vec!["criterion".into()],
            acceptance_ledger: vec![AcceptanceLedgerItem {
                criterion_index: 1,
                criterion: "criterion".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let future = Subtask {
            id: "future".into(),
            title: "Future".into(),
            status: crate::project::SubtaskStatus::AwaitingConfirmation,
            execution_result: Some(crate::project::ExecutionResult {
                success: true,
                ..Default::default()
            }),
            acceptance_criteria: vec!["criterion".into()],
            acceptance_ledger: vec![AcceptanceLedgerItem {
                criterion_index: 1,
                criterion: "criterion".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut project = Project::new("node-capabilities");
        project.current_milestone_id = "m".into();
        project.milestones.push(Milestone {
            id: "m".into(),
            version: "v0.1".into(),
            title: "Milestone".into(),
            description: String::new(),
            tech_stack: String::new(),
            status: crate::project::MilestoneStatus::InProgress,
            mode: crate::project::StageMode::Quick,
            mid_stages: Vec::new(),
            subtasks: vec![current, future],
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

        let snapshot = build(&project).unwrap();
        let current = &snapshot.nodes[0].children[0];
        let future = &snapshot.nodes[0].children[1];
        assert!(current
            .capabilities
            .contains(&"accept_deviation".to_string()));
        assert_eq!(current.actionable_acceptance_criteria, vec![1]);
        assert!(future.capabilities.is_empty());
        assert_eq!(
            future
                .disabled_reasons
                .get("accept_deviation")
                .map(String::as_str),
            Some("只能操作当前叶子或当前人工恢复会话绑定的任务")
        );
    }
}
