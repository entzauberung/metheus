// src-tauri/src/commands/workflow.rs — 集中工作流状态转换
use crate::project;
use serde::{Deserialize, Serialize};

/// 合法的工作流转换表
/// (from_step, to_step) -> bool
fn is_valid_transition(from: &project::WorkflowStep, to: &project::WorkflowStep) -> bool {
    use project::WorkflowStep::*;
    matches!(
        (from, to),
        // Before -> First Discussion
        (WaitingEntry, ExistingAnalysis)
        | (WaitingEntry, Discussion)
        // Half Project
        | (ExistingAnalysis, BaselineApproval)
        | (BaselineApproval, Discussion)
        // Discussion -> checks -> plan
        | (Discussion, ThreeChecks)
        | (ThreeChecks, Discussion)          // check failed
        | (ThreeChecks, PlanApproval)        // when generating plan draft, stays at PlanApproval
        // Plan flow
        | (PlanApproval, Discussion)         // rejected
        | (PlanApproval, MilestoneGeneration) // entering console
        // Console planning chain
        | (MilestoneGeneration, MilestoneCheck)
        | (MilestoneCheck, MilestoneGeneration)      // 仅描述语义；正式重生成由原子业务命令完成
        | (MilestoneCheck, MilestoneApproval)        // check passed
        | (MilestoneApproval, MilestoneGeneration)   // 仅描述语义；正式重生成由原子业务命令完成
        | (MilestoneApproval, MilestoneSelection)    // approved
        | (MilestoneSelection, MidStageGeneration)
        | (MidStageGeneration, MidStageCheck)
        | (MidStageCheck, MidStageGeneration) // 仅描述语义；正式重生成由原子业务命令完成
        | (MidStageCheck, Discussion)        // check failed -> branch discussion for fix
        | (MidStageCheck, MidStageApproval)
        | (MidStageApproval, MidStageGeneration) // 仅描述语义；正式重生成由原子业务命令完成
        | (MidStageApproval, MidStageSelection)
        | (MidStageSelection, PlanGeneration)
        | (PlanGeneration, PlanCheck)
        | (PlanCheck, PlanGeneration)        // 仅描述语义；正式重生成由原子业务命令完成
        | (PlanCheck, Discussion)            // check failed -> discussion
        | (PlanCheck, PlanApproving)
        | (PlanApproving, PlanGeneration)    // 仅描述语义；正式重生成由原子业务命令完成
        | (PlanApproving, MidStageSelection) // re-generate plan
        | (PlanApproving, Execution)
        // Execution flow
        | (Execution, PauseDecision)
        | (Execution, MilestoneReview)       // all mid stages complete
        | (Execution, Discussion)            // execution failure -> discussion
        | (PauseDecision, Discussion)        // adjust only -> discussion
        | (PauseDecision, Execution)         // continue
        | (PauseDecision, RollbackPreview)
        // Branch discussion
        | (Discussion, MilestoneReview)      // user decides to review again
        | (MilestoneReview, MilestoneSelection)  // A: continue to next milestone
        | (MilestoneReview, Discussion)          // B or C: enters branch discussion
        | (Discussion, FuturePlanApproval)       // C: draft generated
        | (FuturePlanApproval, MilestoneSelection) // C: approved
        | (MilestoneReview, project::WorkflowStep::Completed) // last milestone A
        // Rollback
        | (RollbackPreview, Discussion)      // cancel rollback
        | (RollbackPreview, PlanGeneration) // confirmed rollback
    )
}

/// Allow returning to Discussion from non-execution steps
fn can_enter_discussion(from: &project::WorkflowStep) -> bool {
    use project::WorkflowStep::*;
    // PlanApproval → Discussion 必须通过 reject_version_plan 命令（会清除 preflight_results）
    matches!(
        from,
        Discussion
            | ThreeChecks
            | MilestoneSelection
            | MidStageCheck
            | PlanCheck
            | RollbackPreview
            | BranchDiscussion
            | MilestoneReview
            | FuturePlanApproval
    )
}

/// Check if a step can transition to Completed
fn can_complete(from: &project::WorkflowStep) -> bool {
    use project::WorkflowStep::*;
    // 只有 MilestoneReview（最后一个大阶段选 A 分支）可以进入 Completed
    // Discussion 和 PlanApproval 不能直接跳到 Completed
    matches!(from, MilestoneReview)
}

/// 转换工作流状态（前端调用）
#[tauri::command]
pub(crate) async fn transition_workflow(
    project_name: String,
    target_step: String,
    reason: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;
    let current = proj.workflow_state.current_step.clone();

    // Parse target step
    let to_step =
        parse_step(&target_step).ok_or_else(|| format!("未知的工作流步骤：{}", target_step))?;

    // Validate transition (including fallbacks)
    let valid = is_valid_transition(&current, &to_step)
        || (to_step == project::WorkflowStep::Discussion && can_enter_discussion(&current))
        || (to_step == project::WorkflowStep::Completed && can_complete(&current));

    if !valid {
        return Err(format!(
            "非法工作流转换：从 {:?} 到 {:?} 不被允许。原因：{}",
            current, to_step, reason
        ));
    }

    // Update workflow state
    proj.workflow_state.current_step = to_step.clone();
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    // Update top level phase based on step
    proj.workflow_state.top_level_phase = match &to_step {
        s if *s == project::WorkflowStep::WaitingEntry
            || *s == project::WorkflowStep::ExistingAnalysis
            || *s == project::WorkflowStep::BaselineApproval =>
        {
            project::TopLevelPhase::Before
        }
        s if *s == project::WorkflowStep::Discussion
            || *s == project::WorkflowStep::ThreeChecks
            || *s == project::WorkflowStep::PlanApproval =>
        {
            project::TopLevelPhase::FirstDiscussion
        }
        s if *s == project::WorkflowStep::Completed => project::TopLevelPhase::Completed,
        _ => project::TopLevelPhase::Console,
    };

    crate::save_and_reload_project(&proj)
}

