use crate::project;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const MAX_PLANNING_REGENERATIONS: u32 = 2;

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
        "omissions:{}\nout_of_scope:{}\nnot_executable:{}\nsuggestions:{}",
        normalized_group(&result.omissions),
        normalized_group(&result.out_of_scope),
        normalized_group(&result.not_executable),
        normalized_group(&result.suggestions),
    ))
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
    let Some(mid_stage) = project
        .milestones
        .iter()
        .find(|milestone| milestone.id == project.current_milestone_id)
        .and_then(|milestone| {
            milestone
                .mid_stages
                .iter()
                .find(|mid_stage| mid_stage.id == project.current_mid_stage_id)
        })
    else {
        return PlanningAction::Stop("当前中阶段不存在。".to_string());
    };
    let Some(check) = mid_stage.plan_check_result.as_ref() else {
        return PlanningAction::CheckPlan;
    };
    if check.passed {
        return PlanningAction::CheckPlan;
    }
    if mid_stage.plan_regeneration_count >= MAX_PLANNING_REGENERATIONS {
        return PlanningAction::Stop("执行计划自动重生成已达到两次上限。".to_string());
    }
    if mid_stage.plan_no_progress_count > 0 {
        return PlanningAction::Stop("执行计划重生成没有减少阻断问题。".to_string());
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
        let session_matches_workflow = proj.workflow_state.current_step
            == project::WorkflowStep::Execution
            && session.milestone_id == proj.current_milestone_id
            && session.mid_stage_id == proj.current_mid_stage_id
            && proj.milestones.iter().any(|milestone| {
                milestone.id == session.milestone_id
                    && milestone.mid_stages.iter().any(|mid| {
                        mid.id == session.mid_stage_id
                            && mid.subtasks.iter().any(|subtask| {
                                subtask.id == session.subtask_id
                                    && subtask.status == project::SubtaskStatus::Executing
                            })
                    })
            });
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
            PlanningAction::RegeneratePlan => {
                let Some(mid) = target
                    .mid_stages
                    .iter()
                    .find(|mid| mid.id == proj.current_mid_stage_id)
                else {
                    return permanent_block(
                        "当前中阶段不存在。",
                        project::AutopilotRecoveryAction::WaitHumanDecision,
                    );
                };
                next_step(
                    "regenerate_execution_plan",
                    serde_json::json!({
                        "projectName": project_name,
                        "expectedDataRevision": proj.workflow_state.data_revision,
                        "expectedPlanDraftRevision": mid.plan_draft_revision,
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
    let current = target
        .mid_stages
        .iter()
        .find(|mid| mid.id == proj.current_mid_stage_id);
    let Some(current) = current else {
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
    };

    let has_awaiting = current
        .subtasks
        .iter()
        .any(|item| item.status == project::SubtaskStatus::AwaitingConfirmation);
    let has_pending = current
        .subtasks
        .iter()
        .any(|item| item.status == project::SubtaskStatus::Pending);
    let has_rejected = current
        .subtasks
        .iter()
        .any(|item| item.status == project::SubtaskStatus::Rejected);
    if has_rejected && !has_awaiting && !has_pending {
        return permanent_block(
            format!(
                "中阶段「{}」存在已驳回的小阶段，需要人工决定是否重试或重新生成执行计划",
                current.title
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
    let mut decision = transition(
        project_name,
        "MilestoneReview",
        "autopilot: 所有中阶段完成，进入大阶段审阅",
        "所有中阶段已完成，进入大阶段审阅",
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
    fn fingerprints_ignore_whitespace_order_and_duplicates() {
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
}
