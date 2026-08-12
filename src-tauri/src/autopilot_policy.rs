use crate::project;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const MAX_PLANNING_REGENERATIONS: u32 = 2;
pub(crate) const MAX_PLAN_NO_PROGRESS: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AutopilotDecisionKind {
    Execute,
    WaitExecution,
    RetryAfter,
    HumanBoundary,
    PermanentBlock,
    MilestoneComplete,
    InitializeQualityRecovery,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutopilotNextStep {
    pub command: String,
    pub args: serde_json::Value,
    pub description: String,
    pub at_milestone_boundary: bool,
    pub is_error: bool,
    pub error_message: String,
    pub result_kind: project::AutopilotCommandResultKind,
    #[serde(default)]
    pub waiting_for_execution: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct AutopilotTerminalDirective {
    pub run_status: project::AutopilotRunStatus,
    pub recovery_action: project::AutopilotRecoveryAction,
}

#[derive(Debug, Clone)]
pub(crate) struct AutopilotDecision {
    pub kind: AutopilotDecisionKind,
    pub next: AutopilotNextStep,
    pub terminal: Option<AutopilotTerminalDirective>,
    pub quality_recovery_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct AutopilotPolicyBlock {
    pub description: String,
    pub recovery_action: project::AutopilotRecoveryAction,
}

#[derive(Debug, Clone)]
pub(crate) enum QualityGateFact {
    NotApplicable,
    Passed,
    Failed(String),
}

#[derive(Debug, Clone)]
pub(crate) struct AutopilotPolicyFacts {
    pub precondition_block: Option<AutopilotPolicyBlock>,
    pub quality_gate: QualityGateFact,
    pub needs_calibration: bool,
}

fn next_step(
    command: &str,
    args: serde_json::Value,
    description: impl Into<String>,
    result_kind: project::AutopilotCommandResultKind,
) -> AutopilotDecision {
    AutopilotDecision {
        kind: AutopilotDecisionKind::Execute,
        next: AutopilotNextStep {
            command: command.to_string(),
            args,
            description: description.into(),
            at_milestone_boundary: false,
            is_error: false,
            error_message: String::new(),
            result_kind,
            waiting_for_execution: false,
        },
        terminal: None,
        quality_recovery_reason: None,
    }
}

fn waiting(description: impl Into<String>) -> AutopilotDecision {
    AutopilotDecision {
        kind: AutopilotDecisionKind::WaitExecution,
        next: AutopilotNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: description.into(),
            at_milestone_boundary: false,
            is_error: false,
            error_message: String::new(),
            result_kind: project::AutopilotCommandResultKind::NoResult,
            waiting_for_execution: true,
        },
        terminal: None,
        quality_recovery_reason: None,
    }
}

fn terminal(
    kind: AutopilotDecisionKind,
    description: impl Into<String>,
    run_status: project::AutopilotRunStatus,
    recovery_action: project::AutopilotRecoveryAction,
    at_milestone_boundary: bool,
    is_error: bool,
) -> AutopilotDecision {
    let description = description.into();
    AutopilotDecision {
        kind,
        next: AutopilotNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: description.clone(),
            at_milestone_boundary,
            is_error,
            error_message: if is_error {
                description.clone()
            } else {
                String::new()
            },
            result_kind: project::AutopilotCommandResultKind::NoResult,
            waiting_for_execution: false,
        },
        terminal: Some(AutopilotTerminalDirective {
            run_status,
            recovery_action,
        }),
        quality_recovery_reason: None,
    }
}

fn permanent_block(
    description: impl Into<String>,
    recovery_action: project::AutopilotRecoveryAction,
) -> AutopilotDecision {
    terminal(
        AutopilotDecisionKind::PermanentBlock,
        description,
        project::AutopilotRunStatus::ErrorStopped,
        recovery_action,
        false,
        true,
    )
}

fn milestone_boundary() -> AutopilotDecision {
    terminal(
        AutopilotDecisionKind::HumanBoundary,
        "到达大阶段边界，等待人工 A/B/C 决策",
        project::AutopilotRunStatus::WaitingMilestoneReview,
        project::AutopilotRecoveryAction::WaitHumanDecision,
        true,
        false,
    )
}

fn transition(
    project_name: &str,
    target: &str,
    reason: &str,
    description: &str,
) -> AutopilotDecision {
    next_step(
        "transition_workflow",
        serde_json::json!({
            "projectName": project_name,
            "targetStep": target,
            "reason": reason,
        }),
        description,
        project::AutopilotCommandResultKind::ProjectState,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PlanningAction {
    CheckMidStage,
    RegenerateMidStage,
    CheckPlan,
    PassPlanCheck,
    RegeneratePlan,
    Stop(String),
}

fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub(crate) fn text_fingerprint(value: &str) -> String {
    digest(&normalize_text(value))
}

fn normalized_group(values: &[String]) -> String {
    let mut normalized = values
        .iter()
        .map(|value| normalize_text(value))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized.join("\n")
}

pub(crate) fn plan_failure_fingerprint(result: &project::StagePlanCheckResult) -> String {
    digest(&format!(
        "omissions:{}\nout_of_scope:{}\nnot_executable:{}",
        normalized_group(&result.omissions),
        normalized_group(&result.out_of_scope),
        normalized_group(&result.not_executable),
    ))
}

fn suggestion_only_issue(value: &str) -> bool {
    let normalized = normalize_text(value);
    ["可考虑", "建议", "可选", "optional"]
        .iter()
        .any(|marker| normalized.contains(marker))
}

fn normalize_issue_group(values: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() || normalized.iter().any(|existing| existing == value) {
            continue;
        }
        normalized.push(value.to_string());
    }
    normalized
}

fn move_suggestions(blocking: &mut Vec<String>, suggestions: &mut Vec<String>) {
    let (misclassified, retained): (Vec<_>, Vec<_>) = std::mem::take(blocking)
        .into_iter()
        .partition(|issue| suggestion_only_issue(issue));
    *blocking = retained;
    suggestions.extend(misclassified);
}

pub(crate) fn normalize_plan_check_result(
    mut result: project::StagePlanCheckResult,
) -> project::StagePlanCheckResult {
    move_suggestions(&mut result.omissions, &mut result.suggestions);
    move_suggestions(&mut result.out_of_scope, &mut result.suggestions);
    move_suggestions(&mut result.not_executable, &mut result.suggestions);
    result.omissions = normalize_issue_group(result.omissions);
    result.out_of_scope = normalize_issue_group(result.out_of_scope);
    result.not_executable = normalize_issue_group(result.not_executable);
    result.suggestions = normalize_issue_group(result.suggestions);
    result.passed = blocking_plan_issue_count(&result) == 0;
    result
}

pub(crate) fn blocking_plan_issue_count(result: &project::StagePlanCheckResult) -> u32 {
    result
        .omissions
        .len()
        .saturating_add(result.out_of_scope.len())
        .saturating_add(result.not_executable.len())
        .try_into()
        .unwrap_or(u32::MAX)
}

pub(crate) fn mid_stage_candidate_fingerprint(candidates: &[project::MidStage]) -> String {
    let mut rows = candidates
        .iter()
        .map(|candidate| {
            format!(
                "{}|{}|{}|{}",
                normalize_text(&candidate.title),
                normalize_text(&candidate.description),
                normalize_text(&candidate.tech_focus),
                candidate.order.unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    digest(&rows.join("\n"))
}

pub(crate) fn mid_stage_planning_action(project: &project::Project) -> PlanningAction {
    let Some(draft) = project.mid_stage_draft.as_ref() else {
        return PlanningAction::CheckMidStage;
    };
    if draft.status != project::MidStageDraftStatus::CheckFailed {
        return PlanningAction::CheckMidStage;
    }
    if draft.regeneration_count >= MAX_PLANNING_REGENERATIONS {
        return PlanningAction::Stop("中阶段草稿自动重生成已达到两次上限。".to_string());
    }
    if draft.no_progress_count > 0 {
        return PlanningAction::Stop("中阶段草稿重生成没有结构进展。".to_string());
    }
    PlanningAction::RegenerateMidStage
}

pub(crate) fn plan_planning_action(project: &project::Project) -> PlanningAction {
    let scope = match crate::plan_scope::PlanScope::resolve(project) {
        Ok(scope) => scope,
        Err(error) => return PlanningAction::Stop(error),
    };
    let Some(check) = scope.plan_check_result(project) else {
        return PlanningAction::CheckPlan;
    };
    let blocking_count = blocking_plan_issue_count(check);
    if blocking_count == 0 {
        return PlanningAction::PassPlanCheck;
    }
    let blockers = check
        .omissions
        .iter()
        .chain(&check.out_of_scope)
        .chain(&check.not_executable)
        .cloned()
        .collect::<Vec<_>>()
        .join("；");
    if scope.plan_regeneration_count(project) >= MAX_PLANNING_REGENERATIONS {
        return PlanningAction::Stop(format!(
            "执行计划自动重生成已达到两次上限。具体硬阻断：{}",
            blockers
        ));
    }
    if scope.plan_no_progress_count(project) >= MAX_PLAN_NO_PROGRESS {
        return PlanningAction::Stop(format!(
            "执行计划连续两次没有减少硬阻断。具体硬阻断：{}",
            blockers
        ));
    }
    PlanningAction::RegeneratePlan
}

pub(crate) fn decide_next_step(
    proj: &project::Project,
    project_name: &str,
    facts: &AutopilotPolicyFacts,
) -> AutopilotDecision {
    if !proj.workflow_state.autopilot_active {
        return permanent_block(
            "自动驾驶未激活",
            project::AutopilotRecoveryAction::WaitHumanDecision,
        );
    }

    if let Some(state) = proj.workflow_state.autopilot_state.as_ref() {
        match state.run_status {
            project::AutopilotRunStatus::Paused => {
                return terminal(
                    AutopilotDecisionKind::HumanBoundary,
                    "自动驾驶已暂停，等待手动操作",
                    project::AutopilotRunStatus::Paused,
                    project::AutopilotRecoveryAction::None,
                    false,
                    false,
                );
            }
            project::AutopilotRunStatus::ErrorStopped => {
                if state.next_retry_at.is_some()
                    && crate::autopilot_failure::is_transient(&state.last_failure_kind)
                {
                    let mut decision = waiting("瞬时错误等待后端自动重试");
                    decision.kind = AutopilotDecisionKind::RetryAfter;
                    return decision;
                }
                return permanent_block(
                    format!("自动驾驶因错误停止：{}", state.error_message),
                    state.recovery_action.clone(),
                );
            }
            project::AutopilotRunStatus::WaitingMilestoneReview => return milestone_boundary(),
            project::AutopilotRunStatus::Running => {}
        }
    }

    if let Some(recovery) = proj.workflow_state.recovery_state.as_ref() {
        if matches!(
            recovery.phase,
            project::RecoveryPhase::Diagnosing
                | project::RecoveryPhase::Repairing
                | project::RecoveryPhase::Retesting
                | project::RecoveryPhase::Replanning
        ) {
            if crate::recovery::is_review_validation_recovery(recovery)
                && !crate::recovery::validation_retry_due(recovery)
            {
                let mut decision = waiting(format!(
                    "等待第 {}/{} 次 AI 审查验证重试",
                    recovery.validation_retry_count.saturating_add(1),
                    recovery.max_validation_retries
                ));
                decision.kind = AutopilotDecisionKind::RetryAfter;
                return decision;
            }
            let recovery_is_running = proj.execution_session.as_ref().is_some_and(|session| {
                session.active
                    && session.status.eq_ignore_ascii_case("recovering")
                    && session.execution_id == recovery.execution_id
            });
            let recovery_action_is_claimed = proj
                .workflow_state
                .autopilot_state
                .as_ref()
                .is_some_and(|state| {
                    !state.current_action_id.is_empty()
                        && state.current_action_kind == "run_error_recovery"
                });
            if recovery_is_running
                && recovery_action_is_claimed
                && matches!(
                    recovery.phase,
                    project::RecoveryPhase::Repairing
                        | project::RecoveryPhase::Retesting
                        | project::RecoveryPhase::Replanning
                )
            {
                return waiting("错误恢复任务仍在运行，等待当前修复完成");
            }
            // Persistent recovery without a live worker is bounded.
            let autopilot = proj.workflow_state.autopilot_state.as_ref();
            let disposition = crate::recovery::reconcile_stalled_recovery(
                crate::recovery::StalledRecoveryInput {
                    phase: &recovery.phase,
                    updated_at: &recovery.updated_at,
                    replan_execution_attempted: recovery.replan_execution_attempted,
                    next_validation_retry_at: recovery.next_validation_retry_at.as_deref(),
                    is_validation_recovery: crate::recovery::is_review_validation_recovery(recovery),
                    action_id: autopilot
                        .map(|s| s.current_action_id.as_str())
                        .unwrap_or(""),
                    action_kind: autopilot
                        .map(|s| s.current_action_kind.as_str())
                        .unwrap_or(""),
                    action_started_at: autopilot
                        .map(|s| s.action_started_at.as_str())
                        .unwrap_or(""),
                    now: chrono::Utc::now(),
                },
            );
            match disposition {
                crate::recovery::StalledRecoveryDisposition::AllowAutomaticClaim => {}
                crate::recovery::StalledRecoveryDisposition::Wait => {
                    return waiting(if recovery_action_is_claimed {
                        "错误恢复任务仍在运行，等待当前修复完成"
                    } else {
                        "自动恢复等待中"
                    });
                }
                crate::recovery::StalledRecoveryDisposition::MarkStalled
                | crate::recovery::StalledRecoveryDisposition::EnterHumanBoundary => {
                    let detail = if recovery.last_repair_summary.is_empty() {
                        format!("{:?}", recovery.phase)
                    } else {
                        recovery.last_repair_summary.clone()
                    };
                    return permanent_block(
                        format!("自动恢复已停滞或进入人工边界：{}", detail),
                        project::AutopilotRecoveryAction::WaitHumanDecision,
                    );
                }
            }
            return next_step(
                "run_error_recovery",
                serde_json::json!({ "projectName": project_name }),
                match recovery.phase {
                    project::RecoveryPhase::Diagnosing => "正在诊断错误",
                    project::RecoveryPhase::Repairing => "正在继续受限修复",
                    project::RecoveryPhase::Retesting => "正在重新测试",
                    project::RecoveryPhase::Replanning => "正在重新规划当前任务",
                    _ => "正在恢复",
                },
                project::AutopilotCommandResultKind::ProjectState,
            );
        }
    }

    if let Some(session) = proj
        .execution_session
        .as_ref()
        .filter(|session| session.active && session.status.eq_ignore_ascii_case("executing"))
    {
        let session_task_matches = crate::task_tree::locate_task(proj, &session.subtask_id)
            .ok()
            .flatten()
            .is_some_and(|address| {
                address.milestone_id == session.milestone_id
                    && address.mid_stage_id == session.mid_stage_id
                    && crate::task_tree::find_task(proj, &session.subtask_id)
                        .ok()
                        .flatten()
                        .is_some_and(|subtask| subtask.status == project::SubtaskStatus::Executing)
            });
        let session_matches_workflow = proj.workflow_state.current_step
            == project::WorkflowStep::Execution
            && session.milestone_id == proj.current_milestone_id
            && session.mid_stage_id == proj.current_mid_stage_id
            && session_task_matches;
        if session_matches_workflow {
            return waiting(format!(
                "小阶段「{}」正在执行，等待当前执行完成",
                session.subtask_title
            ));
        }
        return permanent_block(
            "活动执行会话与当前工作流上下文不一致，请同步后关闭自动驾驶",
            project::AutopilotRecoveryAction::SyncAndClose,
        );
    }

    let step = &proj.workflow_state.current_step;
    if *step == project::WorkflowStep::MilestoneReview {
        return milestone_boundary();
    }

    let target_id = &proj.workflow_state.autopilot_target_milestone_id;
    let Some(target) = proj.milestones.iter().find(|item| item.id == *target_id) else {
        return permanent_block(
            "目标大阶段不存在",
            project::AutopilotRecoveryAction::WaitHumanDecision,
        );
    };
    if let Some(block) = facts.precondition_block.as_ref() {
        return permanent_block(block.description.clone(), block.recovery_action.clone());
    }
    if proj.current_milestone_id.is_empty() || proj.current_milestone_id != *target_id {
        return next_step(
            "select_milestone",
            serde_json::json!({ "projectName": project_name, "milestoneId": target.id }),
            format!("选择大阶段：{}", target.title),
            project::AutopilotCommandResultKind::ProjectState,
        );
    }

    use project::WorkflowStep::*;
    match step {
        MilestoneSelection if target.mode == project::StageMode::Quick => {
            match crate::workflow_resolution::resolve_direct_milestone_step(target) {
                Ok(project::WorkflowStep::PlanGeneration) => transition(
                    project_name,
                    "PlanGeneration",
                    "autopilot: Quick 大阶段直接生成执行计划",
                    "进入大阶段直挂计划生成",
                ),
                Ok(project::WorkflowStep::PlanCheck) => transition(
                    project_name,
                    "PlanCheck",
                    "autopilot: 恢复 Quick 大阶段计划检查",
                    "恢复大阶段直挂计划检查",
                ),
                Ok(project::WorkflowStep::PlanApproving) => transition(
                    project_name,
                    "PlanApproving",
                    "autopilot: 恢复 Quick 大阶段计划批准",
                    "恢复大阶段直挂计划批准",
                ),
                Ok(project::WorkflowStep::Execution) => transition(
                    project_name,
                    "Execution",
                    "autopilot: 恢复 Quick 大阶段执行",
                    "恢复大阶段直挂任务执行",
                ),
                Ok(project::WorkflowStep::MilestoneReview) => transition(
                    project_name,
                    "MilestoneReview",
                    "autopilot: Quick 大阶段任务均已完成",
                    "进入大阶段审阅",
                ),
                Ok(step) => permanent_block(
                    format!("Quick 大阶段解析到不支持的步骤：{:?}", step),
                    project::AutopilotRecoveryAction::WaitHumanDecision,
                ),
                Err(reason) => {
                    permanent_block(reason, project::AutopilotRecoveryAction::WaitHumanDecision)
                }
            }
        }
        MilestoneSelection => match crate::workflow_resolution::resolve_mid_stage_route(target) {
            crate::workflow_resolution::MidStageRoute::NeedsInitialGeneration => transition(
                project_name,
                "MidStageGeneration",
                "autopilot: 首次生成中阶段",
                "当前大阶段没有中阶段，进入首次生成",
            ),
            crate::workflow_resolution::MidStageRoute::SelectExisting { mid_stage_id }
            | crate::workflow_resolution::MidStageRoute::ResumeExisting { mid_stage_id, .. } => {
                next_step(
                    "select_mid_stage",
                    serde_json::json!({ "projectName": project_name, "midStageId": mid_stage_id }),
                    "按项目事实选择或恢复现有中阶段",
                    project::AutopilotCommandResultKind::ProjectState,
                )
            }
            crate::workflow_resolution::MidStageRoute::ReviewMilestone => transition(
                project_name,
                "MilestoneReview",
                "autopilot: 当前大阶段的中阶段均已完成",
                "进入大阶段审阅",
            ),
            crate::workflow_resolution::MidStageRoute::WaitHuman { reason } => {
                permanent_block(reason, project::AutopilotRecoveryAction::WaitHumanDecision)
            }
        },
        MidStageGeneration => next_step(
            "generate_mid_stage_draft",
            serde_json::json!({ "projectName": project_name }),
            "生成中阶段草稿",
            project::AutopilotCommandResultKind::ProjectState,
        ),
        MidStageCheck => match mid_stage_planning_action(proj) {
            PlanningAction::CheckMidStage => next_step(
                "check_mid_stage_draft",
                serde_json::json!({ "projectName": project_name }),
                "检查中阶段草稿",
                project::AutopilotCommandResultKind::ProjectState,
            ),
            PlanningAction::RegenerateMidStage => {
                let Some(draft) = proj.mid_stage_draft.as_ref() else {
                    return permanent_block(
                        "没有中阶段草稿。",
                        project::AutopilotRecoveryAction::WaitHumanDecision,
                    );
                };
                next_step(
                    "regenerate_mid_stage_draft",
                    serde_json::json!({
                        "projectName": project_name,
                        "currentDraftId": draft.draft_id,
                        "expectedDataRevision": proj.workflow_state.data_revision,
                        "feedback": draft.check_result.clone().unwrap_or_default(),
                        "source": "check_failed",
                    }),
                    "按检查结果重新生成中阶段草稿",
                    project::AutopilotCommandResultKind::ProjectState,
                )
            }
            PlanningAction::Stop(reason) => {
                permanent_block(reason, project::AutopilotRecoveryAction::WaitHumanDecision)
            }
            _ => unreachable!("中阶段策略返回了执行计划动作"),
        },
        MidStageApproval => next_step(
            "approve_mid_stage_draft",
            serde_json::json!({ "projectName": project_name }),
            "批准中阶段草稿",
            project::AutopilotCommandResultKind::ProjectState,
        ),
        MidStageSelection if !proj.current_mid_stage_id.is_empty() => {
            let approved = target
                .mid_stages
                .iter()
                .find(|mid| mid.id == proj.current_mid_stage_id)
                .is_some_and(|mid| !mid.subtasks.is_empty() && mid.plan_approved_at.is_some());
            if approved {
                transition(
                    project_name,
                    "Execution",
                    "autopilot: 进入执行阶段",
                    "进入执行阶段",
                )
            } else {
                transition(
                    project_name,
                    "PlanGeneration",
                    "autopilot: 进入执行计划生成",
                    "进入执行计划生成",
                )
            }
        }
        MidStageSelection => {
            let Some(mid) = target
                .mid_stages
                .iter()
                .find(|mid| mid.status != project::MidStageStatus::Completed)
            else {
                return permanent_block(
                    "没有未完成的中阶段",
                    project::AutopilotRecoveryAction::WaitHumanDecision,
                );
            };
            next_step(
                "select_mid_stage",
                serde_json::json!({ "projectName": project_name, "midStageId": mid.id }),
                format!("选择中阶段：{}", mid.title),
                project::AutopilotCommandResultKind::ProjectState,
            )
        }
        PlanGeneration => next_step(
            "generate_execution_plan",
            serde_json::json!({ "projectName": project_name }),
            "生成执行计划",
            project::AutopilotCommandResultKind::ProjectState,
        ),
        PlanCheck => match plan_planning_action(proj) {
            PlanningAction::CheckPlan => next_step(
                "check_stage_plan",
                serde_json::json!({ "projectName": project_name }),
                "检查执行计划",
                project::AutopilotCommandResultKind::ProjectState,
            ),
            PlanningAction::PassPlanCheck => transition(
                project_name,
                "PlanApproving",
                "autopilot: 计划仅有建议或硬阻断为空，按后端分级通过",
                "计划硬阻断为空，进入批准阶段",
            ),
            PlanningAction::RegeneratePlan => {
                let scope = match crate::plan_scope::PlanScope::resolve(proj) {
                    Ok(scope) => scope,
                    Err(error) => {
                        return permanent_block(
                            error,
                            project::AutopilotRecoveryAction::WaitHumanDecision,
                        );
                    }
                };
                next_step(
                    "regenerate_execution_plan",
                    serde_json::json!({
                        "projectName": project_name,
                        "expectedDataRevision": proj.workflow_state.data_revision,
                        "expectedPlanDraftRevision": scope.plan_draft_revision(proj),
                        "feedback": "",
                        "source": "check_failed",
                    }),
                    "按检查结果重新生成执行计划",
                    project::AutopilotCommandResultKind::ProjectState,
                )
            }
            PlanningAction::Stop(reason) => {
                permanent_block(reason, project::AutopilotRecoveryAction::WaitHumanDecision)
            }
            _ => unreachable!("执行计划策略返回了中阶段动作"),
        },
        PlanApproving => next_step(
            "approve_stage_plan",
            serde_json::json!({ "projectName": project_name }),
            "批准执行计划，进入执行阶段",
            project::AutopilotCommandResultKind::ProjectState,
        ),
        Execution => decide_execution(proj, project_name, target, facts),
        Discussion
        | BranchDiscussion
        | PauseDecision
        | RollbackPreview
        | FuturePlanApproval
        | ThreeChecks
        | ProjectPlanGeneration
        | PlanApproval => permanent_block(
            format!("当前步骤 {:?} 需要人工介入，无法自动推进", step),
            project::AutopilotRecoveryAction::WaitHumanDecision,
        ),
        MilestoneGeneration | MilestoneCheck | MilestoneApproval => permanent_block(
            "请先手动完成大阶段生成、检查和批准，然后激活自动驾驶。",
            project::AutopilotRecoveryAction::WaitHumanDecision,
        ),
        _ => permanent_block(
            format!("自动驾驶不支持从 {:?} 自动推进", step),
            project::AutopilotRecoveryAction::WaitHumanDecision,
        ),
    }
}

fn decide_execution(
    proj: &project::Project,
    project_name: &str,
    target: &project::Milestone,
    facts: &AutopilotPolicyFacts,
) -> AutopilotDecision {
    if target.mode == project::StageMode::Professional && proj.current_mid_stage_id.is_empty() {
        if let Some(next) = target
            .mid_stages
            .iter()
            .find(|mid| mid.status != project::MidStageStatus::Completed)
        {
            return next_step(
                "select_mid_stage",
                serde_json::json!({ "projectName": project_name, "midStageId": next.id }),
                format!("选择中阶段：{}", next.title),
                project::AutopilotCommandResultKind::ProjectState,
            );
        }
        let mut decision = transition(
            project_name,
            "MilestoneReview",
            "autopilot: 所有中阶段完成，进入大阶段审阅",
            "所有中阶段已完成，进入大阶段审阅",
        );
        decision.kind = AutopilotDecisionKind::MilestoneComplete;
        decision.next.at_milestone_boundary = true;
        return decision;
    }
    let scope = match crate::plan_scope::PlanScope::resolve(proj) {
        Ok(scope) => scope,
        Err(error) => {
            return permanent_block(error, project::AutopilotRecoveryAction::WaitHumanDecision);
        }
    };
    let tasks = scope.subtasks(proj);
    let target_title = scope
        .mid_stage(proj)
        .map(|stage| stage.title.as_str())
        .unwrap_or(target.title.as_str());

    let has_awaiting = tasks
        .iter()
        .any(|item| item.status == project::SubtaskStatus::AwaitingConfirmation);
    let has_pending = tasks
        .iter()
        .any(|item| item.status == project::SubtaskStatus::Pending);
    let has_rejected = tasks
        .iter()
        .any(|item| item.status == project::SubtaskStatus::Rejected);
    if has_rejected && !has_awaiting && !has_pending {
        return permanent_block(
            format!(
                "计划目标「{}」存在已驳回的小阶段，需要人工决定是否重试或重新生成执行计划",
                target_title
            ),
            project::AutopilotRecoveryAction::WaitHumanDecision,
        );
    }
    if has_awaiting {
        return match &facts.quality_gate {
            QualityGateFact::Passed => next_step(
                "confirm_subtask_result",
                serde_json::json!({ "projectName": project_name }),
                "自动确认小阶段执行结果",
                project::AutopilotCommandResultKind::ProjectState,
            ),
            QualityGateFact::Failed(reason) => AutopilotDecision {
                kind: AutopilotDecisionKind::InitializeQualityRecovery,
                next: AutopilotNextStep {
                    command: String::new(),
                    args: serde_json::json!({}),
                    description: format!("质量门禁未通过：{}", reason),
                    at_milestone_boundary: false,
                    is_error: false,
                    error_message: String::new(),
                    result_kind: project::AutopilotCommandResultKind::NoResult,
                    waiting_for_execution: false,
                },
                terminal: None,
                quality_recovery_reason: Some(reason.clone()),
            },
            QualityGateFact::NotApplicable => permanent_block(
                "待确认任务缺少质量门禁事实。",
                project::AutopilotRecoveryAction::WaitHumanDecision,
            ),
        };
    }
    if has_pending {
        return next_step(
            if facts.needs_calibration {
                "calibrate_next_subtask_command"
            } else {
                "execute_current_subtask"
            },
            serde_json::json!({ "projectName": project_name }),
            if facts.needs_calibration {
                "扫描最新代码事实并按需校准下一任务"
            } else {
                "执行下一个待处理小阶段"
            },
            if facts.needs_calibration {
                project::AutopilotCommandResultKind::ProjectState
            } else {
                project::AutopilotCommandResultKind::PipelineState
            },
        );
    }

    if let Some(current) = scope.mid_stage(proj) {
        if let Some(next) = target
            .mid_stages
            .iter()
            .filter(|mid| mid.id != current.id)
            .find(|mid| mid.status != project::MidStageStatus::Completed)
        {
            return next_step(
                "select_mid_stage",
                serde_json::json!({ "projectName": project_name, "midStageId": next.id }),
                format!(
                    "中阶段「{}」已完成，切换到下一中阶段：{}",
                    current.title, next.title
                ),
                project::AutopilotCommandResultKind::ProjectState,
            );
        }
    }
    let mut decision = transition(
        project_name,
        "MilestoneReview",
        "autopilot: 当前大阶段的全部计划任务完成，进入大阶段审阅",
        "当前大阶段任务已完成，进入大阶段审阅",
    );
    decision.kind = AutopilotDecisionKind::MilestoneComplete;
    decision.next.at_milestone_boundary = true;
    decision
}

pub(crate) fn resolve_quality_recovery(
    project_name: &str,
    gate_reason: &str,
    automatic: bool,
) -> AutopilotDecision {
    if automatic {
        next_step(
            "run_error_recovery",
            serde_json::json!({ "projectName": project_name }),
            "质量门禁未通过，开始受限自动修复",
            project::AutopilotCommandResultKind::ProjectState,
        )
    } else {
        permanent_block(
            format!("质量门禁阻断：{}", gate_reason),
            project::AutopilotRecoveryAction::WaitHumanDecision,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_convergence_fingerprints_ignore_whitespace_order_and_duplicates() {
        let left = project::StagePlanCheckResult {
            passed: false,
            omissions: vec![" Missing API ".to_string(), "missing api".to_string()],
            out_of_scope: vec!["Extra file".to_string()],
            not_executable: vec![],
            suggestions: vec![],
            checked_at: String::new(),
        };
        let right = project::StagePlanCheckResult {
            omissions: vec!["missing   api".to_string()],
            out_of_scope: vec![" extra FILE ".to_string()],
            ..left.clone()
        };
        assert_eq!(
            plan_failure_fingerprint(&left),
            plan_failure_fingerprint(&right)
        );
        assert_eq!(blocking_plan_issue_count(&left), 3);
    }

    #[test]
    fn check_convergence_suggestions_do_not_block_or_change_failure_fingerprint() {
        let base = project::StagePlanCheckResult {
            passed: false,
            omissions: vec!["可考虑调用 loadSearchConfig 优化兼容性".to_string()],
            out_of_scope: vec![],
            not_executable: vec![],
            suggestions: vec!["可选：补充说明".to_string()],
            checked_at: String::new(),
        };
        let normalized = normalize_plan_check_result(base);
        assert!(normalized.passed);
        assert!(normalized.omissions.is_empty());
        assert_eq!(normalized.suggestions.len(), 2);

        let mut changed_suggestions = normalized.clone();
        changed_suggestions.suggestions = vec!["另一条建议".to_string()];
        assert_eq!(
            plan_failure_fingerprint(&normalized),
            plan_failure_fingerprint(&changed_suggestions)
        );
    }

    #[test]
    fn check_convergence_real_plan_omissions_remain_blocking() {
        let normalized = normalize_plan_check_result(project::StagePlanCheckResult {
            passed: true,
            omissions: vec!["缺少必需的配置持久化产物".to_string()],
            out_of_scope: vec![],
            not_executable: vec![],
            suggestions: vec![],
            checked_at: String::new(),
        });
        assert!(!normalized.passed);
        assert_eq!(blocking_plan_issue_count(&normalized), 1);
    }
}