/// 迁移旧项目到新工作流（含执行会话对账与 autopilot sanity）
#[tauri::command]
pub(crate) async fn migrate_project_workflow(
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    // === 0. 执行会话对账（最先执行，防止误恢复） ===
    reconcile_execution_in_migration(&mut proj);

    // === 0.5. autopilot sanity 检查 ===
    reconcile_autopilot_in_migration(&mut proj);

    // Repair rule: PlanApproving + approved plan → Execution
    // Fixes projects stuck in the old "stay at PlanApproving" state after approval.
    if proj.workflow_state.current_step == project::WorkflowStep::PlanApproving {
        // Check if any mid-stage has an approved plan
        let has_approved_plan = proj.milestones.iter().any(|ms| {
            ms.mid_stages
                .iter()
                .any(|mid| mid.plan_approved_at.is_some() && mid.plan_revision > 0)
        });
        // Check if there are execution facts that should be preserved
        let has_execution_facts = proj.milestones.iter().any(|ms| {
            ms.mid_stages.iter().any(|mid| {
                mid.subtasks.iter().any(|st| {
                    matches!(
                        st.status,
                        project::SubtaskStatus::AwaitingConfirmation
                            | project::SubtaskStatus::Passed
                    ) || st.auto_tag.as_ref().is_some_and(|tag| !tag.is_empty())
                })
            })
        });
        if has_approved_plan {
            // Protect existing execution facts: keep Execution, don't go backward
            if has_execution_facts {
                proj.workflow_state.current_step = project::WorkflowStep::Execution;
                proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
            } else if !proj.current_mid_stage_id.is_empty() {
                // Has approved plan with a selected mid-stage → migrate to Execution
                proj.workflow_state.current_step = project::WorkflowStep::Execution;
                proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
            }
            // If no mid-stage selected, keep at current step (will be handled by normal flow)
        }
        proj.workflow_state.data_revision += 1;
        proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
    }

    // === 2026-07-15: 补全 autopilot / Already 宪法字段迁移 ===
    // 确保旧项目加载时这些字段有默认值
    if proj.workflow_state.autopilot_target_milestone_id.is_empty()
        && proj.workflow_state.autopilot_active
        && !proj.milestones.is_empty()
    {
        // 有 autopilot 标记但无目标大阶段 — 找第一个未完成
        if let Some(target) = proj
            .milestones
            .iter()
            .find(|m| m.status != project::MilestoneStatus::Completed)
        {
            proj.workflow_state.autopilot_target_milestone_id = target.id.clone();
            proj.workflow_state.autopilot_state = Some(project::AutopilotState {
                active: true,
                target_milestone_id: target.id.clone(),
                run_status: project::AutopilotRunStatus::Paused,
                last_action: "从旧版本迁移恢复".to_string(),
                last_action_at: chrono::Utc::now().to_rfc3339(),
                error_message: String::new(),
            });
        } else {
            // 所有大阶段已完成 — 关闭 autopilot
            proj.workflow_state.autopilot_active = false;
            proj.workflow_state.autopilot_state = None;
        }
    }

    // Ensure ExistingProjectBaseline has Already constitution fields
    if let Some(ref mut baseline) = proj.existing_baseline {
        if baseline.already_constitution_path.is_empty() && !proj.project_path.is_empty() {
            let already_path =
                std::path::Path::new(&proj.project_path).join("ALREADY_CONSTITUTION.md");
            if already_path.exists() {
                baseline.already_constitution_path = already_path.to_string_lossy().to_string();
                baseline.already_constitution_summary = "从已有文件恢复".to_string();
            }
        }
    }

    // Only migrate if workflow step is still default
    if proj.workflow_state.current_step != project::WorkflowStep::WaitingEntry
        || proj.workflow_state.top_level_phase != project::TopLevelPhase::Before
    {
        return crate::save_and_reload_project(&proj); // Already migrated or repaired above
    }

    // Try to deduce from old fields
    let has_version_plan = !proj.version_plan.is_empty();
    let has_milestones = !proj.milestones.is_empty();
    let is_half_project = proj.existing_baseline.is_some();
    let _has_plan_draft = proj.plan_draft.is_some();
    let all_milestones_done = proj
        .milestones
        .iter()
        .all(|m| m.status == project::MilestoneStatus::Completed);
    let is_quick = proj.mode == project::ProjectMode::Quick;

    // Quick mode: just reset to Before
    if is_quick {
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Before;
        proj.workflow_state.current_step = project::WorkflowStep::Discussion;
        proj.workflow_state.data_revision = 1;
        proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
        return crate::save_and_reload_project(&proj);
    }

    if !has_version_plan && !has_milestones {
        // Fresh project or old idle project
        if is_half_project {
            proj.workflow_state.current_step = project::WorkflowStep::ExistingAnalysis;
            proj.workflow_state.top_level_phase = project::TopLevelPhase::Before;
        } else {
            proj.workflow_state.current_step = project::WorkflowStep::Discussion;
            proj.workflow_state.top_level_phase = project::TopLevelPhase::FirstDiscussion;
        }
        proj.workflow_state.data_revision = 1;
        proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
        return crate::save_and_reload_project(&proj);
    }

    // Has version plan but no milestones — validate approval consistency
    if has_version_plan && !has_milestones {
        let is_approved = proj
            .plan_draft
            .as_ref()
            .map(|d| d.draft_status == project::DraftStatus::Approved || d.approved)
            .unwrap_or(false);

        if is_approved {
            // Verify approval consistency: plan_content matches version_plan,
            // approved_at exists, and draft is genuinely Approved
            let approval_consistent = proj
                .plan_draft
                .as_ref()
                .map(|d| {
                    d.plan_content == proj.version_plan
                        && d.approved_at.is_some()
                        && d.draft_status == project::DraftStatus::Approved
                })
                .unwrap_or(false);

            if approval_consistent {
                proj.workflow_state.current_step = project::WorkflowStep::PlanApproval;
            } else {
                // Inconsistent approval — move draft to history, reset to Discussion
                if let Some(mut inconsistent_draft) = proj.plan_draft.take() {
                    inconsistent_draft.draft_status = project::DraftStatus::Superseded;
                    inconsistent_draft.superseded_at = Some(chrono::Utc::now().to_rfc3339());
                    proj.draft_history.push(inconsistent_draft);
                }
                proj.version_plan.clear();
                proj.preflight_results.clear();
                proj.workflow_state.current_step = project::WorkflowStep::Discussion;
            }
        } else {
            proj.workflow_state.current_step = project::WorkflowStep::Discussion;
        }
        proj.workflow_state.top_level_phase = project::TopLevelPhase::FirstDiscussion;
        proj.workflow_state.data_revision = 1;
        proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
        return crate::save_and_reload_project(&proj);
    }

    // Has milestones — preserve Console state (never force back to decision layer)
    if has_milestones {
        if all_milestones_done {
            proj.workflow_state.current_step = project::WorkflowStep::Completed;
            proj.workflow_state.top_level_phase = project::TopLevelPhase::Completed;
        } else {
            // Keep existing Console state if already in Console, otherwise set to MilestoneSelection
            if proj.workflow_state.top_level_phase != project::TopLevelPhase::Console {
                proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
                proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
            }
            // If already in Console, preserve current step (may be mid-execution)
        }
        proj.workflow_state.data_revision = 1;
        proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
        return crate::save_and_reload_project(&proj);
    }

    // Fallback
    proj.workflow_state.top_level_phase = project::TopLevelPhase::FirstDiscussion;
    proj.workflow_state.current_step = project::WorkflowStep::Discussion;
    proj.workflow_state.data_revision = 1;

    // === Migration: ensure draft lifecycle fields ===
    if let Some(ref mut draft) = proj.plan_draft {
        if draft.draft_id.is_empty() {
            draft.draft_id = uuid::Uuid::new_v4().to_string();
        }
        // Derive draft_status from deprecated approved bool
        if draft.draft_status == project::DraftStatus::Pending && draft.approved {
            draft.draft_status = project::DraftStatus::Approved;
        }
    }

    // Migrate draft_history entries: old Superseded drafts may have expired_at but not superseded_at
    for draft in &mut proj.draft_history {
        if draft.draft_id.is_empty() {
            draft.draft_id = uuid::Uuid::new_v4().to_string();
        }
        // Old approved drafts moved to history with expired_at → migrate to Superseded
        if draft.draft_status == project::DraftStatus::Approved && draft.expired_at.is_some() {
            draft.draft_status = project::DraftStatus::Superseded;
            if draft.superseded_at.is_none() {
                draft.superseded_at = draft.expired_at.clone();
            }
        }
        // Old Pending drafts with expired_at → migrate to Expired
        if draft.draft_status == project::DraftStatus::Pending && draft.expired_at.is_some() {
            draft.draft_status = project::DraftStatus::Expired;
        }
    }

    crate::save_and_reload_project(&proj)
}

fn parse_step(s: &str) -> Option<project::WorkflowStep> {
    use project::WorkflowStep::*;
    match s {
        "WaitingEntry" => Some(WaitingEntry),
        "ExistingAnalysis" => Some(ExistingAnalysis),
        "BaselineApproval" => Some(BaselineApproval),
        "Discussion" => Some(Discussion),
        "ThreeChecks" => Some(ThreeChecks),
        "PlanApproval" => Some(PlanApproval),
        "MilestoneGeneration" => Some(MilestoneGeneration),
        "MilestoneCheck" => Some(MilestoneCheck),
        "MilestoneApproval" => Some(MilestoneApproval),
        "MilestoneSelection" => Some(MilestoneSelection),
        "MidStageGeneration" => Some(MidStageGeneration),
        "MidStageCheck" => Some(MidStageCheck),
        "MidStageApproval" => Some(MidStageApproval),
        "MidStageSelection" => Some(MidStageSelection),
        "PlanGeneration" => Some(PlanGeneration),
        "PlanCheck" => Some(PlanCheck),
        "PlanApproving" => Some(PlanApproving),
        "Execution" => Some(Execution),
        "PauseDecision" => Some(PauseDecision),
        "RollbackPreview" => Some(RollbackPreview),
        "BranchDiscussion" => Some(BranchDiscussion),
        "FuturePlanApproval" => Some(FuturePlanApproval),
        "MilestoneReview" => Some(MilestoneReview),
        "Completed" => Some(Completed),
        _ => None,
    }
}

