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
        | (RollbackPreview, PlanGeneration)  // confirmed rollback
    )
}

/// Allow returning to Discussion from non-execution steps
fn can_enter_discussion(from: &project::WorkflowStep) -> bool {
    use project::WorkflowStep::*;
    // PlanApproval → Discussion 必须通过 reject_version_plan 命令（会清除 preflight_results）
    matches!(from, Discussion | ThreeChecks | MilestoneSelection
        | MidStageCheck | PlanCheck | RollbackPreview | BranchDiscussion
        | MilestoneReview | FuturePlanApproval)
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
    let to_step = parse_step(&target_step)
        .ok_or_else(|| format!("未知的工作流步骤：{}", target_step))?;

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
            || *s == project::WorkflowStep::BaselineApproval => project::TopLevelPhase::Before,
        s if *s == project::WorkflowStep::Discussion
            || *s == project::WorkflowStep::ThreeChecks
            || *s == project::WorkflowStep::PlanApproval => project::TopLevelPhase::FirstDiscussion,
        s if *s == project::WorkflowStep::Completed => project::TopLevelPhase::Completed,
        _ => project::TopLevelPhase::Console,
    };

    crate::save_project(&proj)?;
    Ok(proj)
}

/// 迁移旧项目到新工作流
#[tauri::command]
pub(crate) async fn migrate_project_workflow(
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    // Repair rule: PlanApproving + approved plan → Execution
    // Fixes projects stuck in the old "stay at PlanApproving" state after approval.
    if proj.workflow_state.current_step == project::WorkflowStep::PlanApproving {
        // Check if any mid-stage has an approved plan
        let has_approved_plan = proj.milestones.iter().any(|ms| {
            ms.mid_stages.iter().any(|mid| {
                mid.plan_approved_at.is_some() && mid.plan_revision > 0
            })
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
        if let Some(target) = proj.milestones.iter()
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
        if baseline.already_constitution_path.is_empty()
            && !proj.project_path.is_empty()
        {
            let already_path = std::path::Path::new(&proj.project_path)
                .join("ALREADY_CONSTITUTION.md");
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
        crate::save_project(&proj)?;
        return Ok(proj); // Already migrated or repaired above
    }

    // Try to deduce from old fields
    let has_version_plan = !proj.version_plan.is_empty();
    let has_milestones = !proj.milestones.is_empty();
    let is_half_project = proj.existing_baseline.is_some();
    let _has_plan_draft = proj.plan_draft.is_some();
    let all_milestones_done = proj.milestones.iter().all(|m| m.status == project::MilestoneStatus::Completed);
    let is_quick = proj.mode == project::ProjectMode::Quick;

    // Quick mode: just reset to Before
    if is_quick {
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Before;
        proj.workflow_state.current_step = project::WorkflowStep::Discussion;
        proj.workflow_state.data_revision = 1;
        proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
        crate::save_project(&proj)?;
        return Ok(proj);
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
        crate::save_project(&proj)?;
        return Ok(proj);
    }

    // Has version plan but no milestones — validate approval consistency
    if has_version_plan && !has_milestones {
        let is_approved = proj.plan_draft.as_ref().map(|d| {
            d.draft_status == project::DraftStatus::Approved || d.approved
        }).unwrap_or(false);

        if is_approved {
            // Verify approval consistency: plan_content matches version_plan,
            // approved_at exists, and draft is genuinely Approved
            let approval_consistent = proj.plan_draft.as_ref().map(|d| {
                d.plan_content == proj.version_plan
                    && d.approved_at.is_some()
                    && d.draft_status == project::DraftStatus::Approved
            }).unwrap_or(false);

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
        crate::save_project(&proj)?;
        return Ok(proj);
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
        crate::save_project(&proj)?;
        return Ok(proj);
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
        if draft.draft_status == project::DraftStatus::Approved
            && draft.expired_at.is_some()
        {
            draft.draft_status = project::DraftStatus::Superseded;
            if draft.superseded_at.is_none() {
                draft.superseded_at = draft.expired_at.clone();
            }
        }
        // Old Pending drafts with expired_at → migrate to Expired
        if draft.draft_status == project::DraftStatus::Pending
            && draft.expired_at.is_some()
        {
            draft.draft_status = project::DraftStatus::Expired;
        }
    }

    crate::save_project(&proj)?;
    Ok(proj)
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
            return Err(
                "请先批准已有项目基线（Already Baseline），再进行三项检查。".to_string()
            );
        }
    }

    // 过渡到 ThreeChecks
    proj.workflow_state.current_step = project::WorkflowStep::ThreeChecks;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_project(&proj)?;
    Ok(proj)
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

    let parsed = parse_step(&source_step)
        .ok_or_else(|| format!("未知来源步骤：{}", source_step))?;

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

    crate::save_project(&proj)?;
    Ok(proj)
}

/// 从 Discussion 恢复方案审批（仅当存在有效待审批草稿、讨论未变化、检查有效时）
#[tauri::command]
pub(crate) async fn resume_plan_approval(
    project_name: String,
) -> Result<project::Project, String> {
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
    let check_types = ["goal_completeness", "reality_consistency", "task_executability"];
    for ct in &check_types {
        let result = proj.preflight_results.iter().find(|r| r.check_type == *ct)
            .ok_or_else(|| format!("检查「{}」缺失，请重新进行三项检查。", ct))?;
        if !result.passed {
            return Err(format!("检查「{}」未通过，请返回三项检查页面重新检查。", ct));
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

    crate::save_project(&proj)?;
    Ok(proj)
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

    crate::save_project(&proj)?;
    Ok(proj)
}

/// 重新开始三项检查（清除当前所有检查结果，从第一项开始）
#[tauri::command]
pub(crate) async fn restart_checks(
    project_name: String,
) -> Result<project::Project, String> {
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

    crate::save_project(&proj)?;
    Ok(proj)
}

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

    if active {
        // Auto-select first non-Completed milestone
        let target = proj.milestones.iter()
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
        && proj.execution_session.as_ref().map(|s| s.status == "executing").unwrap_or(false);

    let now = chrono::Utc::now().to_rfc3339();

    if is_executing {
        // In Stop: kill child process, rollback to last completed subtask
        let child_pid = {
            let mut guard = state.pipeline_state.lock().await;
            if let Some(s) = guard.as_mut() {
                s.status = crate::pipeline::PipelineStatus::Failed;
                let pid = s.child_pid.take();
                s.child_pid = None;
                pid
            } else {
                None
            }
        };

        if let Some(pid) = child_pid {
            #[cfg(unix)]
            { let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).output(); }
            #[cfg(not(unix))]
            { let _ = std::process::Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output(); }
        }

        // Find last passed subtask for rollback reference
        let last_passed_tag = crate::pipeline::find_last_passed_subtask(&proj)
            .and_then(|st| st.auto_tag.clone());

        // Revert code to last stable tag if available
        if let Some(ref tag) = last_passed_tag {
            let _ = crate::git_ops::git_stash_and_reset_to_tag(&proj.project_path, tag);
        }

        // Clear execution session
        proj.execution_session = None;

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
}

#[tauri::command]
pub(crate) async fn autopilot_next_step(
    project_name: String,
) -> Result<AutopilotNextStep, String> {
    let proj = crate::load_project(&project_name)?;

    if !proj.workflow_state.autopilot_active {
        return Ok(AutopilotNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: "自动驾驶未激活".to_string(),
            at_milestone_boundary: false,
            is_error: true,
            error_message: "自动驾驶未激活".to_string(),
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
        });
    }
    let target_ms = match target_ms {
        Some(ms) => ms,
        None => return Ok(AutopilotNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: "目标大阶段不存在".to_string(),
            at_milestone_boundary: false,
            is_error: true,
            error_message: "目标大阶段不存在".to_string(),
        }),
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
            });
        }

        // Select target milestone if not selected
        _ if proj.current_milestone_id.is_empty()
            || proj.current_milestone_id != *target_ms_id => {
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
            }
        }

        // Milestone selected → transition to mid-stage generation
        MilestoneSelection => {
            AutopilotNextStep {
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
            }
        }

        // Enter mid-stage generation → generate draft (auto-transitions to MidStageCheck)
        MidStageGeneration => {
            AutopilotNextStep {
                command: "generate_mid_stage_draft".to_string(),
                args: serde_json::json!({ "projectName": project_name }),
                description: "生成中阶段草稿".to_string(),
                at_milestone_boundary: false,
                is_error: false,
                error_message: String::new(),
            }
        }

        // Mid-stage draft generated → check (auto-transitions to MidStageApproval)
        MidStageCheck => {
            AutopilotNextStep {
                command: "check_mid_stage_draft".to_string(),
                args: serde_json::json!({ "projectName": project_name }),
                description: "检查中阶段草稿".to_string(),
                at_milestone_boundary: false,
                is_error: false,
                error_message: String::new(),
            }
        }

        // Mid-stage check passed → approve (auto-transitions to MidStageSelection)
        MidStageApproval => {
            AutopilotNextStep {
                command: "approve_mid_stage_draft".to_string(),
                args: serde_json::json!({ "projectName": project_name }),
                description: "批准中阶段草稿".to_string(),
                at_milestone_boundary: false,
                is_error: false,
                error_message: String::new(),
            }
        }

        // Mid-stages approved and at selection — select first non-completed mid-stage,
        // then transition to plan generation
        MidStageSelection if !proj.current_mid_stage_id.is_empty()
            && target_ms.mid_stages.iter()
                .find(|m| m.id == proj.current_mid_stage_id)
                .map(|m| !m.subtasks.is_empty() && m.plan_approved_at.is_some())
                .unwrap_or(false) => {
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
            }
        }

        MidStageSelection => {
            // No mid-stage selected yet → select first non-completed
            let next_mid = target_ms.mid_stages.iter()
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
                },
                None => AutopilotNextStep {
                    command: String::new(),
                    args: serde_json::json!({}),
                    description: "没有未完成的中阶段".to_string(),
                    at_milestone_boundary: false,
                    is_error: true,
                    error_message: "没有未完成的中阶段".to_string(),
                },
            }
        }

        // Plan generation → generate execution plan (auto-transitions to PlanCheck)
        PlanGeneration => {
            AutopilotNextStep {
                command: "generate_execution_plan".to_string(),
                args: serde_json::json!({ "projectName": project_name }),
                description: "生成执行计划".to_string(),
                at_milestone_boundary: false,
                is_error: false,
                error_message: String::new(),
            }
        }

        // Plan generated → check (auto-transitions to PlanApproving)
        PlanCheck => {
            AutopilotNextStep {
                command: "check_stage_plan".to_string(),
                args: serde_json::json!({ "projectName": project_name }),
                description: "检查执行计划".to_string(),
                at_milestone_boundary: false,
                is_error: false,
                error_message: String::new(),
            }
        }

        // Plan check passed → approve (auto-transitions to Execution)
        PlanApproving => {
            AutopilotNextStep {
                command: "approve_stage_plan".to_string(),
                args: serde_json::json!({ "projectName": project_name }),
                description: "批准执行计划，进入执行阶段".to_string(),
                at_milestone_boundary: false,
                is_error: false,
                error_message: String::new(),
            }
        }

        // In execution — execute next pending or confirm awaiting
        Execution => {
            let has_awaiting = target_ms.mid_stages.iter()
                .any(|mid| mid.subtasks.iter()
                    .any(|st| st.status == project::SubtaskStatus::AwaitingConfirmation));
            let has_pending = target_ms.mid_stages.iter()
                .any(|mid| mid.subtasks.iter()
                    .any(|st| st.status == project::SubtaskStatus::Pending));

            if has_awaiting {
                AutopilotNextStep {
                    command: "confirm_subtask_result".to_string(),
                    args: serde_json::json!({ "projectName": project_name }),
                    description: "自动确认小阶段执行结果".to_string(),
                    at_milestone_boundary: false,
                    is_error: false,
                    error_message: String::new(),
                }
            } else if has_pending {
                AutopilotNextStep {
                    command: "execute_current_subtask".to_string(),
                    args: serde_json::json!({ "projectName": project_name }),
                    description: "执行下一个待处理小阶段".to_string(),
                    at_milestone_boundary: false,
                    is_error: false,
                    error_message: String::new(),
                }
            } else {
                AutopilotNextStep {
                    command: String::new(),
                    args: serde_json::json!({}),
                    description: "等待状态推进（中阶段可能已完成）".to_string(),
                    at_milestone_boundary: false,
                    is_error: true,
                    error_message: "Execution 步骤中未找到待处理或待确认的小阶段".to_string(),
                }
            }
        }

        // States where autopilot can't help
        Discussion | BranchDiscussion | PauseDecision | RollbackPreview
        | FuturePlanApproval | ThreeChecks | PlanApproval => {
            AutopilotNextStep {
                command: String::new(),
                args: serde_json::json!({}),
                description: format!("当前步骤 {:?} 需要人工介入，无法自动推进", step),
                at_milestone_boundary: false,
                is_error: true,
                error_message: format!("{:?} 步骤需要人工介入", step),
            }
        }

        // Milestone generation/check/approval — user should handle these before autopilot
        MilestoneGeneration | MilestoneCheck | MilestoneApproval => {
            AutopilotNextStep {
                command: "transition_workflow".to_string(),
                args: serde_json::json!({
                    "projectName": project_name,
                    "targetStep": "MilestoneSelection",
                    "reason": "autopilot: 跳过大阶段审批（已由用户完成）",
                }),
                description: "大阶段审批流程应由用户完成".to_string(),
                at_milestone_boundary: false,
                is_error: true,
                error_message: "请先手动完成大阶段生成、检查和批准".to_string(),
            }
        }

        _ => AutopilotNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: format!("未处理的步骤：{:?}", step),
            at_milestone_boundary: false,
            is_error: true,
            error_message: format!("自动驾驶不支持从 {:?} 自动推进", step),
        },
    };

    Ok(next)
}