/// 开始三项检查（专用业务命令，仅在 Discussion 步骤可调用）
#[tauri::command]
pub(crate) async fn start_preflight_check(
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    // 校验当前步骤
    if proj.workflow_state.current_step != project::WorkflowStep::Discussion {
        return Err(format!(
            "当前步骤为 {:?}，只有 Discussion 步骤可以开始三项检查",
            proj.workflow_state.current_step
        ));
    }

    // Half Project: 未批准基线时拒绝
    if proj.entry_kind == project::ProjectEntryKind::HalfProject {
        let baseline_approved = proj
            .existing_baseline
            .as_ref()
            .map(|b| b.approved)
            .unwrap_or(false);
        if !baseline_approved {
            return Err("请先批准已有项目基线（Already Baseline），再进行三项检查。".to_string());
        }
    }

    // 过渡到 ThreeChecks
    proj.workflow_state.current_step = project::WorkflowStep::ThreeChecks;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

/// 返回继续讨论（从 ThreeChecks 或 PlanApproval 返回 Discussion）
///
/// - 从 ThreeChecks 返回：保留未过期检查结果
/// - 从 PlanApproval（待审批草稿）返回：保留草稿和有效检查结果
/// - 从 PlanApproval（过期草稿）返回：草稿已在 chat_with_role 中移入历史，直接返回 Discussion
#[tauri::command]
pub(crate) async fn return_to_discussion(
    project_name: String,
    source_step: String,
    _reason: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    let parsed =
        parse_step(&source_step).ok_or_else(|| format!("未知来源步骤：{}", source_step))?;

    // 验证来源步骤与当前步骤一致
    if proj.workflow_state.current_step != parsed {
        return Err(format!(
            "当前步骤为 {:?}，与来源步骤 {:?} 不一致，请刷新页面",
            proj.workflow_state.current_step, parsed
        ));
    }

    // 允许的来源步骤：ThreeChecks 或 PlanApproval
    match parsed {
        project::WorkflowStep::ThreeChecks => {
            // 保留未过期检查结果，直接转换到 Discussion
        }
        project::WorkflowStep::PlanApproval => {
            // 如果有待审批草稿，保留它（用户可能在 Discussion 中继续审阅）
            // 过期草稿已在 chat_with_role 中移入 draft_history
            if let Some(ref draft) = proj.plan_draft {
                if draft.draft_status == project::DraftStatus::Approved {
                    return Err(
                        "方案已批准，无法直接返回讨论。请使用「重新讨论方案」功能。".to_string()
                    );
                }
                // Pending 草稿保留；Expired/Rejected 草稿保留在 draft_history 中
            }
            // 保留未过期检查结果
        }
        _ => {
            return Err(format!(
                "return_to_discussion 只能从 ThreeChecks 或 PlanApproval 调用，当前来源为 {:?}",
                parsed
            ));
        }
    }

    // 过渡到 Discussion
    proj.workflow_state.current_step = project::WorkflowStep::Discussion;
    proj.workflow_state.top_level_phase = project::TopLevelPhase::FirstDiscussion;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

/// 从 Discussion 恢复方案审批（仅当存在有效待审批草稿、讨论未变化、检查有效时）
#[tauri::command]
pub(crate) async fn resume_plan_approval(project_name: String) -> Result<project::Project, String> {
    let proj = crate::load_project(&project_name)?;

    // 1. 验证当前步骤为 Discussion
    if proj.workflow_state.current_step != project::WorkflowStep::Discussion {
        return Err(format!(
            "当前步骤为 {:?}，只有 Discussion 步骤可以恢复方案审批",
            proj.workflow_state.current_step
        ));
    }

    // 2. 验证存在待审批草稿
    let draft = proj
        .plan_draft
        .as_ref()
        .ok_or("没有可恢复的方案草稿，请重新进行三项检查并生成方案。".to_string())?;

    if draft.draft_status != project::DraftStatus::Pending {
        return Err(format!(
            "草稿状态为 {:?}，只有待审批草稿可以恢复审批。请重新生成方案。",
            draft.draft_status
        ));
    }

    // 3. 验证讨论修订号一致（用户未在返回讨论后发送新需求）
    if draft.generation_revision != proj.discussion_revision {
        return Err(
            "讨论已变化（草稿生成修订号 {} 不等于当前讨论修订号 {}），草稿已过期。请重新进行三项检查并生成方案。".to_string()
                .replace("{}", &draft.generation_revision.to_string())
                .replace("{}", &proj.discussion_revision.to_string())
        );
    }

    // 4. 验证三项检查全部有效（未过期且通过）
    let check_types = [
        "goal_completeness",
        "reality_consistency",
        "task_executability",
    ];
    for ct in &check_types {
        let result = proj
            .preflight_results
            .iter()
            .find(|r| r.check_type == *ct)
            .ok_or_else(|| format!("检查「{}」缺失，请重新进行三项检查。", ct))?;
        if !result.passed {
            return Err(format!(
                "检查「{}」未通过，请返回三项检查页面重新检查。",
                ct
            ));
        }
        if result.stale || result.discussion_revision < proj.discussion_revision {
            return Err(format!("检查「{}」已过期，请重新进行三项检查。", ct));
        }
    }

    // 5. 转换到 PlanApproval
    let mut proj = crate::load_project(&project_name)?;
    proj.workflow_state.current_step = project::WorkflowStep::PlanApproval;
    proj.workflow_state.top_level_phase = project::TopLevelPhase::FirstDiscussion;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

/// 重新讨论已批准方案（将已批准方案移入历史，回到 Discussion）
///
/// 仅在 PlanApproval 步骤且草稿已批准时可调用。
/// 已批准方案保留在 draft_history 中，version_plan 和 preflight_results 被清空。
#[tauri::command]
pub(crate) async fn restart_discussion_from_approved(
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    // 1. 验证当前步骤
    if proj.workflow_state.current_step != project::WorkflowStep::PlanApproval {
        return Err(format!(
            "当前步骤为 {:?}，无法重新讨论已批准方案",
            proj.workflow_state.current_step
        ));
    }

    // 2. 验证草稿已批准
    let draft = proj
        .plan_draft
        .as_ref()
        .ok_or("没有方案草稿。".to_string())?;

    if draft.draft_status != project::DraftStatus::Approved {
        return Err(format!(
            "草稿状态为 {:?}，只有已批准方案可以重新讨论。",
            draft.draft_status
        ));
    }

    // 3. 将已批准草稿移入历史，标记为已被替代
    if let Some(mut approved_draft) = proj.plan_draft.take() {
        approved_draft.draft_status = project::DraftStatus::Superseded;
        approved_draft.superseded_at = Some(chrono::Utc::now().to_rfc3339());
        proj.draft_history.push(approved_draft);
    }

    // 4. 清空 version_plan 和 preflight_results（旧批准凭据失效）
    proj.version_plan.clear();
    proj.preflight_results.clear();

    // 5. 回到 Discussion
    proj.workflow_state.current_step = project::WorkflowStep::Discussion;
    proj.workflow_state.top_level_phase = project::TopLevelPhase::FirstDiscussion;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

/// 重新开始三项检查（清除当前所有检查结果，从第一项开始）
#[tauri::command]
pub(crate) async fn restart_checks(project_name: String) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    if proj.workflow_state.current_step != project::WorkflowStep::ThreeChecks {
        return Err(format!(
            "当前步骤为 {:?}，只有 ThreeChecks 步骤可以重新开始检查",
            proj.workflow_state.current_step
        ));
    }

    // 清除所有检查结果
    proj.preflight_results.clear();
    proj.workflow_state.data_revision += 1;

    crate::save_and_reload_project(&proj)
}

// ===================================================================
// V2 托管层（Managed Flow）：ThreeChecks 后自动推进到大阶段批准
// ===================================================================

/// 激活托管层：从当前步骤开始自动推进到大阶段批准完成
#[tauri::command]
pub(crate) async fn start_managed_flow(project_name: String) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    // 仅允许在 ThreeChecks 或 PlanApproval 步骤启动托管
    match proj.workflow_state.current_step {
        project::WorkflowStep::ThreeChecks
        | project::WorkflowStep::PlanApproval
        | project::WorkflowStep::MilestoneGeneration => {}
        _ => {
            return Err(format!(
                "当前步骤为 {:?}，托管层只能在 ThreeChecks、PlanApproval 或 MilestoneGeneration 启动",
                proj.workflow_state.current_step
            ));
        }
    }

    // 托管层和 autopilot 不得同时激活
    if proj.workflow_state.autopilot_active {
        return Err("自动驾驶已激活，无法同时启动托管层。请先关闭自动驾驶。".to_string());
    }

    let now = chrono::Utc::now().to_rfc3339();
    let current_step_str = format!("{:?}", proj.workflow_state.current_step);

    proj.workflow_state.managed_flow_state = Some(project::ManagedFlowState {
        active: true,
        managed_state: current_step_str,
        managed_target: "MilestoneApproval".to_string(),
        last_action: "托管层已激活，开始自动推进".to_string(),
        last_action_at: now.clone(),
        run_status: project::ManagedRunStatus::Running,
        error_message: String::new(),
    });

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    crate::save_and_reload_project(&proj)
}

/// 托管层下一步顾问：只读判断，返回下一步该执行的原子命令
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ManagedNextStep {
    pub command: String,
    pub args: serde_json::Value,
    pub description: String,
    pub reached_target: bool,
    pub needs_human: bool,
    pub is_error: bool,
    pub error_message: String,
}

#[tauri::command]
pub(crate) async fn managed_next_step(project_name: String) -> Result<ManagedNextStep, String> {
    let proj = crate::load_project(&project_name)?;

    let managed = match proj.workflow_state.managed_flow_state.as_ref() {
        Some(m) => m,
        None => {
            return Ok(ManagedNextStep {
                command: String::new(),
                args: serde_json::json!({}),
                description: "托管层未激活".to_string(),
                reached_target: false,
                needs_human: false,
                is_error: true,
                error_message: "托管层未激活".to_string(),
            });
        }
    };

    if !managed.active {
        return Ok(ManagedNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: "托管层未激活".to_string(),
            reached_target: false,
            needs_human: false,
            is_error: false,
            error_message: String::new(),
        });
    }

    if managed.run_status == project::ManagedRunStatus::Paused {
        return Ok(ManagedNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: "托管层已暂停".to_string(),
            reached_target: false,
            needs_human: false,
            is_error: false,
            error_message: String::new(),
        });
    }

    if managed.run_status == project::ManagedRunStatus::ErrorStopped {
        return Ok(ManagedNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: format!("托管层因错误停止：{}", managed.error_message),
            reached_target: false,
            needs_human: true,
            is_error: true,
            error_message: managed.error_message.clone(),
        });
    }

    let step = &proj.workflow_state.current_step;
    use project::WorkflowStep::*;

    let next = match step {
        // MilestoneApproval: auto-approve if possible, then signal target reached
        MilestoneApproval => {
            let draft_approved = proj
                .milestone_draft
                .as_ref()
                .map(|d| {
                    d.status == project::MilestoneDraftStatus::Approved && d.approved_at.is_some()
                })
                .unwrap_or(false);

            if draft_approved {
                ManagedNextStep {
                    command: String::new(),
                    args: serde_json::json!({}),
                    description: "大阶段已批准，托管层目标达成。可启动自动驾驶继续推进。"
                        .to_string(),
                    reached_target: true,
                    needs_human: false,
                    is_error: false,
                    error_message: String::new(),
                }
            } else {
                // Check if we can auto-approve (check passed, draft exists)
                let can_approve = proj
                    .milestone_draft
                    .as_ref()
                    .map(|d| {
                        d.status != project::MilestoneDraftStatus::CheckFailed
                            && d.check_result.is_some()
                            && !d.candidate_milestones.is_empty()
                    })
                    .unwrap_or(false);

                if can_approve {
                    ManagedNextStep {
                        command: "approve_milestone_draft".to_string(),
                        args: serde_json::json!({ "projectName": project_name }),
                        description: "大阶段检查已通过，自动批准大阶段草稿".to_string(),
                        reached_target: false,
                        needs_human: false,
                        is_error: false,
                        error_message: String::new(),
                    }
                } else {
                    ManagedNextStep {
                        command: String::new(),
                        args: serde_json::json!({}),
                        description: "大阶段草稿尚未通过检查，等待检查完成".to_string(),
                        reached_target: false,
                        needs_human: true,
                        is_error: false,
                        error_message: String::new(),
                    }
                }
            }
        }

        // MilestoneSelection: managed flow target is reached after milestone is approved
        // (MilestoneSelection follows MilestoneApproval; autopilot takes over from here)
        MilestoneSelection => ManagedNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: "大阶段已批准并进入选择阶段，托管层目标达成。可启动自动驾驶继续推进。"
                .to_string(),
            reached_target: true,
            needs_human: false,
            is_error: false,
            error_message: String::new(),
        },

        // ThreeChecks → generate plan draft
        ThreeChecks => {
            // Check if all three checks passed
            let all_passed = [
                "goal_completeness",
                "reality_consistency",
                "task_executability",
            ]
            .iter()
            .all(|ct| {
                proj.preflight_results
                    .iter()
                    .any(|r| r.check_type == *ct && r.passed && !r.stale)
            });

            if all_passed {
                ManagedNextStep {
                    command: "generate_version_plan".to_string(),
                    args: serde_json::json!({
                        "projectName": project_name,
                        "expectedDiscussionRevision": proj.discussion_revision,
                        "expectedDataRevision": proj.workflow_state.data_revision,
                    }),
                    description: "三项检查全部通过，生成方案草稿".to_string(),
                    reached_target: false,
                    needs_human: false,
                    is_error: false,
                    error_message: String::new(),
                }
            } else {
                ManagedNextStep {
                    command: String::new(),
                    args: serde_json::json!({}),
                    description: "等待三项检查全部通过".to_string(),
                    reached_target: false,
                    needs_human: true,
                    is_error: false,
                    error_message: String::new(),
                }
            }
        }

        // PlanApproval: auto-approve if possible, then enter Console
        PlanApproval => {
            let is_approved = proj
                .plan_draft
                .as_ref()
                .map(|d| d.draft_status == project::DraftStatus::Approved)
                .unwrap_or(false);

            if is_approved {
                ManagedNextStep {
                    command: "enter_console".to_string(),
                    args: serde_json::json!({ "projectName": project_name }),
                    description: "方案已批准，进入控制台".to_string(),
                    reached_target: false,
                    needs_human: false,
                    is_error: false,
                    error_message: String::new(),
                }
            } else {
                // Check if we can auto-approve: draft exists, is pending, and can_approve
                let can_auto_approve = proj
                    .plan_draft
                    .as_ref()
                    .map(|d| {
                        d.draft_status == project::DraftStatus::Pending
                            && !d.plan_content.trim().is_empty()
                            && !d.constitution_part1_draft.trim().is_empty()
                            && d.generation_revision == proj.discussion_revision
                    })
                    .unwrap_or(false);

                if can_auto_approve {
                    ManagedNextStep {
                        command: "approve_version_plan".to_string(),
                        args: serde_json::json!({
                            "projectName": project_name,
                            "draftId": proj.plan_draft.as_ref().map(|d| d.draft_id.clone()).unwrap_or_default(),
                            "generationRevision": proj.plan_draft.as_ref().map(|d| d.generation_revision).unwrap_or(0),
                        }),
                        description: "托管层自动批准方案草稿".to_string(),
                        reached_target: false,
                        needs_human: false,
                        is_error: false,
                        error_message: String::new(),
                    }
                } else {
                    ManagedNextStep {
                        command: String::new(),
                        args: serde_json::json!({}),
                        description: "等待方案草稿生成（需先生成方案草稿方可自动批准）".to_string(),
                        reached_target: false,
                        needs_human: true,
                        is_error: false,
                        error_message: String::new(),
                    }
                }
            }
        }

        // MilestoneGeneration → generate milestones (this is the entry step after enter_console)
        MilestoneGeneration => ManagedNextStep {
            command: "generate_milestone_draft".to_string(),
            args: serde_json::json!({ "projectName": project_name }),
            description: "生成大阶段草稿".to_string(),
            reached_target: false,
            needs_human: false,
            is_error: false,
            error_message: String::new(),
        },

        // MilestoneCheck → check draft
        MilestoneCheck => ManagedNextStep {
            command: "check_milestone_draft".to_string(),
            args: serde_json::json!({ "projectName": project_name }),
            description: "检查大阶段草稿".to_string(),
            reached_target: false,
            needs_human: false,
            is_error: false,
            error_message: String::new(),
        },

        // Steps where managed flow cannot help
        Discussion | BranchDiscussion | PauseDecision | Execution | MidStageGeneration
        | MidStageCheck | MidStageApproval | MidStageSelection | PlanGeneration | PlanCheck
        | PlanApproving => ManagedNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: format!("当前步骤 {:?} 不在托管范围内", step),
            reached_target: false,
            needs_human: true,
            is_error: false,
            error_message: format!("{:?} 不在托管层范围内", step),
        },

        _ => ManagedNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: format!("托管层未覆盖步骤：{:?}", step),
            reached_target: false,
            needs_human: true,
            is_error: true,
            error_message: format!("托管层不支持从 {:?} 自动推进", step),
        },
    };

    Ok(next)
}

/// 暂停托管层
#[tauri::command]
pub(crate) async fn pause_managed_flow(project_name: String) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    let managed = proj
        .workflow_state
        .managed_flow_state
        .as_ref()
        .ok_or("托管层未激活。".to_string())?;

    if !managed.active {
        return Err("托管层未激活。".to_string());
    }

    let now = chrono::Utc::now().to_rfc3339();
    if let Some(ref mut m) = proj.workflow_state.managed_flow_state {
        m.run_status = project::ManagedRunStatus::Paused;
        m.last_action = "托管层已暂停".to_string();
        m.last_action_at = now.clone();
    }

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    crate::save_and_reload_project(&proj)
}

/// 恢复托管层
#[tauri::command]
pub(crate) async fn resume_managed_flow(project_name: String) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    let managed = proj
        .workflow_state
        .managed_flow_state
        .as_ref()
        .ok_or("托管层未激活。".to_string())?;

    if !managed.active {
        return Err("托管层未激活。".to_string());
    }

    if managed.run_status != project::ManagedRunStatus::Paused {
        return Err(format!(
            "托管层当前状态为 {:?}，只有暂停状态可以恢复",
            managed.run_status
        ));
    }

    // Prevent simultaneous automated systems
    if proj.workflow_state.autopilot_active {
        return Err("自动驾驶已激活，无法恢复托管层。请先关闭自动驾驶。".to_string());
    }

    let now = chrono::Utc::now().to_rfc3339();
    if let Some(ref mut m) = proj.workflow_state.managed_flow_state {
        m.run_status = project::ManagedRunStatus::Running;
        m.last_action = "托管层已恢复".to_string();
        m.last_action_at = now.clone();
    }

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    crate::save_and_reload_project(&proj)
}

/// 停止托管层（交接给 autopilot 或回到手动模式）
///
/// 清除 managed_flow_state 并保持在当前步骤，由用户手动操作。
/// 如果当前在托管范围内但未完成的步骤，保留当前步骤不变。
/// 如果当前在 Console 阶段且有大阶段可选，过渡到对应的手动选择步骤。
#[tauri::command]
pub(crate) async fn stop_managed_flow(project_name: String) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    if proj.workflow_state.managed_flow_state.is_none() {
        return Err("托管层未激活。".to_string());
    }

    let now = chrono::Utc::now().to_rfc3339();

    // Determine the appropriate manual step based on current workflow state
    use project::WorkflowStep::*;
    let current_step = &proj.workflow_state.current_step;

    // If we're at a milestone step and there are milestones, transition to
    // the appropriate manual selection/generation step
    let new_step = match current_step {
        MilestoneApproval | MilestoneSelection => {
            // Check if milestone draft exists and is approved
            let draft_approved = proj
                .milestone_draft
                .as_ref()
                .map(|d| d.status == project::MilestoneDraftStatus::Approved)
                .unwrap_or(false);
            if draft_approved {
                MilestoneSelection
            } else {
                // Go back to milestone generation so user can manually approve
                MilestoneGeneration
            }
        }
        // For PlanApproval / MilestoneGeneration / MilestoneCheck: keep current step
        // (user was in the middle of these — let them continue manually)
        _ => current_step.clone(),
    };

    proj.workflow_state.current_step = new_step;
    proj.workflow_state.managed_flow_state = None;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    crate::save_and_reload_project(&proj)
}

/// 自动驾驶持久化错误信息最大长度，防止项目文件异常膨胀
const AUTOPILOT_ERROR_MESSAGE_MAX_LENGTH: usize = 2048;

// ===================================================================
// V1 大阶段自动驾驶：可见、可监督、可中断
// ===================================================================

/// 激活自动驾驶：自动选择第一个未完成大阶段并开始推进
#[tauri::command]
pub(crate) async fn toggle_autopilot(
    project_name: String,
    active: bool,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    // Only allow toggling within Console phase
    if proj.workflow_state.top_level_phase != project::TopLevelPhase::Console {
        return Err("自动驾驶仅可在 Console 阶段使用。".to_string());
    }

    // Prevent simultaneous autopilot and managed flow
    if active
        && proj
            .workflow_state
            .managed_flow_state
            .as_ref()
            .map(|m| m.active)
            .unwrap_or(false)
    {
        return Err("托管层正在运行，无法激活自动驾驶。请先停止托管层。".to_string());
    }

    if active {
        // Auto-select first non-Completed milestone
        let target = proj
            .milestones
            .iter()
            .find(|m| m.status != project::MilestoneStatus::Completed)
            .ok_or("所有大阶段已完成，无法激活自动驾驶。".to_string())?;

        let now = chrono::Utc::now().to_rfc3339();
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_target_milestone_id = target.id.clone();
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: target.id.clone(),
            run_status: project::AutopilotRunStatus::Running,
            last_action: format!("自动驾驶已激活，目标大阶段：{}", target.title),
            last_action_at: now,
            error_message: String::new(),
        });
    } else {
        proj.workflow_state.autopilot_active = false;
        proj.workflow_state.autopilot_target_milestone_id = String::new();
        proj.workflow_state.autopilot_state = None;
    }

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

/// 自动驾驶暂停：执行中则 In Stop 回退，否则仅置暂停
#[tauri::command]
pub(crate) async fn autopilot_pause(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    if !proj.workflow_state.autopilot_active {
        return Err("自动驾驶未激活。".to_string());
    }

    let is_executing = proj.workflow_state.current_step == project::WorkflowStep::Execution
        && proj
            .execution_session
            .as_ref()
            .map(|s| s.status == "executing")
            .unwrap_or(false);

    let now = chrono::Utc::now().to_rfc3339();

    if is_executing {
        // In Stop: delegate to unified perform_in_stop
        crate::pipeline::perform_in_stop(&state, &mut proj).await?;

        // Set autopilot to paused
        if let Some(ref mut ap) = proj.workflow_state.autopilot_state {
            ap.run_status = project::AutopilotRunStatus::Paused;
            ap.last_action = "执行中暂停（In Stop），已回退到最近完成小阶段".to_string();
            ap.last_action_at = now.clone();
        }
    } else {
        // Not executing: just set autopilot to paused
        if let Some(ref mut ap) = proj.workflow_state.autopilot_state {
            ap.run_status = project::AutopilotRunStatus::Paused;
            ap.last_action = "自动驾驶已暂停".to_string();
            ap.last_action_at = now.clone();
        }
    }

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    crate::save_and_reload_project(&proj)
}

/// 持久化自动驾驶步骤状态：写入 last_action、last_action_at、run_status 和 error_message
fn autopilot_persist_step_state(
    proj: &mut project::Project,
    action: &str,
    status: project::AutopilotRunStatus,
    error_msg: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let truncated_error = if error_msg.len() > AUTOPILOT_ERROR_MESSAGE_MAX_LENGTH {
        format!("{}...", &error_msg[..AUTOPILOT_ERROR_MESSAGE_MAX_LENGTH])
    } else {
        error_msg.to_string()
    };

    if let Some(ref mut ap) = proj.workflow_state.autopilot_state {
        ap.last_action = action.to_string();
        ap.last_action_at = now.clone();
        ap.run_status = status;
        ap.error_message = truncated_error;
    }

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;
    Ok(())
}

/// 自动驾驶标记错误：持久化 ErrorStopped 和可读错误，再同步项目
#[tauri::command]
pub(crate) async fn autopilot_mark_error(
    project_name: String,
    action_description: String,
    error_detail: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    if !proj.workflow_state.autopilot_active {
        return Err("自动驾驶未激活。".to_string());
    }

    autopilot_persist_step_state(
        &mut proj,
        &action_description,
        project::AutopilotRunStatus::ErrorStopped,
        &error_detail,
    )?;

    crate::save_and_reload_project(&proj)
}

/// 自动驾驶恢复：验证恢复条件后设置 Running
#[tauri::command]
pub(crate) async fn autopilot_resume(project_name: String) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    if !proj.workflow_state.autopilot_active {
        return Err("自动驾驶未激活。".to_string());
    }

    // Verify recovery conditions
    let can_resume = match proj.workflow_state.autopilot_state.as_ref() {
        Some(ap) => match ap.run_status {
            project::AutopilotRunStatus::Paused => true,
            project::AutopilotRunStatus::ErrorStopped => {
                // ErrorStopped can only resume if there's no unresolved quality failure
                // Check: if execution step with awaiting subtask that failed quality, block
                if proj.workflow_state.current_step == project::WorkflowStep::Execution {
                    if let Some(ref session) = proj.execution_session {
                        if session.status == "awaiting_confirmation" {
                            return Err(
                                "存在待确认的任务，请先处理质量结果后再恢复自动驾驶。".to_string()
                            );
                        }
                    }
                }
                true
            }
            project::AutopilotRunStatus::WaitingMilestoneReview => {
                return Err("等待大阶段审阅中，请先完成 A/B/C 决策后再恢复。".to_string());
            }
            project::AutopilotRunStatus::Running => {
                return Err("自动驾驶已在运行中。".to_string());
            }
        },
        None => return Err("自动驾驶状态不存在，请先激活自动驾驶。".to_string()),
    };

    if !can_resume {
        return Err("当前状态不允许恢复自动驾驶。".to_string());
    }

    let now = chrono::Utc::now().to_rfc3339();
    if let Some(ref mut ap) = proj.workflow_state.autopilot_state {
        ap.run_status = project::AutopilotRunStatus::Running;
        ap.last_action = "自动驾驶已恢复".to_string();
        ap.last_action_at = now.clone();
        ap.error_message = String::new();
    }

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    crate::save_and_reload_project(&proj)
}

/// 自动驾驶下一步顾问：只读判断，返回下一步该执行什么原子命令
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutopilotNextStep {
    /// 原子命令名（前端直接 invoke）
    pub command: String,
    /// 命令参数（JSON 对象）
    pub args: serde_json::Value,
    /// 人类可读说明
    pub description: String,
    /// 是否到达大阶段边界（需人工 A/B/C）
    pub at_milestone_boundary: bool,
    /// 是否出错
    pub is_error: bool,
    /// 错误/暂停说明
    pub error_message: String,
    /// 命令返回类别（前端按类别分流处理）
    pub result_kind: project::AutopilotCommandResultKind,
}

#[tauri::command]
pub(crate) async fn autopilot_next_step(project_name: String) -> Result<AutopilotNextStep, String> {
    let proj = crate::load_project(&project_name)?;

    if !proj.workflow_state.autopilot_active {
        return Ok(AutopilotNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: "自动驾驶未激活".to_string(),
            at_milestone_boundary: false,
            is_error: true,
            error_message: "自动驾驶未激活".to_string(),
            result_kind: project::AutopilotCommandResultKind::NoResult,
        });
    }

    // Check if autopilot is paused or errored
    if let Some(ref ap) = proj.workflow_state.autopilot_state {
        match ap.run_status {
            project::AutopilotRunStatus::Paused => {
                return Ok(AutopilotNextStep {
                    command: String::new(),
                    args: serde_json::json!({}),
                    description: "自动驾驶已暂停，等待手动操作".to_string(),
                    at_milestone_boundary: false,
                    is_error: false,
                    error_message: String::new(),
                    result_kind: project::AutopilotCommandResultKind::NoResult,
                });
            }
            project::AutopilotRunStatus::ErrorStopped => {
                return Ok(AutopilotNextStep {
                    command: String::new(),
                    args: serde_json::json!({}),
                    description: format!("自动驾驶因错误停止：{}", ap.error_message),
                    at_milestone_boundary: false,
                    is_error: true,
                    error_message: ap.error_message.clone(),
                    result_kind: project::AutopilotCommandResultKind::NoResult,
                });
            }
            project::AutopilotRunStatus::WaitingMilestoneReview => {
                return Ok(AutopilotNextStep {
                    command: String::new(),
                    args: serde_json::json!({}),
                    description: "到达大阶段边界，等待人工 A/B/C 决策".to_string(),
                    at_milestone_boundary: true,
                    is_error: false,
                    error_message: String::new(),
                    result_kind: project::AutopilotCommandResultKind::NoResult,
                });
            }
            _ => {} // Running — continue
        }
    }

    let step = &proj.workflow_state.current_step;
    let target_ms_id = &proj.workflow_state.autopilot_target_milestone_id;

    // Ensure target milestone exists
    let target_ms = proj.milestones.iter().find(|m| m.id == *target_ms_id);
    if target_ms.is_none() {
        return Ok(AutopilotNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: "目标大阶段不存在".to_string(),
            at_milestone_boundary: false,
            is_error: true,
            error_message: "目标大阶段不存在".to_string(),
            result_kind: project::AutopilotCommandResultKind::NoResult,
        });
    }
    let target_ms = match target_ms {
        Some(ms) => ms,
        None => {
            return Ok(AutopilotNextStep {
                command: String::new(),
                args: serde_json::json!({}),
                description: "目标大阶段不存在".to_string(),
                at_milestone_boundary: false,
                is_error: true,
                error_message: "目标大阶段不存在".to_string(),
                result_kind: project::AutopilotCommandResultKind::NoResult,
            })
        }
    };

    use project::WorkflowStep::*;
    let next = match step {
        // If at MilestoneReview, stop for human A/B/C
        MilestoneReview => {
            return Ok(AutopilotNextStep {
                command: String::new(),
                args: serde_json::json!({}),
                description: "到达大阶段边界，等待人工 A/B/C 决策".to_string(),
                at_milestone_boundary: true,
                is_error: false,
                error_message: String::new(),
                result_kind: project::AutopilotCommandResultKind::NoResult,
            });
        }

        // Select target milestone if not selected
        _ if proj.current_milestone_id.is_empty() || proj.current_milestone_id != *target_ms_id => {
            AutopilotNextStep {
                command: "select_milestone".to_string(),
                args: serde_json::json!({
                    "projectName": project_name,
                    "milestoneId": target_ms.id,
                }),
                description: format!("选择大阶段：{}", target_ms.title),
                at_milestone_boundary: false,
                is_error: false,
                error_message: String::new(),
                result_kind: project::AutopilotCommandResultKind::ProjectState,
            }
        }

        // Milestone selected → transition to mid-stage generation
        MilestoneSelection => AutopilotNextStep {
            command: "transition_workflow".to_string(),
            args: serde_json::json!({
                "projectName": project_name,
                "targetStep": "MidStageGeneration",
                "reason": "autopilot: 进入中阶段生成",
            }),
            description: "进入中阶段规划流程".to_string(),
            at_milestone_boundary: false,
            is_error: false,
            error_message: String::new(),
            result_kind: project::AutopilotCommandResultKind::ProjectState,
        },

        // Enter mid-stage generation → generate draft (auto-transitions to MidStageCheck)
        MidStageGeneration => AutopilotNextStep {
            command: "generate_mid_stage_draft".to_string(),
            args: serde_json::json!({ "projectName": project_name }),
            description: "生成中阶段草稿".to_string(),
            at_milestone_boundary: false,
            is_error: false,
            error_message: String::new(),
            result_kind: project::AutopilotCommandResultKind::ProjectState,
        },

        // Mid-stage draft generated → check (auto-transitions to MidStageApproval)
        MidStageCheck => AutopilotNextStep {
            command: "check_mid_stage_draft".to_string(),
            args: serde_json::json!({ "projectName": project_name }),
            description: "检查中阶段草稿".to_string(),
            at_milestone_boundary: false,
            is_error: false,
            error_message: String::new(),
            result_kind: project::AutopilotCommandResultKind::ProjectState,
        },

        // Mid-stage check passed → approve (auto-transitions to MidStageSelection)
        MidStageApproval => AutopilotNextStep {
            command: "approve_mid_stage_draft".to_string(),
            args: serde_json::json!({ "projectName": project_name }),
            description: "批准中阶段草稿".to_string(),
            at_milestone_boundary: false,
            is_error: false,
            error_message: String::new(),
            result_kind: project::AutopilotCommandResultKind::ProjectState,
        },

        // Mid-stages approved and at selection — select first non-completed mid-stage,
        // then transition to plan generation
        MidStageSelection
            if !proj.current_mid_stage_id.is_empty()
                && target_ms
                    .mid_stages
                    .iter()
                    .find(|m| m.id == proj.current_mid_stage_id)
                    .map(|m| !m.subtasks.is_empty() && m.plan_approved_at.is_some())
                    .unwrap_or(false) =>
        {
            // Mid-stage already selected AND has plan approved → execute
            AutopilotNextStep {
                command: "transition_workflow".to_string(),
                args: serde_json::json!({
                    "projectName": project_name,
                    "targetStep": "Execution",
                    "reason": "autopilot: 进入执行阶段",
                }),
                description: "进入执行阶段".to_string(),
                at_milestone_boundary: false,
                is_error: false,
                error_message: String::new(),
                result_kind: project::AutopilotCommandResultKind::ProjectState,
            }
        }

        MidStageSelection if !proj.current_mid_stage_id.is_empty() => {
            // Mid-stage selected → transition to plan generation
            AutopilotNextStep {
                command: "transition_workflow".to_string(),
                args: serde_json::json!({
                    "projectName": project_name,
                    "targetStep": "PlanGeneration",
                    "reason": "autopilot: 进入执行计划生成",
                }),
                description: "进入执行计划生成".to_string(),
                at_milestone_boundary: false,
                is_error: false,
                error_message: String::new(),
                result_kind: project::AutopilotCommandResultKind::ProjectState,
            }
        }

        MidStageSelection => {
            // No mid-stage selected yet → select first non-completed
            let next_mid = target_ms
                .mid_stages
                .iter()
                .find(|m| m.status != project::MidStageStatus::Completed);
            match next_mid {
                Some(mid) => AutopilotNextStep {
                    command: "select_mid_stage".to_string(),
                    args: serde_json::json!({
                        "projectName": project_name,
                        "midStageId": mid.id,
                    }),
                    description: format!("选择中阶段：{}", mid.title),
                    at_milestone_boundary: false,
                    is_error: false,
                    error_message: String::new(),
                    result_kind: project::AutopilotCommandResultKind::ProjectState,
                },
                None => AutopilotNextStep {
                    command: String::new(),
                    args: serde_json::json!({}),
                    description: "没有未完成的中阶段".to_string(),
                    at_milestone_boundary: false,
                    is_error: true,
                    error_message: "没有未完成的中阶段".to_string(),
                    result_kind: project::AutopilotCommandResultKind::NoResult,
                },
            }
        }

        // Plan generation → generate execution plan (auto-transitions to PlanCheck)
        PlanGeneration => AutopilotNextStep {
            command: "generate_execution_plan".to_string(),
            args: serde_json::json!({ "projectName": project_name }),
            description: "生成执行计划".to_string(),
            at_milestone_boundary: false,
            is_error: false,
            error_message: String::new(),
            result_kind: project::AutopilotCommandResultKind::ProjectState,
        },

        // Plan generated → check (auto-transitions to PlanApproving)
        PlanCheck => AutopilotNextStep {
            command: "check_stage_plan".to_string(),
            args: serde_json::json!({ "projectName": project_name }),
            description: "检查执行计划".to_string(),
            at_milestone_boundary: false,
            is_error: false,
            error_message: String::new(),
            result_kind: project::AutopilotCommandResultKind::ProjectState,
        },

        // Plan check passed → approve (auto-transitions to Execution)
        PlanApproving => AutopilotNextStep {
            command: "approve_stage_plan".to_string(),
            args: serde_json::json!({ "projectName": project_name }),
            description: "批准执行计划，进入执行阶段".to_string(),
            at_milestone_boundary: false,
            is_error: false,
            error_message: String::new(),
            result_kind: project::AutopilotCommandResultKind::ProjectState,
        },

        // In execution — execute next pending or confirm awaiting
        // 只围绕当前中阶段判断，不跨中阶段串扰
        Execution => {
            // 先确定当前中阶段
            let current_mid = if !proj.current_mid_stage_id.is_empty() {
                target_ms
                    .mid_stages
                    .iter()
                    .find(|m| m.id == proj.current_mid_stage_id)
            } else {
                None
            };

            // 当前中阶段不存在或未设置 → 尝试选择第一个未完成中阶段
            let current_mid = match current_mid {
                Some(mid) => mid,
                None => {
                    let next_mid = target_ms
                        .mid_stages
                        .iter()
                        .find(|m| m.status != project::MidStageStatus::Completed);
                    match next_mid {
                        Some(mid) => {
                            return Ok(AutopilotNextStep {
                                command: "select_mid_stage".to_string(),
                                args: serde_json::json!({
                                    "projectName": project_name,
                                    "midStageId": mid.id,
                                }),
                                description: format!("选择中阶段：{}", mid.title),
                                at_milestone_boundary: false,
                                is_error: false,
                                error_message: String::new(),
                                result_kind: project::AutopilotCommandResultKind::ProjectState,
                            });
                        }
                        None => {
                            // 所有中阶段已完成 → 进入大阶段审阅
                            return Ok(AutopilotNextStep {
                                command: "transition_workflow".to_string(),
                                args: serde_json::json!({
                                    "projectName": project_name,
                                    "targetStep": "MilestoneReview",
                                    "reason": "autopilot: 所有中阶段完成，进入大阶段审阅",
                                }),
                                description: "所有中阶段已完成，进入大阶段审阅".to_string(),
                                at_milestone_boundary: true,
                                is_error: false,
                                error_message: String::new(),
                                result_kind: project::AutopilotCommandResultKind::ProjectState,
                            });
                        }
                    }
                }
            };

            // 只在当前中阶段内判断 subtasks 状态
            let has_awaiting = current_mid
                .subtasks
                .iter()
                .any(|st| st.status == project::SubtaskStatus::AwaitingConfirmation);
            let has_pending = current_mid
                .subtasks
                .iter()
                .any(|st| st.status == project::SubtaskStatus::Pending);
            let has_rejected = current_mid
                .subtasks
                .iter()
                .any(|st| st.status == project::SubtaskStatus::Rejected);

            // 当前中阶段有 Rejected 且无待确认/待执行 → 需人工处理
            if has_rejected && !has_awaiting && !has_pending {
                AutopilotNextStep {
                    command: String::new(),
                    args: serde_json::json!({}),
                    description: format!(
                        "中阶段「{}」存在已驳回的小阶段，需要人工决定是否重试或重新生成执行计划",
                        current_mid.title
                    ),
                    at_milestone_boundary: false,
                    is_error: true,
                    error_message: format!(
                        "中阶段「{}」中存在 Rejected 小阶段，请人工处理。",
                        current_mid.title
                    ),
                    result_kind: project::AutopilotCommandResultKind::NoResult,
                }
            } else if has_awaiting {
                // 质量门禁预检：执行结果、测试结果、证据完整性
                match crate::pipeline::validate_subtask_quality_gate(&proj) {
                    Ok(()) => AutopilotNextStep {
                        command: "confirm_subtask_result".to_string(),
                        args: serde_json::json!({ "projectName": project_name }),
                        description: "自动确认小阶段执行结果".to_string(),
                        at_milestone_boundary: false,
                        is_error: false,
                        error_message: String::new(),
                        result_kind: project::AutopilotCommandResultKind::ProjectState,
                    },
                    Err(gate_reason) => {
                        // 质量失败：持久化 ErrorStopped
                        let mut save_proj = proj.clone();
                        let now = chrono::Utc::now().to_rfc3339();
                        if let Some(ref mut ap) = save_proj.workflow_state.autopilot_state {
                            ap.run_status = project::AutopilotRunStatus::ErrorStopped;
                            ap.last_action = format!("质量门禁阻断：{}", gate_reason);
                            ap.last_action_at = now;
                            ap.error_message = gate_reason.clone();
                        }
                        save_proj.workflow_state.data_revision += 1;
                        let _ = crate::save_project(&save_proj);
                        AutopilotNextStep {
                            command: String::new(),
                            args: serde_json::json!({}),
                            description: format!("质量门禁阻断：{}", gate_reason),
                            at_milestone_boundary: false,
                            is_error: true,
                            error_message: gate_reason,
                            result_kind: project::AutopilotCommandResultKind::NoResult,
                        }
                    }
                }
            } else if has_pending {
                AutopilotNextStep {
                    command: "execute_current_subtask".to_string(),
                    args: serde_json::json!({ "projectName": project_name }),
                    description: "执行下一个待处理小阶段".to_string(),
                    at_milestone_boundary: false,
                    is_error: false,
                    error_message: String::new(),
                    result_kind: project::AutopilotCommandResultKind::PipelineState,
                }
            } else {
                // 当前中阶段没有 pending/awaiting/rejected → 已完成
                // 显式切换到下一个中阶段或进入大阶段审阅
                let next_mid = target_ms
                    .mid_stages
                    .iter()
                    .filter(|m| m.id != current_mid.id)
                    .find(|m| m.status != project::MidStageStatus::Completed);

                match next_mid {
                    Some(mid) => AutopilotNextStep {
                        command: "select_mid_stage".to_string(),
                        args: serde_json::json!({
                            "projectName": project_name,
                            "midStageId": mid.id,
                        }),
                        description: format!(
                            "中阶段「{}」已完成，切换到下一中阶段：{}",
                            current_mid.title, mid.title
                        ),
                        at_milestone_boundary: false,
                        is_error: false,
                        error_message: String::new(),
                        result_kind: project::AutopilotCommandResultKind::ProjectState,
                    },
                    None => AutopilotNextStep {
                        command: "transition_workflow".to_string(),
                        args: serde_json::json!({
                            "projectName": project_name,
                            "targetStep": "MilestoneReview",
                            "reason": "autopilot: 所有中阶段完成，进入大阶段审阅",
                        }),
                        description: "所有中阶段已完成，进入大阶段审阅".to_string(),
                        at_milestone_boundary: true,
                        is_error: false,
                        error_message: String::new(),
                        result_kind: project::AutopilotCommandResultKind::ProjectState,
                    },
                }
            }
        }

        // States where autopilot can't help
        Discussion | BranchDiscussion | PauseDecision | RollbackPreview | FuturePlanApproval
        | ThreeChecks | PlanApproval => AutopilotNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: format!("当前步骤 {:?} 需要人工介入，无法自动推进", step),
            at_milestone_boundary: false,
            is_error: true,
            error_message: format!("{:?} 步骤需要人工介入", step),
            result_kind: project::AutopilotCommandResultKind::NoResult,
        },

        // Milestone generation/check/approval — user should handle these before autopilot
        MilestoneGeneration | MilestoneCheck | MilestoneApproval => AutopilotNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: "请先手动完成大阶段生成、检查和批准，然后激活自动驾驶。".to_string(),
            at_milestone_boundary: false,
            is_error: true,
            error_message: "请先手动完成大阶段生成、检查和批准。".to_string(),
            result_kind: project::AutopilotCommandResultKind::NoResult,
        },

        _ => AutopilotNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: format!("未处理的步骤：{:?}", step),
            at_milestone_boundary: false,
            is_error: true,
            error_message: format!("自动驾驶不支持从 {:?} 自动推进", step),
            result_kind: project::AutopilotCommandResultKind::NoResult,
        },
    };

    Ok(next)
}

// ===================================================================
// 迁移时执行会话与 autopilot 对账
// ===================================================================

/// 在 migrate_project_workflow 中执行会话对账
///
/// 迁移时没有 AppState，因此无法获取内存 PipelineState。
/// 此时传递 None 意味着：
/// - "executing" 会话 → StartupRecoverable（保留会话，不判丢失）
/// - "awaiting_confirmation" 会话 → AwaitingConfirmation（保留）
/// - 无效/冲突会话 → 照常清理
///
/// 真正的 SessionLost 判断只发生在 reconcile_on_startup 中，
/// 那时 PipelineState 已可用，可以准确区分"进程已死"和"刚启动尚未恢复"。
fn reconcile_execution_in_migration(proj: &mut crate::project::Project) {
    let reconciliation = { crate::pipeline::reconcile_execution_state(proj, None) };

    // 只在真正不可恢复时才清理：无效会话、数据冲突。
    // StartupRecoverable / Executing / AwaitingConfirmation 均保留不动。
    if matches!(
        reconciliation,
        crate::pipeline::ExecutionReconciliation::SessionInvalid
            | crate::pipeline::ExecutionReconciliation::DataConflict
    ) {
        crate::pipeline::apply_execution_reconciliation(proj, &reconciliation);
    }
}

/// 在 migrate_project_workflow 中 autopilot sanity 检查
fn reconcile_autopilot_in_migration(proj: &mut crate::project::Project) {
    if !proj.workflow_state.autopilot_active {
        if proj.workflow_state.autopilot_state.is_some() {
            proj.workflow_state.autopilot_state = None;
            proj.workflow_state.autopilot_target_milestone_id = String::new();
            proj.workflow_state.data_revision += 1;
        }
        return;
    }

    // Verify autopilot state exists
    if proj.workflow_state.autopilot_state.is_none() {
        proj.workflow_state.autopilot_active = false;
        proj.workflow_state.autopilot_target_milestone_id = String::new();
        proj.workflow_state.data_revision += 1;
        return;
    }

    // Verify target milestone still exists
    let target_id = &proj.workflow_state.autopilot_target_milestone_id;
    if !target_id.is_empty() {
        let target_exists = proj.milestones.iter().any(|m| m.id == *target_id);
        if !target_exists {
            // Target milestone gone — find new target or deactivate
            if let Some(next) = proj
                .milestones
                .iter()
                .find(|m| m.status != crate::project::MilestoneStatus::Completed)
            {
                proj.workflow_state.autopilot_target_milestone_id = next.id.clone();
                if let Some(ref mut ap) = proj.workflow_state.autopilot_state {
                    ap.target_milestone_id = next.id.clone();
                    ap.last_action = "目标大阶段已自动修复（原目标不存在）".to_string();
                    ap.last_action_at = chrono::Utc::now().to_rfc3339();
                }
            } else {
                // All milestones complete
                proj.workflow_state.autopilot_active = false;
                proj.workflow_state.autopilot_target_milestone_id = String::new();
                proj.workflow_state.autopilot_state = None;
            }
            proj.workflow_state.data_revision += 1;
        }
    }

    // Check autopilot not active outside Console
    if proj.workflow_state.top_level_phase != crate::project::TopLevelPhase::Console {
        proj.workflow_state.autopilot_active = false;
        proj.workflow_state.autopilot_target_milestone_id = String::new();
        proj.workflow_state.autopilot_state = None;
        proj.workflow_state.data_revision += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autopilot_inactive_returns_error_step() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let mut proj = project::Project::new("test-ap-inactive");
            proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
            crate::save_project(&proj).unwrap();
            autopilot_next_step("test-ap-inactive".to_string()).await
        });
        assert!(result.is_ok());
        let step = result.unwrap();
        assert!(step.is_error);
        assert_eq!(
            step.result_kind,
            project::AutopilotCommandResultKind::NoResult
        );
        let _ = std::fs::remove_file(crate::project_data_path("test-ap-inactive").unwrap());
    }

    #[test]
    fn toggle_autopilot_requires_console_phase() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let proj = project::Project::new("test-ap-phase");
            crate::save_project(&proj).unwrap();
            toggle_autopilot("test-ap-phase".to_string(), true).await
        });
        assert!(result.is_err());
        let _ = std::fs::remove_file(crate::project_data_path("test-ap-phase").unwrap());
    }
}
