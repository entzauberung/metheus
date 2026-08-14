// src-tauri/src/commands/workflow.rs — 集中工作流状态转换
use crate::project;

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
        | (ThreeChecks, ProjectPlanGeneration) // all checks passed
        | (ProjectPlanGeneration, Discussion)  // requirements changed before generation
        | (ProjectPlanGeneration, PlanApproval) // draft generated successfully
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
        | (MilestoneSelection, PlanGeneration) // Quick topology skips mid stages
        | (MilestoneSelection, MilestoneReview) // existing mid stages are already complete
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
            | ProjectPlanGeneration
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

fn has_valid_preflight_checks(proj: &project::Project) -> bool {
    crate::workload_policy::current_profile(proj).is_ok()
        && [
            "goal_completeness",
            "reality_consistency",
            "task_executability",
        ]
        .iter()
        .all(|check_type| {
            proj.preflight_results.iter().any(|result| {
                result.check_type == *check_type
                    && result.passed
                    && !result.stale
                    && result.discussion_revision == proj.discussion_revision
            })
        })
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
    if to_step == project::WorkflowStep::MilestoneReview {
        return crate::commands::milestone::enter_milestone_review(project_name).await;
    }
    if to_step == project::WorkflowStep::PlanApproval && proj.plan_draft.is_none() {
        return Err("没有可审批的项目方案草稿，无法进入 PlanApproval。".to_string());
    }
    if current == project::WorkflowStep::PlanCheck
        && to_step == project::WorkflowStep::PlanApproving
    {
        let scope = crate::plan_scope::PlanScope::resolve(&proj)?;
        let check = scope
            .plan_check_result_mut(&mut proj)
            .ok_or_else(|| "执行计划尚未检查，无法进入批准阶段。".to_string())?;
        *check = crate::autopilot_policy::normalize_plan_check_result(check.clone());
        if !check.passed {
            return Err("执行计划仍有硬阻断，无法进入批准阶段。".to_string());
        }
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
            || *s == project::WorkflowStep::ProjectPlanGeneration
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
    let persisted_revision = proj.workflow_state.data_revision;

    // === 0. 执行会话与控制锁对账（最先执行，防止误恢复） ===
    // 活跃的本地或其他进程租约会让该入口保持只读；陈旧租约则与 Git/执行事实一起收口。
    crate::pipeline::reconcile_loaded_project_under_pipeline_lock(&mut proj, None);

    // === 0.5. autopilot sanity 检查 ===
    reconcile_autopilot_in_migration(&mut proj);

    // === 0.75. 旧执行计划契约迁移 ===
    // 无执行事实的旧计划退回检查；已有执行事实只停止自动驾驶，不改写历史。
    reconcile_plan_contract_in_migration(&mut proj);

    // === 0.8. workflow closure migration ===
    let closure_changed = reconcile_workflow_closure_state(&mut proj)?;
    if closure_changed {
        proj.workflow_state.data_revision = persisted_revision.saturating_add(1);
    }

    // Repair rule: PlanApproving + approved plan → Execution
    // Fixes projects stuck in the old "stay at PlanApproving" state after approval.
    if proj.workflow_state.current_step == project::WorkflowStep::PlanApproving {
        if let Ok(scope) = crate::plan_scope::PlanScope::resolve(&proj) {
            let has_approved_plan =
                scope.plan_approved_at(&proj).is_some() && scope.plan_revision(&proj) > 0;
            if has_approved_plan {
                proj.workflow_state.current_step = project::WorkflowStep::Execution;
                proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
            }
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
                recovery_action: project::AutopilotRecoveryAction::None,
                ..Default::default()
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
                proj.workload_profile = None;
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
        "ProjectPlanGeneration" => Some(ProjectPlanGeneration),
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

/// 返回继续讨论（从检查、项目方案生成或方案审批返回 Discussion）
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

    // 允许的来源步骤：ThreeChecks、ProjectPlanGeneration 或 PlanApproval
    match parsed {
        project::WorkflowStep::ThreeChecks | project::WorkflowStep::ProjectPlanGeneration => {
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
                "return_to_discussion 只能从 ThreeChecks、ProjectPlanGeneration 或 PlanApproval 调用，当前来源为 {:?}",
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

    let profile = crate::workload_policy::current_profile(&proj)?;
    if draft.workload_profile_fingerprint != profile.fingerprint {
        return Err("方案草稿绑定的工作负载画像已变化，请重新进行三项检查并生成方案。".to_string());
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
        if result.stale || result.discussion_revision != proj.discussion_revision {
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
    proj.workload_profile = None;

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
    proj.workload_profile = None;
    proj.workflow_state.data_revision += 1;

    crate::save_and_reload_project(&proj)
}

// ===================================================================
// V2 托管层（Managed Flow）：ThreeChecks 后自动推进到大阶段批准完成
// ===================================================================

fn pending_plan_draft_is_valid(proj: &project::Project) -> bool {
    proj.plan_draft.as_ref().is_some_and(|draft| {
        draft.draft_status == project::DraftStatus::Pending
            && !draft.plan_content.trim().is_empty()
            && !draft.constitution_part1_draft.trim().is_empty()
            && draft.generation_revision == proj.discussion_revision
            && proj.workload_profile.as_ref().is_some_and(|profile| {
                profile.discussion_revision == proj.discussion_revision
                    && !profile.fingerprint.is_empty()
                    && draft.workload_profile_fingerprint == profile.fingerprint
            })
    }) && has_valid_preflight_checks(proj)
}

fn reconcile_managed_plan_state(proj: &mut project::Project) -> bool {
    let original_step = proj.workflow_state.current_step.clone();

    if original_step == project::WorkflowStep::ThreeChecks && has_valid_preflight_checks(proj) {
        proj.workflow_state.current_step = project::WorkflowStep::ProjectPlanGeneration;
    }

    if original_step == project::WorkflowStep::PlanApproval && proj.plan_draft.is_none() {
        proj.workflow_state.current_step = project::WorkflowStep::ProjectPlanGeneration;
    }

    if matches!(
        original_step,
        project::WorkflowStep::ProjectPlanGeneration | project::WorkflowStep::PlanApproval
    ) {
        let has_approved = proj.plan_draft.as_ref().is_some_and(|draft| {
            draft.draft_status == project::DraftStatus::Approved
                && !draft.plan_content.trim().is_empty()
        });
        if pending_plan_draft_is_valid(proj) || has_approved {
            proj.workflow_state.current_step = project::WorkflowStep::PlanApproval;
        } else if proj.plan_draft.is_some() {
            if let Some(mut invalid) = proj.plan_draft.take() {
                if invalid.draft_status == project::DraftStatus::Pending {
                    invalid.draft_status = project::DraftStatus::Expired;
                    invalid.expired_at = Some(chrono::Utc::now().to_rfc3339());
                }
                proj.draft_history.push(invalid);
            }
            proj.workflow_state.current_step = if has_valid_preflight_checks(proj) {
                project::WorkflowStep::ProjectPlanGeneration
            } else {
                project::WorkflowStep::ThreeChecks
            };
        }
    }

    if proj.workflow_state.current_step == original_step {
        return false;
    }
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(managed) = proj.workflow_state.managed_flow_state.as_mut() {
        managed.managed_state = format!("{:?}", proj.workflow_state.current_step);
        managed.last_action = "托管层已按方案草稿事实修复工作流状态".to_string();
        managed.last_action_at = now.clone();
    }
    proj.workflow_state.data_revision = proj.workflow_state.data_revision.saturating_add(1);
    proj.workflow_state.last_transition_at = now;
    true
}

/// 激活托管层：从当前步骤开始自动推进到大阶段批准完成
#[tauri::command]
pub(crate) async fn start_managed_flow(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    let updated = start_managed_flow_state(project_name.clone()).await?;
    state.managed_runtime.start(project_name).await?;
    Ok(updated)
}

pub(crate) async fn start_managed_flow_state(
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;
    reconcile_managed_plan_state(&mut proj);

    // 仅允许在检查完成后的生成/审批步骤或大阶段生成步骤启动托管
    match proj.workflow_state.current_step {
        project::WorkflowStep::ThreeChecks
        | project::WorkflowStep::ProjectPlanGeneration
        | project::WorkflowStep::PlanApproval
        | project::WorkflowStep::MilestoneGeneration => {}
        _ => {
            return Err(format!(
                "当前步骤为 {:?}，托管层只能在 ThreeChecks、ProjectPlanGeneration、PlanApproval 或 MilestoneGeneration 启动",
                proj.workflow_state.current_step
            ));
        }
    }

    // 托管层和 autopilot 不得同时激活
    if proj.workflow_state.autopilot_active {
        return Err("自动驾驶已激活，无法同时启动托管层。请先关闭自动驾驶。".to_string());
    }

    if let Some(existing) = proj.workflow_state.managed_flow_state.as_mut() {
        match (existing.active, &existing.run_status) {
            (true, project::ManagedRunStatus::Running) => {
                return Err("托管层已在运行，不能重复启动。".to_string());
            }
            (true, project::ManagedRunStatus::Paused)
            | (true, project::ManagedRunStatus::WaitingHuman) => {
                return Err("托管层当前已暂停或等待人工，请使用恢复动作。".to_string());
            }
            (true, project::ManagedRunStatus::ErrorStopped) => {
                crate::managed_runtime::assign_new_job_identity(
                    existing,
                    "托管层已显式重启，后端开始新的作业代次",
                );
                existing.run_status = project::ManagedRunStatus::Running;
            }
            (false, _) => {}
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    let current_step_str = format!("{:?}", proj.workflow_state.current_step);

    if let Some(existing) = proj.workflow_state.managed_flow_state.as_mut() {
        if !existing.active {
            existing.active = true;
            existing.run_status = project::ManagedRunStatus::Running;
            existing.managed_state = current_step_str;
            existing.managed_target = "MilestoneSelection".to_string();
            crate::managed_runtime::assign_new_job_identity(
                existing,
                "托管层已激活，后端开始自动推进",
            );
        }
    } else {
        let mut managed_state = project::ManagedFlowState {
            active: true,
            managed_state: current_step_str,
            managed_target: "MilestoneSelection".to_string(),
            last_action: "托管层已激活，开始自动推进".to_string(),
            last_action_at: now.clone(),
            run_status: project::ManagedRunStatus::Running,
            error_message: String::new(),
            ..Default::default()
        };
        crate::managed_runtime::assign_new_job_identity(
            &mut managed_state,
            "托管层已激活，后端开始自动推进",
        );
        proj.workflow_state.managed_flow_state = Some(managed_state);
    }

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
    let mut proj = crate::load_project(&project_name)?;
    if reconcile_managed_plan_state(&mut proj) {
        crate::save_project(&proj)?;
    }

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

    if managed.run_status == project::ManagedRunStatus::WaitingHuman {
        return Ok(ManagedNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: managed.last_action.clone(),
            reached_target: false,
            needs_human: true,
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
                        d.status == project::MilestoneDraftStatus::CheckPassed
                            && d.check_result
                                .as_deref()
                                .is_some_and(|result| !result.trim().is_empty())
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

        // ThreeChecks remains a real human boundary until every check passes.
        ThreeChecks => {
            // Check if all three checks passed
            let all_passed = has_valid_preflight_checks(&proj);

            if all_passed {
                ManagedNextStep {
                    command: String::new(),
                    args: serde_json::json!({}),
                    description: "检查已通过，请同步项目状态进入方案生成".to_string(),
                    reached_target: false,
                    needs_human: true,
                    is_error: true,
                    error_message: "检查状态尚未对账到 ProjectPlanGeneration".to_string(),
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

        ProjectPlanGeneration => ManagedNextStep {
            command: "generate_version_plan".to_string(),
            args: serde_json::json!({
                "projectName": project_name,
                "expectedDiscussionRevision": proj.discussion_revision,
                "expectedDataRevision": proj.workflow_state.data_revision,
            }),
            description: "生成项目方案草稿".to_string(),
            reached_target: false,
            needs_human: false,
            is_error: false,
            error_message: String::new(),
        },

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
                let can_auto_approve = pending_plan_draft_is_valid(&proj);

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
                        description: "方案审批状态缺少有效草稿，请同步项目状态".to_string(),
                        reached_target: false,
                        needs_human: true,
                        is_error: true,
                        error_message: "PlanApproval 缺少有效草稿".to_string(),
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

        MilestoneCheck => {
            let waiting = |description: String| ManagedNextStep {
                command: String::new(),
                args: serde_json::json!({}),
                description,
                reached_target: false,
                needs_human: true,
                is_error: false,
                error_message: String::new(),
            };
            let Some(draft) = proj.milestone_draft.as_ref() else {
                return Ok(waiting("大阶段检查步骤缺少草稿，等待人工同步".to_string()));
            };
            if draft.draft_kind != project::MilestoneDraftKind::Normal {
                return Ok(waiting(
                    "大阶段检查步骤包含未来规划草稿，等待人工同步".to_string(),
                ));
            }
            if draft.candidate_milestones.is_empty() {
                return Ok(waiting("候选大阶段为空，等待人工处理".to_string()));
            }

            match draft.status {
                project::MilestoneDraftStatus::Pending => ManagedNextStep {
                    command: "check_milestone_draft".to_string(),
                    args: serde_json::json!({ "projectName": project_name }),
                    description: "检查大阶段草稿".to_string(),
                    reached_target: false,
                    needs_human: false,
                    is_error: false,
                    error_message: String::new(),
                },
                project::MilestoneDraftStatus::CheckFailed => {
                    let feedback = draft
                        .check_result
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty());
                    let Some(feedback) = feedback else {
                        return Ok(waiting(
                            "大阶段检查未通过但缺少反馈，等待人工处理".to_string(),
                        ));
                    };
                    if draft.regeneration_count
                        >= crate::autopilot_policy::MAX_PLANNING_REGENERATIONS
                    {
                        return Ok(waiting(
                            "大阶段草稿自动重生成已达到两次上限，等待人工处理".to_string(),
                        ));
                    }
                    let repeated_issue = draft.regeneration_count > 0
                        && draft
                            .last_regeneration_reason
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .is_some_and(|previous| {
                                crate::autopilot_policy::text_fingerprint(previous)
                                    == crate::autopilot_policy::text_fingerprint(feedback)
                            });
                    if repeated_issue {
                        return Ok(waiting(
                            "大阶段草稿连续出现相同检查问题，等待人工处理".to_string(),
                        ));
                    }
                    ManagedNextStep {
                        command: "regenerate_milestone_draft".to_string(),
                        args: serde_json::json!({
                            "projectName": project_name,
                            "currentDraftId": draft.draft_id,
                            "expectedDataRevision": proj.workflow_state.data_revision,
                            "feedback": feedback,
                            "source": "check_failed",
                        }),
                        description: "按检查结果重新生成大阶段草稿".to_string(),
                        reached_target: false,
                        needs_human: false,
                        is_error: false,
                        error_message: String::new(),
                    }
                }
                project::MilestoneDraftStatus::CheckPassed
                | project::MilestoneDraftStatus::Approved => {
                    waiting("大阶段草稿状态与检查步骤不一致，等待人工同步".to_string())
                }
            }
        }

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

/// 将托管层置为等待人工，并保留具体阻断原因。
#[tauri::command]
pub(crate) async fn wait_managed_flow_for_human(
    project_name: String,
    reason: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;
    let managed = proj
        .workflow_state
        .managed_flow_state
        .as_ref()
        .ok_or("托管层未激活。".to_string())?;
    if !managed.active {
        return Err("托管层未激活。".to_string());
    }

    let reason = reason.trim();
    let reason = if reason.is_empty() {
        "托管流程等待人工处理"
    } else {
        reason
    };
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(ref mut managed) = proj.workflow_state.managed_flow_state {
        managed.run_status = project::ManagedRunStatus::WaitingHuman;
        managed.last_action = reason.to_string();
        managed.last_action_at = now.clone();
    }
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;
    crate::save_and_reload_project(&proj)
}

/// 恢复托管层
#[tauri::command]
pub(crate) async fn resume_managed_flow(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    let updated = resume_managed_flow_state(project_name.clone()).await?;
    state.managed_runtime.start(project_name).await?;
    Ok(updated)
}

pub(crate) async fn resume_managed_flow_state(
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    let managed = proj
        .workflow_state
        .managed_flow_state
        .as_ref()
        .ok_or("托管层未激活。".to_string())?;

    if !managed.active {
        return Err("托管层未激活。".to_string());
    }

    if !matches!(
        managed.run_status,
        project::ManagedRunStatus::Paused | project::ManagedRunStatus::WaitingHuman
    ) {
        return Err(format!(
            "托管层当前状态为 {:?}，只有暂停或等待人工状态可以恢复",
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
        crate::managed_runtime::assign_new_job_identity(m, "托管层已恢复，后端继续推进");
    }

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    crate::save_and_reload_project(&proj)
}

/// 停止托管层（交接给 autopilot 或回到手动模式）
///
/// 清除 managed_flow_state 并保持当前人工步骤；仅修复已经批准却仍停在批准页的旧状态。
#[tauri::command]
pub(crate) async fn stop_managed_flow(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    state.managed_runtime.stop(&project_name).await;
    stop_managed_flow_state(project_name).await
}

pub(crate) async fn stop_managed_flow_state(
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    if proj.workflow_state.managed_flow_state.is_none() {
        return Err("托管层未激活。".to_string());
    }

    let now = chrono::Utc::now().to_rfc3339();

    // 停止托管只释放控制权。仅修复“已经批准却仍停在批准页”的历史状态；
    // 正常的 CheckPassed 待批准状态必须原地保留给用户手动处理。
    if proj.workflow_state.current_step == project::WorkflowStep::MilestoneApproval
        && proj
            .milestone_draft
            .as_ref()
            .is_some_and(|draft| draft.status == project::MilestoneDraftStatus::Approved)
    {
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
    }
    proj.workflow_state.managed_flow_state = None;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    crate::save_and_reload_project(&proj)
}

pub(crate) fn reconcile_managed_milestone_project(proj: &mut project::Project) -> bool {
    let mut changed = false;
    if proj.workflow_state.current_step == project::WorkflowStep::MilestoneApproval {
        if let Some(draft) = proj.milestone_draft.as_mut() {
            let has_check_result = draft
                .check_result
                .as_deref()
                .is_some_and(|result| !result.trim().is_empty());
            let legacy_status = matches!(
                draft.status,
                project::MilestoneDraftStatus::Pending | project::MilestoneDraftStatus::CheckFailed
            );
            if legacy_status && has_check_result && !draft.candidate_milestones.is_empty() {
                draft.status = project::MilestoneDraftStatus::CheckPassed;
                changed = true;
            }
        }
    }

    if let Some(managed) = proj.workflow_state.managed_flow_state.as_mut() {
        if managed.managed_target != "MilestoneSelection" {
            managed.managed_target = "MilestoneSelection".to_string();
            changed = true;
        }
    }
    changed
}

/// 修复旧版本留下的大阶段检查/托管矛盾状态；不自动恢复暂停的托管流程。
#[tauri::command]
pub(crate) async fn reconcile_managed_milestone_state(
    project_name: String,
) -> Result<project::Project, String> {
    reconcile_managed_milestone_state_with_pipeline(&project_name, None)
}

pub(crate) fn reconcile_managed_milestone_state_with_pipeline(
    project_name: &str,
    pipeline_status: Option<&crate::pipeline::PipelineState>,
) -> Result<project::Project, String> {
    crate::mutate_project_for_control(project_name, |proj| {
        let mut changed =
            crate::pipeline::reconcile_loaded_project_under_pipeline_lock(proj, pipeline_status);
        let managed_changed = reconcile_managed_milestone_project(proj);
        changed |= managed_changed;
        if managed_changed {
            proj.workflow_state.data_revision = proj.workflow_state.data_revision.saturating_add(1);
            proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
        }
        Ok((proj.clone(), changed))
    })
}

/// 自动驾驶持久化错误信息最大长度，防止项目文件异常膨胀
const AUTOPILOT_ERROR_MESSAGE_MAX_LENGTH: usize = 2048;

fn autopilot_can_activate_from(step: &project::WorkflowStep) -> bool {
    matches!(
        step,
        project::WorkflowStep::MilestoneSelection
            | project::WorkflowStep::MidStageGeneration
            | project::WorkflowStep::MidStageCheck
            | project::WorkflowStep::MidStageApproval
            | project::WorkflowStep::MidStageSelection
            | project::WorkflowStep::PlanGeneration
            | project::WorkflowStep::PlanCheck
            | project::WorkflowStep::PlanApproving
            | project::WorkflowStep::Execution
    )
}

fn require_fresh_autopilot_profile(proj: &project::Project) -> Result<(), String> {
    crate::workload_policy::current_profile(proj)
        .map(|_| ())
        .map_err(|error| {
            if error.contains("重新完成目标完整性检查") {
                error
            } else {
                format!("{} 请重新完成目标完整性检查。", error)
            }
        })
}

fn truncate_autopilot_error(error_msg: &str) -> String {
    let mut chars = error_msg.chars();
    let truncated: String = chars
        .by_ref()
        .take(AUTOPILOT_ERROR_MESSAGE_MAX_LENGTH)
        .collect();
    if chars.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

// ===================================================================
// V1 大阶段自动驾驶：可见、可监督、可中断
// ===================================================================

async fn toggle_autopilot_state(
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
        if !autopilot_can_activate_from(&proj.workflow_state.current_step) {
            return Err(format!(
                "当前步骤为 {:?}，请先完成人工大阶段生成、检查和批准，并进入大阶段选择后再激活自动驾驶。",
                proj.workflow_state.current_step
            ));
        }
        require_fresh_autopilot_profile(&proj)?;

        // 优先沿用用户已选择且未完成的大阶段，否则选择第一个未完成阶段。
        let selected_target = proj.milestones.iter().find(|m| {
            m.id == proj.current_milestone_id && m.status != project::MilestoneStatus::Completed
        });
        let target = selected_target
            .or_else(|| {
                proj.milestones
                    .iter()
                    .find(|m| m.status != project::MilestoneStatus::Completed)
            })
            .ok_or("所有大阶段已完成，无法激活自动驾驶。".to_string())?;
        let target_id = target.id.clone();
        let target_title = target.title.clone();

        let now = chrono::Utc::now().to_rfc3339();
        let next_generation = proj
            .workflow_state
            .autopilot_state
            .as_ref()
            .map(|state| state.job_generation.saturating_add(1))
            .unwrap_or(1);
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_target_milestone_id = target_id.clone();
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: target_id,
            run_status: project::AutopilotRunStatus::Running,
            last_action: format!("自动驾驶已激活，目标大阶段：{}", target_title),
            last_action_at: now,
            error_message: String::new(),
            recovery_action: project::AutopilotRecoveryAction::None,
            job_id: uuid::Uuid::new_v4().to_string(),
            job_generation: next_generation,
            job_owner: project::AutopilotJobOwner::BackendRuntime,
            heartbeat_at: chrono::Utc::now().to_rfc3339(),
            ..Default::default()
        });
    } else {
        if let Some(state) = proj.workflow_state.autopilot_state.as_mut() {
            state.job_generation = state.job_generation.saturating_add(1);
            state.job_owner = project::AutopilotJobOwner::None;
            state.current_action_id.clear();
            state.current_action_kind.clear();
            state.action_started_at.clear();
            state.next_retry_at = None;
        }
        proj.workflow_state.autopilot_active = false;
        proj.workflow_state.autopilot_target_milestone_id = String::new();
        proj.workflow_state.autopilot_state = None;
    }

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

/// 激活自动驾驶：持久化作业身份后，由 Rust 后端开始推进。
#[tauri::command]
pub(crate) async fn toggle_autopilot(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
    active: bool,
) -> Result<project::Project, String> {
    let project = toggle_autopilot_state(project_name.clone(), active).await?;
    if active {
        state
            .autopilot_runtime
            .start(state.pipeline_state.clone(), project_name)
            .await?;
    }
    Ok(project)
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
            invalidate_autopilot_job(ap);
        }
    } else {
        // Not executing: just set autopilot to paused
        if let Some(ref mut ap) = proj.workflow_state.autopilot_state {
            ap.run_status = project::AutopilotRunStatus::Paused;
            ap.last_action = "自动驾驶已暂停".to_string();
            ap.last_action_at = now.clone();
            invalidate_autopilot_job(ap);
        }
    }

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    crate::save_and_reload_project(&proj)
}

fn invalidate_autopilot_job(state: &mut project::AutopilotState) {
    state.job_generation = state.job_generation.saturating_add(1);
    state.job_owner = project::AutopilotJobOwner::None;
    state.current_action_id.clear();
    state.current_action_kind.clear();
    state.action_started_at.clear();
    state.next_retry_at = None;
}

/// 持久化自动驾驶步骤状态：写入 last_action、last_action_at、run_status、error_message 和 recovery_action
fn autopilot_persist_step_state(
    proj: &mut project::Project,
    action: &str,
    status: project::AutopilotRunStatus,
    error_msg: &str,
    recovery_action: project::AutopilotRecoveryAction,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let truncated_error = truncate_autopilot_error(error_msg);

    if let Some(ref mut ap) = proj.workflow_state.autopilot_state {
        ap.last_action = action.to_string();
        ap.last_action_at = now.clone();
        ap.run_status = status;
        ap.error_message = truncated_error;
        ap.recovery_action = recovery_action;
    }

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;
    Ok(())
}

fn workspace_recovery_action(
    workspace: &project::ExecutionWorkspaceStatus,
) -> Option<project::AutopilotRecoveryAction> {
    if workspace.ready {
        None
    } else if workspace.has_commits
        && workspace
            .issues
            .contains(&project::ExecutionWorkspaceIssue::DirtyWorkingTree)
    {
        Some(project::AutopilotRecoveryAction::ResolveWorkspaceChanges)
    } else {
        Some(project::AutopilotRecoveryAction::PrepareExecutionWorkspace)
    }
}

fn current_control_task<'a>(
    proj: &'a project::Project,
) -> Result<Option<(crate::task_tree::TaskNodeAddress, &'a project::Subtask)>, String> {
    let Some(address) = crate::task_tree::select_current_leaf(proj)? else {
        return Ok(None);
    };
    let task = crate::task_tree::find_task(proj, &address.task_id)?
        .ok_or_else(|| format!("当前叶子任务不存在：{}", address.task_id))?;
    Ok(Some((address, task)))
}

fn evaluate_control_decision(
    proj: &project::Project,
    shadow: bool,
) -> Result<Option<crate::control_scheduler::TaskControlDecision>, String> {
    let Some((address, task)) = current_control_task(proj)? else {
        return Ok(None);
    };
    let workload = crate::workload_policy::current_profile(proj)?;
    let compiled = crate::task_compiler::compile(
        task,
        address.ancestor_task_ids.last().map(String::as_str),
        address.depth,
        workload,
    );
    let facts_fingerprint = task
        .fact_snapshot
        .as_ref()
        .map(|snapshot| snapshot.structural_fingerprint.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");
    Ok(Some(crate::control_scheduler::decide_next_action(
        task,
        &compiled,
        facts_fingerprint,
        shadow,
        proj.human_review_cadence,
    )))
}

fn serial_takeover_allows_macro_fallback(command: &str) -> bool {
    command.is_empty() || matches!(command, "select_mid_stage" | "transition_workflow")
}

fn policy_state_takes_execution_priority(proj: &project::Project) -> bool {
    let recovery_is_active = proj
        .workflow_state
        .recovery_state
        .as_ref()
        .is_some_and(|recovery| {
            matches!(
                recovery.phase,
                project::RecoveryPhase::Diagnosing
                    | project::RecoveryPhase::Repairing
                    | project::RecoveryPhase::Retesting
                    | project::RecoveryPhase::Replanning
            )
        });
    let execution_session_is_active = proj
        .execution_session
        .as_ref()
        .is_some_and(|session| session.active && session.status.eq_ignore_ascii_case("executing"));
    recovery_is_active || execution_session_is_active
}

fn classify_autopilot_precondition(
    proj: &project::Project,
) -> Result<Option<(String, project::AutopilotRecoveryAction)>, String> {
    if let Err(error) = require_fresh_autopilot_profile(proj) {
        return Ok(Some((
            error,
            project::AutopilotRecoveryAction::WaitHumanDecision,
        )));
    }
    let step = &proj.workflow_state.current_step;
    if !matches!(
        step,
        project::WorkflowStep::PlanApproving | project::WorkflowStep::Execution
    ) {
        return Ok(None);
    }
    let scope = match crate::plan_scope::PlanScope::resolve(proj) {
        Ok(scope) => scope,
        Err(error) => {
            return Ok(Some((
                error,
                project::AutopilotRecoveryAction::WaitHumanDecision,
            )));
        }
    };
    let subtasks = scope.subtasks(proj);
    if let Err(error) = crate::plan_contract::validate_subtasks(subtasks) {
        return Ok(Some((
            format!("执行计划契约无效：{}", error),
            if *step == project::WorkflowStep::PlanApproving {
                project::AutopilotRecoveryAction::RegenerateExecutionPlan
            } else {
                project::AutopilotRecoveryAction::WaitHumanDecision
            },
        )));
    }

    let execution_needs_clean_workspace = *step == project::WorkflowStep::Execution
        && subtasks
            .iter()
            .any(|subtask| subtask.status == project::SubtaskStatus::Pending)
        && !subtasks.iter().any(|subtask| {
            subtask.status == project::SubtaskStatus::AwaitingConfirmation
                || subtask.status == project::SubtaskStatus::Executing
        });
    if *step == project::WorkflowStep::PlanApproving || execution_needs_clean_workspace {
        let workspace = crate::pipeline::get_execution_workspace_status_inner(&proj.project_path)?;
        if let Some(recovery) = workspace_recovery_action(&workspace) {
            return Ok(Some((workspace.status_message, recovery)));
        }
    }
    Ok(None)
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

    let existing_recovery = proj
        .workflow_state
        .autopilot_state
        .as_ref()
        .map(|autopilot| autopilot.recovery_action.clone())
        .unwrap_or_default();
    let recovery_action = if existing_recovery != project::AutopilotRecoveryAction::None {
        existing_recovery
    } else {
        classify_autopilot_precondition(&proj)?
            .map(|(_, recovery)| recovery)
            .unwrap_or(project::AutopilotRecoveryAction::RetryAutopilotAdvance)
    };
    let failure_kind = crate::autopilot_failure::classify_message(&error_detail);
    let previous_attempt = proj
        .workflow_state
        .autopilot_state
        .as_ref()
        .map(|state| state.transient_retry_count)
        .unwrap_or_default();
    let next_attempt = previous_attempt.saturating_add(1);
    let retry_delay = if crate::autopilot_failure::is_transient(&failure_kind) {
        crate::autopilot_failure::retry_delay_secs(next_attempt)
    } else {
        None
    };

    if let Some(delay_secs) = retry_delay {
        let now = chrono::Utc::now();
        autopilot_persist_step_state(
            &mut proj,
            &format!("{}；将在 {} 秒后自动重试", action_description, delay_secs),
            project::AutopilotRunStatus::Running,
            &error_detail,
            recovery_action,
        )?;
        if let Some(state) = proj.workflow_state.autopilot_state.as_mut() {
            state.transient_retry_count = next_attempt;
            state.next_retry_at =
                Some((now + chrono::Duration::seconds(delay_secs as i64)).to_rfc3339());
            state.last_failure_kind = failure_kind;
            state.last_failure_fingerprint =
                crate::autopilot_policy::text_fingerprint(&error_detail);
        }
    } else {
        autopilot_persist_step_state(
            &mut proj,
            &action_description,
            project::AutopilotRunStatus::ErrorStopped,
            &error_detail,
            recovery_action,
        )?;
        if let Some(state) = proj.workflow_state.autopilot_state.as_mut() {
            state.next_retry_at = None;
            state.last_failure_kind = failure_kind;
            state.last_failure_fingerprint =
                crate::autopilot_policy::text_fingerprint(&error_detail);
        }
    }

    crate::save_and_reload_project(&proj)
}

pub(crate) async fn autopilot_resume_state(
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    if !proj.workflow_state.autopilot_active {
        return Err("自动驾驶未激活。".to_string());
    }

    // Verify recovery conditions
    let can_resume = match proj.workflow_state.autopilot_state.as_ref() {
        Some(ap) => match ap.run_status {
            project::AutopilotRunStatus::Paused => true,
            project::AutopilotRunStatus::ErrorStopped => {
                match ap.recovery_action {
                    project::AutopilotRecoveryAction::RestoreExecutionBaseline => {
                        return Err(
                            "存在执行失败需要先恢复执行基线，请先完成基线恢复后再恢复自动驾驶。"
                                .to_string(),
                        );
                    }
                    project::AutopilotRecoveryAction::WaitHumanDecision => {
                        return Err("当前错误需要先完成人工决策。".to_string());
                    }
                    project::AutopilotRecoveryAction::SyncAndClose => {
                        return Err("当前状态只允许同步并关闭自动驾驶。".to_string());
                    }
                    project::AutopilotRecoveryAction::RegenerateExecutionPlan => {
                        return Err("当前执行计划需要先重新生成。".to_string());
                    }
                    project::AutopilotRecoveryAction::PrepareExecutionWorkspace => {
                        return Err("请先准备 Git 执行工作区。".to_string());
                    }
                    project::AutopilotRecoveryAction::ResolveWorkspaceChanges => {
                        return Err("请先处理工作区变更并刷新状态。".to_string());
                    }
                    project::AutopilotRecoveryAction::RunAutomaticRecovery => {
                        return Err("自动错误恢复正在进行，不能手动跳过。".to_string());
                    }
                    project::AutopilotRecoveryAction::RetryGitConfirmation => {
                        return Err("Git 确认受阻，请先重新确认提交。".to_string());
                    }
                    project::AutopilotRecoveryAction::None
                    | project::AutopilotRecoveryAction::RetryAutopilotAdvance => {}
                }
                // ErrorStopped can only resume if there's no unresolved quality failure
                if proj.workflow_state.current_step == project::WorkflowStep::Execution {
                    if let Some(ref session) = proj.execution_session {
                        if session.status == "awaiting_confirmation"
                            || session.status == "quality_blocked"
                            || session.status == "confirmation_blocked"
                            || session.is_recoverable_failure()
                        {
                            return Err(
                                "存在未处理的执行会话，请先恢复基线或处理质量结果后再恢复自动驾驶。"
                                    .to_string(),
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
        ap.recovery_action = project::AutopilotRecoveryAction::None;
        ap.job_id = uuid::Uuid::new_v4().to_string();
        ap.job_generation = ap.job_generation.saturating_add(1);
        ap.job_owner = project::AutopilotJobOwner::BackendRuntime;
        ap.current_action_id.clear();
        ap.current_action_kind.clear();
        ap.action_started_at.clear();
        ap.heartbeat_at = now.clone();
        ap.transient_retry_count = 0;
        ap.next_retry_at = None;
    }

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    crate::save_and_reload_project(&proj)
}

/// 恢复自动驾驶：验证人工边界后创建新一代后端作业。
#[tauri::command]
pub(crate) async fn autopilot_resume(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    let project = autopilot_resume_state(project_name.clone()).await?;
    state
        .autopilot_runtime
        .start(state.pipeline_state.clone(), project_name)
        .await?;
    Ok(project)
}

/// 自动驾驶下一步顾问：采集外部事实后调用纯策略，并只持久化策略终止指令。
pub use crate::autopilot_policy::AutopilotNextStep;

#[tauri::command]
pub(crate) async fn autopilot_next_step(project_name: String) -> Result<AutopilotNextStep, String> {
    let mut proj = crate::load_project(&project_name)?;

    let precondition_block =
        classify_autopilot_precondition(&proj)?.map(|(description, recovery_action)| {
            crate::autopilot_policy::AutopilotPolicyBlock {
                description,
                recovery_action,
            }
        });
    let has_awaiting_confirmation = proj.workflow_state.current_step
        == project::WorkflowStep::Execution
        && proj.execution_session.as_ref().is_some_and(|session| {
            crate::task_tree::find_task(&proj, &session.subtask_id)
                .ok()
                .flatten()
                .is_some_and(|task| task.status == project::SubtaskStatus::AwaitingConfirmation)
        });
    let quality_gate = if has_awaiting_confirmation {
        match crate::pipeline::validate_subtask_quality_gate(&proj) {
            Ok(()) => crate::autopilot_policy::QualityGateFact::Passed,
            Err(reason) => crate::autopilot_policy::QualityGateFact::Failed(reason),
        }
    } else {
        crate::autopilot_policy::QualityGateFact::NotApplicable
    };
    let needs_calibration = if proj.workflow_state.current_step == project::WorkflowStep::Execution
    {
        crate::project_facts::next_task_needs_scan_or_calibration(&proj).unwrap_or(true)
    } else {
        false
    };
    let facts = crate::autopilot_policy::AutopilotPolicyFacts {
        precondition_block,
        quality_gate,
        needs_calibration,
    };
    // Recovery and in-flight session reconciliation are policy-owned states. They must be
    // resolved before either controller tries to select a new leaf from the task tree.
    let policy_state_has_priority = policy_state_takes_execution_priority(&proj);
    // Shadow mode computes a task decision but still returns and executes the legacy command.
    let shadow_decision = if proj.workflow_state.autopilot_active
        && proj.workflow_state.current_step == project::WorkflowStep::Execution
        && proj.task_control.mode == crate::task_control::TaskControlMode::Shadow
        && !policy_state_has_priority
    {
        evaluate_control_decision(&proj, true)?
    } else {
        None
    };

    let serial_takeover_execution = proj.workflow_state.current_step
        == project::WorkflowStep::Execution
        && proj.task_control.mode == crate::task_control::TaskControlMode::SerialTakeover
        && !policy_state_has_priority;
    let mut serial_takeover_without_leaf = false;
    if serial_takeover_execution {
        if let Err(reason) = crate::task_control::ensure_serial_takeover_capability(&proj) {
            let description = format!("串行接管不可用：{}", reason);
            autopilot_persist_step_state(
                &mut proj,
                &description,
                project::AutopilotRunStatus::ErrorStopped,
                &description,
                project::AutopilotRecoveryAction::WaitHumanDecision,
            )?;
            crate::save_project(&proj)?;
            return Ok(AutopilotNextStep {
                command: String::new(),
                args: serde_json::json!({}),
                description: description.clone(),
                at_milestone_boundary: false,
                is_error: true,
                error_message: description,
                result_kind: project::AutopilotCommandResultKind::NoResult,
                waiting_for_execution: false,
            });
        }
        if let Some(block) = facts.precondition_block.as_ref() {
            autopilot_persist_step_state(
                &mut proj,
                &block.description,
                project::AutopilotRunStatus::ErrorStopped,
                &block.description,
                block.recovery_action.clone(),
            )?;
            crate::save_project(&proj)?;
            return Ok(AutopilotNextStep {
                command: String::new(),
                args: serde_json::json!({}),
                description: block.description.clone(),
                at_milestone_boundary: false,
                is_error: true,
                error_message: block.description.clone(),
                result_kind: project::AutopilotCommandResultKind::NoResult,
                waiting_for_execution: false,
            });
        }
        if let Some(mut control) = evaluate_control_decision(&proj, false)? {
            let no_progress = proj
                .workflow_state
                .autopilot_state
                .as_ref()
                .is_some_and(|state| {
                    crate::control_scheduler::should_stop_no_progress(state.consecutive_no_progress)
                });
            if no_progress {
                control.action.kind = crate::control_action::ControlActionKind::Human;
                control.action.reason =
                    "控制动作连续未产生新的合同、事实或证据，已进入人工边界".to_string();
                control.reason = control.action.reason.clone();
            }
            let fingerprint = crate::control_scheduler::decision_fingerprint(&control);
            if fingerprint == proj.task_control.last_decision_fingerprint
                && !proj.task_control.last_decision_id.is_empty()
            {
                control.decision_id = proj.task_control.last_decision_id.clone();
            }
            proj.task_control.last_decision_fingerprint = fingerprint;
            proj.task_control.last_decision_id = control.decision_id.clone();
            proj.task_control.last_decision = Some(control.clone());
            proj.task_control.last_shadow_decision_summary = control.reason.clone();
            proj.task_control.control_source = "task_controller".to_string();
            crate::save_project(&proj)?;
            if control.action.kind == crate::control_action::ControlActionKind::Wait {
                return Ok(AutopilotNextStep {
                    command: String::new(),
                    args: serde_json::json!({}),
                    description: control.reason,
                    at_milestone_boundary: false,
                    is_error: false,
                    error_message: String::new(),
                    result_kind: project::AutopilotCommandResultKind::NoResult,
                    waiting_for_execution: true,
                });
            }
            let request = crate::control_action_executor::ControlActionRequest {
                action_id: format!(
                    "control-{}-{}",
                    control.decision_id,
                    control.action.kind.as_str()
                ),
                action: control.action.kind,
                task_id: control.task_id,
                decision_id: control.decision_id,
                expected_project_revision: Some(proj.workflow_state.data_revision),
                expected_tree_revision: Some(proj.task_control.tree_revision),
                contract_fingerprint: control.contract_fingerprint,
                criterion_indexes: Vec::new(),
                reason: control.reason.clone(),
                source: project::OperationSource::Autopilot,
            };
            return Ok(AutopilotNextStep {
                command: "execute_control_action".to_string(),
                args: serde_json::json!({ "request": request }),
                description: control.reason,
                at_milestone_boundary: false,
                is_error: false,
                error_message: String::new(),
                result_kind: project::AutopilotCommandResultKind::ProjectState,
                waiting_for_execution: false,
            });
        }
        serial_takeover_without_leaf = true;
    }
    let mut decision = crate::autopilot_policy::decide_next_step(&proj, &project_name, &facts);

    if serial_takeover_without_leaf
        && !serial_takeover_allows_macro_fallback(&decision.next.command)
    {
        let rejected_command = decision.next.command.clone();
        let description = format!(
            "串行接管没有可选择的叶子任务，已拒绝旧任务级命令 `{}`；请检查父任务聚合和任务树状态",
            rejected_command
        );
        autopilot_persist_step_state(
            &mut proj,
            &description,
            project::AutopilotRunStatus::ErrorStopped,
            &description,
            project::AutopilotRecoveryAction::WaitHumanDecision,
        )?;
        crate::save_project(&proj)?;
        return Ok(AutopilotNextStep {
            command: String::new(),
            args: serde_json::json!({}),
            description: description.clone(),
            at_milestone_boundary: false,
            is_error: true,
            error_message: description,
            result_kind: project::AutopilotCommandResultKind::NoResult,
            waiting_for_execution: false,
        });
    }

    if decision.kind == crate::autopilot_policy::AutopilotDecisionKind::InitializeQualityRecovery {
        let reason = decision
            .quality_recovery_reason
            .as_deref()
            .unwrap_or("质量门禁未通过")
            .to_string();
        let automatic = crate::recovery::ensure_quality_recovery(&mut proj, &reason)?;
        crate::save_project(&proj)?;
        decision =
            crate::autopilot_policy::resolve_quality_recovery(&project_name, &reason, automatic);
    }

    if let Some(mut shadow) = shadow_decision {
        let fingerprint = crate::control_scheduler::decision_fingerprint(&shadow);
        if fingerprint == proj.task_control.last_decision_fingerprint
            && !proj.task_control.last_decision_id.is_empty()
        {
            shadow.decision_id = proj.task_control.last_decision_id.clone();
        }
        proj.task_control.last_decision_fingerprint = fingerprint;
        proj.task_control.last_decision_id = shadow.decision_id.clone();
        proj.task_control.last_decision = Some(shadow.clone());
        proj.task_control.last_shadow_decision_at = Some(chrono::Utc::now().to_rfc3339());
        proj.task_control.last_shadow_decision_summary =
            format!("{:?}：{}", shadow.action.kind, shadow.reason);
        proj.task_control.control_source = "shadow_controller".to_string();
        proj.task_control
            .record_shadow_comparison(&shadow, &decision.next);
        crate::save_project(&proj)?;
    }

    if proj.workflow_state.autopilot_active {
        if let Some(terminal) = decision.terminal.as_ref() {
            let error = if decision.next.is_error {
                decision.next.error_message.as_str()
            } else {
                ""
            };
            autopilot_persist_step_state(
                &mut proj,
                &decision.next.description,
                terminal.run_status.clone(),
                error,
                terminal.recovery_action.clone(),
            )?;
            crate::save_project(&proj)?;
        }
    }

    Ok(decision.next)
}

// ===================================================================
// 迁移时执行会话与 autopilot 对账
// ===================================================================

fn reconcile_discussion_threads_in_migration(proj: &mut project::Project) -> bool {
    let mut changed = false;
    let now = chrono::Utc::now().to_rfc3339();
    for thread in &mut proj.discussion_threads {
        let revision = thread.revision.max(thread.messages.len() as u64);
        if thread.revision != revision {
            thread.revision = revision;
            changed = true;
        }
        if thread.opened_at.is_empty() {
            thread.opened_at = now.clone();
            changed = true;
        }
    }

    let future_context = proj.workflow_state.current_step
        == project::WorkflowStep::FuturePlanApproval
        || (proj.workflow_state.current_step == project::WorkflowStep::BranchDiscussion
            && (proj.workflow_state.discussion_scope == project::DiscussionScope::AdjustFuture
                || proj.milestone_draft.as_ref().is_some_and(|draft| {
                    draft.draft_kind == project::MilestoneDraftKind::FutureOnly
                })));
    if future_context
        && proj.workflow_state.discussion_scope != project::DiscussionScope::AdjustFuture
    {
        proj.workflow_state.discussion_scope = project::DiscussionScope::AdjustFuture;
        changed = true;
    }

    let mut confirmed_future_source: Option<(String, u64)> = None;
    if future_context {
        let explicit_source = proj
            .milestone_draft
            .as_ref()
            .filter(|draft| draft.draft_kind == project::MilestoneDraftKind::FutureOnly)
            .map(|draft| (draft.source_thread_id.clone(), draft.source_thread_revision));
        if let Some((source_id, source_revision)) = explicit_source {
            if !source_id.is_empty() {
                confirmed_future_source = proj
                    .discussion_threads
                    .iter()
                    .find(|thread| {
                        thread.id == source_id
                            && thread.scope == project::DiscussionScope::AdjustFuture
                            && thread.milestone_id == proj.current_milestone_id
                            && thread.status == project::DiscussionThreadStatus::Open
                            && (source_revision == 0 || source_revision == thread.revision)
                    })
                    .map(|thread| (thread.id.clone(), thread.revision));
            } else {
                let candidates = proj
                    .discussion_threads
                    .iter()
                    .filter(|thread| {
                        thread.scope == project::DiscussionScope::AdjustFuture
                            && thread.milestone_id == proj.current_milestone_id
                            && thread.status == project::DiscussionThreadStatus::Open
                    })
                    .map(|thread| (thread.id.clone(), thread.revision))
                    .collect::<Vec<_>>();
                if candidates.len() == 1 {
                    confirmed_future_source = candidates.into_iter().next();
                }
            }
        }
        if let Some((source_id, _)) = confirmed_future_source.as_ref() {
            if proj.workflow_state.active_discussion_thread_id != *source_id {
                proj.workflow_state.active_discussion_thread_id = source_id.clone();
                changed = true;
            }
        }
    }

    let active_is_valid = proj.active_discussion_thread().is_some_and(|thread| {
        thread.status == project::DiscussionThreadStatus::Open
            && thread.scope == proj.workflow_state.discussion_scope
            && (thread.scope == project::DiscussionScope::FirstDiscussion
                || thread.milestone_id == proj.current_milestone_id)
    });
    if !active_is_valid {
        let scope = proj.workflow_state.discussion_scope.clone();
        let milestone_id = if scope == project::DiscussionScope::FirstDiscussion {
            String::new()
        } else {
            proj.current_milestone_id.clone()
        };
        let review_cycle_id = if scope == project::DiscussionScope::FirstDiscussion {
            String::new()
        } else {
            format!(
                "{}:{}",
                milestone_id,
                if proj.workflow_state.last_transition_at.is_empty() {
                    now.as_str()
                } else {
                    proj.workflow_state.last_transition_at.as_str()
                }
            )
        };
        let previous_active = proj.workflow_state.active_discussion_thread_id.clone();
        let previous_len = proj.discussion_threads.len();
        proj.activate_discussion_thread(scope, &milestone_id, &review_cycle_id);
        changed |= previous_active != proj.workflow_state.active_discussion_thread_id
            || previous_len != proj.discussion_threads.len();
    }

    if future_context {
        let current_revision = proj.workflow_state.data_revision;
        if let Some(draft) = proj
            .milestone_draft
            .as_mut()
            .filter(|draft| draft.draft_kind == project::MilestoneDraftKind::FutureOnly)
        {
            if let Some((source_id, source_revision)) = confirmed_future_source {
                if draft.source_thread_id != source_id {
                    draft.source_thread_id = source_id;
                    changed = true;
                }
                if draft.source_thread_revision != source_revision {
                    draft.source_thread_revision = source_revision;
                    changed = true;
                }
                if draft.generation_revision == 0 && source_revision > 0 {
                    draft.generation_revision = source_revision;
                    changed = true;
                }
                if draft.source_data_revision.saturating_add(1) != current_revision {
                    if !draft.expired {
                        draft.expired = true;
                        changed = true;
                    }
                    let reason = "旧未来草稿的项目事实修订无法确认，请重新生成。".to_string();
                    if draft.expiration_reason.as_ref() != Some(&reason) {
                        draft.expiration_reason = Some(reason);
                        changed = true;
                    }
                }
            } else {
                if !draft.expired {
                    draft.expired = true;
                    changed = true;
                }
                let reason = "旧未来草稿无法确认来源讨论线程，已保留但禁止批准。".to_string();
                if draft.expiration_reason.as_ref() != Some(&reason) {
                    draft.expiration_reason = Some(reason);
                    changed = true;
                }
            }
        }
    }

    changed
}

fn reconcile_legacy_mid_stage_approval(proj: &mut project::Project) -> bool {
    if proj.workflow_state.current_step != project::WorkflowStep::MidStageApproval {
        return false;
    }
    let Some(milestone) = proj
        .milestones
        .iter()
        .find(|milestone| milestone.id == proj.current_milestone_id)
    else {
        return false;
    };
    if milestone.mid_stages.is_empty() {
        return false;
    }

    if let Some(draft) = proj.mid_stage_draft.as_mut() {
        draft.status = project::MidStageDraftStatus::CheckFailed;
        draft.check_result =
            Some("迁移归档：当前大阶段已有正式中阶段，遗留首次整表草稿禁止批准。".to_string());
        draft.allow_full_replacement = false;
        draft.last_regeneration_reason =
            Some("旧项目中阶段事实优先，遗留整表草稿已归档。".to_string());
    }
    proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
    proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
    true
}

pub(crate) fn reconcile_workflow_closure_state(
    proj: &mut project::Project,
) -> Result<bool, String> {
    let mut candidate = proj.clone();
    let initial_revision = candidate.workflow_state.data_revision;
    let now = chrono::Utc::now().to_rfc3339();
    let mut changed = reconcile_discussion_threads_in_migration(&mut candidate);

    if candidate.workflow_state.current_step == project::WorkflowStep::PlanApproval
        && candidate.plan_draft.is_none()
    {
        candidate.workflow_state.current_step = project::WorkflowStep::ProjectPlanGeneration;
        candidate.workflow_state.top_level_phase = project::TopLevelPhase::FirstDiscussion;
        changed = true;
    }
    if candidate.workflow_state.current_step == project::WorkflowStep::ThreeChecks
        && has_valid_preflight_checks(&candidate)
    {
        candidate.workflow_state.current_step = project::WorkflowStep::ProjectPlanGeneration;
        changed = true;
    }

    changed |= reconcile_legacy_mid_stage_approval(&mut candidate);
    changed |= crate::workflow_resolution::reconcile_mid_stage_route(&mut candidate, &now)?;
    if changed && candidate.workflow_state.data_revision == initial_revision {
        candidate.workflow_state.data_revision = initial_revision.saturating_add(1);
        candidate.workflow_state.last_transition_at = now;
    }
    if changed {
        *proj = candidate;
    }
    Ok(changed)
}

fn subtask_has_execution_facts(subtask: &project::Subtask) -> bool {
    !matches!(
        subtask.status,
        project::SubtaskStatus::Pending | project::SubtaskStatus::RolledBack
    ) || subtask.execution_result.is_some()
        || subtask.test_result.is_some()
        || subtask.auto_tag.as_ref().is_some_and(|tag| !tag.is_empty())
}

fn reconcile_plan_contract_target(
    subtasks: &[project::Subtask],
    plan_generated: bool,
    plan_revision: &mut u64,
    plan_approved_at: &mut Option<String>,
    plan_check_result: &mut Option<project::StagePlanCheckResult>,
    has_execution_facts: bool,
) -> Option<(String, bool)> {
    if subtasks.is_empty() && !plan_generated && *plan_revision == 0 {
        return None;
    }
    let error = crate::plan_contract::validate_subtasks(subtasks).err()?;
    if !has_execution_facts {
        *plan_approved_at = None;
        *plan_revision = 0;
        *plan_check_result = Some(project::StagePlanCheckResult {
            passed: false,
            omissions: vec![],
            out_of_scope: vec![],
            not_executable: vec![error.clone()],
            suggestions: vec!["旧执行计划缺少合法文件范围，请重新生成。".to_string()],
            checked_at: chrono::Utc::now().to_rfc3339(),
        });
    }
    Some((error, has_execution_facts))
}

fn reconcile_plan_contract_in_migration(proj: &mut project::Project) {
    let current_milestone_id = proj.current_milestone_id.clone();
    let current_mid_stage_id = proj.current_mid_stage_id.clone();
    let mut current_invalid_without_facts: Option<String> = None;
    let mut current_invalid_with_facts: Option<String> = None;

    for milestone in &mut proj.milestones {
        if milestone.mode == project::StageMode::Quick {
            let has_facts = matches!(
                milestone.status,
                project::MilestoneStatus::InProgress | project::MilestoneStatus::Completed
            ) || milestone.subtasks.iter().any(subtask_has_execution_facts);
            let is_current =
                milestone.id == current_milestone_id && current_mid_stage_id.is_empty();
            if let Some((error, had_facts)) = reconcile_plan_contract_target(
                &milestone.subtasks,
                milestone.plan_generated_at.is_some(),
                &mut milestone.plan_revision,
                &mut milestone.plan_approved_at,
                &mut milestone.plan_check_result,
                has_facts,
            ) {
                if is_current {
                    if had_facts {
                        current_invalid_with_facts = Some(error);
                    } else {
                        current_invalid_without_facts = Some(error);
                    }
                }
            }
        }
        for mid_stage in &mut milestone.mid_stages {
            let has_facts = matches!(
                mid_stage.status,
                project::MidStageStatus::InProgress | project::MidStageStatus::Completed
            ) || mid_stage.completed_at.is_some()
                || !mid_stage.git_tag.is_empty()
                || mid_stage.subtasks.iter().any(subtask_has_execution_facts);
            let is_current =
                milestone.id == current_milestone_id && mid_stage.id == current_mid_stage_id;
            if let Some((error, had_facts)) = reconcile_plan_contract_target(
                &mid_stage.subtasks,
                mid_stage.plan_generated_at.is_some(),
                &mut mid_stage.plan_revision,
                &mut mid_stage.plan_approved_at,
                &mut mid_stage.plan_check_result,
                has_facts,
            ) {
                if is_current {
                    if had_facts {
                        current_invalid_with_facts = Some(error);
                    } else {
                        current_invalid_without_facts = Some(error);
                    }
                }
            }
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    if let Some(error) = current_invalid_without_facts {
        proj.workflow_state.current_step = project::WorkflowStep::PlanCheck;
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
            autopilot.run_status = project::AutopilotRunStatus::ErrorStopped;
            autopilot.last_action = "旧执行计划需要重新生成".to_string();
            autopilot.last_action_at = now.clone();
            autopilot.error_message = error;
            autopilot.recovery_action = project::AutopilotRecoveryAction::RegenerateExecutionPlan;
        }
        proj.workflow_state.data_revision += 1;
        proj.workflow_state.last_transition_at = now;
    } else if let Some(error) = current_invalid_with_facts {
        if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
            autopilot.run_status = project::AutopilotRunStatus::ErrorStopped;
            autopilot.last_action = "已执行计划的文件范围无效，需要人工回退".to_string();
            autopilot.last_action_at = now.clone();
            autopilot.error_message = error;
            autopilot.recovery_action = project::AutopilotRecoveryAction::WaitHumanDecision;
        }
        proj.workflow_state.data_revision += 1;
        proj.workflow_state.last_transition_at = now;
    }
}

/// 在 migrate_project_workflow 中 autopilot sanity 检查
pub(crate) fn reconcile_autopilot_in_migration(proj: &mut crate::project::Project) {
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

    let mut convergence_changed = false;
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(state) = proj.workflow_state.autopilot_state.as_mut() {
        if !state.active {
            state.active = true;
            convergence_changed = true;
        }
        if state.target_milestone_id != proj.workflow_state.autopilot_target_milestone_id {
            state.target_milestone_id = proj.workflow_state.autopilot_target_milestone_id.clone();
            convergence_changed = true;
        }
        if state.job_id.is_empty() {
            state.job_id = uuid::Uuid::new_v4().to_string();
            convergence_changed = true;
        }
        if state.job_generation == 0 {
            state.job_generation = 1;
            convergence_changed = true;
        }
        let expected_owner = if state.run_status == project::AutopilotRunStatus::Running {
            project::AutopilotJobOwner::BackendRuntime
        } else {
            project::AutopilotJobOwner::None
        };
        if state.job_owner != expected_owner {
            state.job_owner = expected_owner;
            convergence_changed = true;
        }
        if state.run_status == project::AutopilotRunStatus::Running && state.heartbeat_at.is_empty()
        {
            state.heartbeat_at = now.clone();
            convergence_changed = true;
        }
    }

    if let Some(draft) = proj.mid_stage_draft.as_mut() {
        if draft.status == project::MidStageDraftStatus::CheckFailed {
            if draft.last_check_failure_fingerprint.is_empty() {
                draft.last_check_failure_fingerprint = crate::autopilot_policy::text_fingerprint(
                    draft.check_result.as_deref().unwrap_or("旧草稿检查失败"),
                );
                convergence_changed = true;
            }
            if draft.last_candidate_fingerprint.is_empty() {
                draft.last_candidate_fingerprint =
                    crate::autopilot_policy::mid_stage_candidate_fingerprint(
                        &draft.candidate_mid_stages,
                    );
                convergence_changed = true;
            }
        }
    }

    let mut exhausted_current_plan = false;
    let mut current_plan_unblocked = false;
    for milestone in &mut proj.milestones {
        if milestone.mode == project::StageMode::Quick {
            if let Some(check) = milestone.plan_check_result.as_mut() {
                let normalized =
                    crate::autopilot_policy::normalize_plan_check_result(check.clone());
                let changed = normalized.passed != check.passed
                    || normalized.omissions != check.omissions
                    || normalized.out_of_scope != check.out_of_scope
                    || normalized.not_executable != check.not_executable
                    || normalized.suggestions != check.suggestions;
                if changed {
                    let was_blocked = !check.passed;
                    *check = normalized;
                    milestone.last_plan_failure_fingerprint.clear();
                    milestone.last_plan_issue_count =
                        crate::autopilot_policy::blocking_plan_issue_count(check);
                    milestone.plan_no_progress_count = 0;
                    current_plan_unblocked |= was_blocked
                        && check.passed
                        && milestone.id == proj.current_milestone_id
                        && proj.current_mid_stage_id.is_empty();
                    convergence_changed = true;
                }
            }
            if let Some(check) = milestone
                .plan_check_result
                .as_ref()
                .filter(|check| !check.passed)
            {
                if milestone.last_plan_failure_fingerprint.is_empty() {
                    milestone.last_plan_failure_fingerprint =
                        crate::autopilot_policy::plan_failure_fingerprint(check);
                    convergence_changed = true;
                }
                if milestone.last_plan_issue_count == 0 {
                    milestone.last_plan_issue_count =
                        crate::autopilot_policy::blocking_plan_issue_count(check);
                    convergence_changed = true;
                }
                if milestone.id == proj.current_milestone_id
                    && proj.current_mid_stage_id.is_empty()
                    && milestone.plan_regeneration_count
                        >= crate::autopilot_policy::MAX_PLANNING_REGENERATIONS
                {
                    exhausted_current_plan = true;
                }
            }
        }
        for mid in &mut milestone.mid_stages {
            if let Some(check) = mid.plan_check_result.as_mut() {
                let normalized =
                    crate::autopilot_policy::normalize_plan_check_result(check.clone());
                let changed = normalized.passed != check.passed
                    || normalized.omissions != check.omissions
                    || normalized.out_of_scope != check.out_of_scope
                    || normalized.not_executable != check.not_executable
                    || normalized.suggestions != check.suggestions;
                if changed {
                    let was_blocked = !check.passed;
                    *check = normalized;
                    mid.last_plan_failure_fingerprint.clear();
                    mid.last_plan_issue_count =
                        crate::autopilot_policy::blocking_plan_issue_count(check);
                    mid.plan_no_progress_count = 0;
                    current_plan_unblocked |= was_blocked
                        && check.passed
                        && milestone.id == proj.current_milestone_id
                        && mid.id == proj.current_mid_stage_id;
                    convergence_changed = true;
                }
            }
            let Some(check) = mid.plan_check_result.as_ref().filter(|check| !check.passed) else {
                continue;
            };
            if mid.last_plan_failure_fingerprint.is_empty() {
                mid.last_plan_failure_fingerprint =
                    crate::autopilot_policy::plan_failure_fingerprint(check);
                convergence_changed = true;
            }
            if mid.last_plan_issue_count == 0 {
                mid.last_plan_issue_count =
                    crate::autopilot_policy::blocking_plan_issue_count(check);
                convergence_changed = true;
            }
            if milestone.id == proj.current_milestone_id
                && mid.id == proj.current_mid_stage_id
                && mid.plan_regeneration_count
                    >= crate::autopilot_policy::MAX_PLANNING_REGENERATIONS
            {
                exhausted_current_plan = true;
            }
        }
    }
    if current_plan_unblocked
        && proj.workflow_state.current_step == project::WorkflowStep::PlanCheck
    {
        proj.workflow_state.current_step = project::WorkflowStep::PlanApproving;
        convergence_changed = true;
    }
    if exhausted_current_plan {
        if let Some(state) = proj.workflow_state.autopilot_state.as_mut() {
            state.run_status = project::AutopilotRunStatus::ErrorStopped;
            state.recovery_action = project::AutopilotRecoveryAction::WaitHumanDecision;
            state.error_message = "旧执行计划重生成记录已达到两次上限，需要人工确认。".to_string();
            state.last_action = "迁移时停止无上限执行计划重生成".to_string();
            state.last_action_at = now.clone();
            state.job_owner = project::AutopilotJobOwner::None;
            state.current_action_id.clear();
            state.current_action_kind.clear();
            state.action_started_at.clear();
            state.next_retry_at = None;
            convergence_changed = true;
        }
    }
    if convergence_changed {
        proj.workflow_state.data_revision = proj.workflow_state.data_revision.saturating_add(1);
        proj.workflow_state.last_transition_at = now;
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
    use std::path::PathBuf;

    struct ProjectDataGuard {
        path: PathBuf,
    }

    impl ProjectDataGuard {
        fn new(project_name: &str) -> Result<Self, String> {
            Ok(Self {
                path: crate::project_data_path(project_name)?,
            })
        }
    }

    impl Drop for ProjectDataGuard {
        fn drop(&mut self) {
            if let Err(error) = std::fs::remove_file(&self.path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("清理测试项目 {} 失败：{}", self.path.display(), error);
                }
            }
        }
    }

    struct TestGitWorkspace {
        path: PathBuf,
    }

    impl TestGitWorkspace {
        fn new(label: &str) -> Result<Self, String> {
            let path = std::env::temp_dir().join(format!(
                "metheus-workflow-{}-{}",
                label,
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
            let workspace = Self { path };
            workspace.git(&["init", "-q"])?;
            workspace.git(&["config", "user.name", "Metheus Test"])?;
            workspace.git(&["config", "user.email", "metheus@example.invalid"])?;
            std::fs::write(workspace.path.join("tracked.txt"), "baseline\n")
                .map_err(|error| error.to_string())?;
            workspace.git(&["add", "tracked.txt"])?;
            workspace.git(&["commit", "-q", "-m", "baseline"])?;
            Ok(workspace)
        }

        fn git(&self, args: &[&str]) -> Result<(), String> {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(&self.path)
                .output()
                .map_err(|error| error.to_string())?;
            if output.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
            }
        }
    }

    impl Drop for TestGitWorkspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn unique_project_name(label: &str) -> String {
        format!("test-{}-{}", label, uuid::Uuid::new_v4())
    }

    #[tokio::test]
    async fn plan_approval_requires_a_concrete_draft() -> Result<(), String> {
        let project_name = unique_project_name("plan-approval-contract");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.top_level_phase = project::TopLevelPhase::FirstDiscussion;
        proj.workflow_state.current_step = project::WorkflowStep::ProjectPlanGeneration;
        crate::save_project(&proj)?;

        let error =
            transition_workflow(project_name, "PlanApproval".to_string(), "test".to_string())
                .await
                .expect_err("无草稿时不得进入审批步骤");
        assert!(error.contains("没有可审批"));
        Ok(())
    }

    #[tokio::test]
    async fn migration_reconciles_legacy_empty_plan_approval() -> Result<(), String> {
        let project_name = unique_project_name("empty-plan-approval");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.top_level_phase = project::TopLevelPhase::FirstDiscussion;
        proj.workflow_state.current_step = project::WorkflowStep::PlanApproval;
        crate::save_project(&proj)?;

        let migrated = migrate_project_workflow(project_name).await?;
        assert_eq!(
            migrated.workflow_state.current_step,
            project::WorkflowStep::ProjectPlanGeneration
        );
        assert!(migrated.plan_draft.is_none());
        Ok(())
    }

    fn test_subtask(status: project::SubtaskStatus) -> project::Subtask {
        project::Subtask {
            id: "subtask-1".to_string(),
            title: "测试小阶段".to_string(),
            prompt: "执行测试".to_string(),
            status,
            test_report: String::new(),
            execution_result: None,
            test_result: None,
            retry_count: 0,
            auto_tag: None,
            order: 1,
            goal: String::new(),
            allowed_file_paths: vec!["tracked.txt".to_string()],
            new_file_paths: vec![],
            evidence_files: vec![],
            context_summary: String::new(),
            acceptance_criteria: vec![],
            stop_rules: vec![],
            execution_prompt: String::new(),
            confirmed_by_user: None,
            confirmed_at: None,
            confirmation_notes: None,
            human_verification: None,
            ..Default::default()
        }
    }

    fn test_mid_stage(status: project::MidStageStatus) -> project::MidStage {
        project::MidStage {
            id: "mid-1".to_string(),
            title: "测试中阶段".to_string(),
            version: "v0.1.1".to_string(),
            order: Some(1),
            status,
            subtasks: vec![],
            domain: None,
            test_log: None,
            created_at: String::new(),
            description: String::new(),
            tech_focus: String::new(),
            test_report: String::new(),
            completed_at: None,
            approved_at: None,
            git_tag: String::new(),
            plan_check_result: None,
            plan_approved_at: None,
            plan_revision: 0,
            plan_draft_revision: 0,
            plan_generated_at: None,
            plan_regeneration_count: 0,
            last_plan_failure_fingerprint: String::new(),
            last_plan_issue_count: 0,
            plan_no_progress_count: 0,
        }
    }

    fn test_milestone(
        id: &str,
        title: &str,
        status: project::MilestoneStatus,
    ) -> project::Milestone {
        project::Milestone {
            id: id.to_string(),
            version: "v0.1".to_string(),
            title: title.to_string(),
            description: String::new(),
            tech_stack: String::new(),
            status,
            mode: project::StageMode::Professional,
            mid_stages: vec![],
            subtasks: vec![],
            qa_result: None,
            git_commit_hash: String::new(),
            decomposition_check: None,
            review_status: None,
            review_conclusion: None,
            approved_at: None,
            goal: String::new(),
            scope: String::new(),
            dependencies: vec![],
            expected_output: String::new(),
            acceptance_criteria: vec![],
            ..Default::default()
        }
    }

    #[test]
    fn workflow_closure_migration_expires_unattributed_future_draft() {
        let mut proj = project::Project::new("legacy-future-unattributed");
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::FuturePlanApproval;
        proj.workflow_state.discussion_scope = project::DiscussionScope::FirstDiscussion;
        proj.workflow_state.data_revision = 5;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.milestones.push(test_milestone(
            "milestone-1",
            "当前大阶段",
            project::MilestoneStatus::Completed,
        ));
        proj.milestone_draft = Some(project::MilestoneDraft {
            draft_kind: project::MilestoneDraftKind::FutureOnly,
            candidate_milestones: vec![test_milestone(
                "milestone-2",
                "未来大阶段",
                project::MilestoneStatus::Pending,
            )],
            ..Default::default()
        });

        assert!(reconcile_workflow_closure_state(&mut proj).unwrap());
        assert_eq!(
            proj.workflow_state.discussion_scope,
            project::DiscussionScope::AdjustFuture
        );
        let active = proj.active_discussion_thread().expect("专属未来讨论线程");
        assert_eq!(active.scope, project::DiscussionScope::AdjustFuture);
        assert_eq!(active.milestone_id, "milestone-1");
        let draft = proj.milestone_draft.as_ref().expect("保留旧未来草稿");
        assert!(draft.expired);
        assert!(draft
            .expiration_reason
            .as_deref()
            .is_some_and(|reason| { reason.contains("无法确认来源讨论线程") }));
    }

    #[test]
    fn workflow_closure_migration_binds_unique_future_thread() {
        let mut proj = project::Project::new("legacy-future-thread");
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::FuturePlanApproval;
        proj.workflow_state.discussion_scope = project::DiscussionScope::AdjustFuture;
        proj.workflow_state.data_revision = 5;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.milestones.push(test_milestone(
            "milestone-1",
            "当前大阶段",
            project::MilestoneStatus::Completed,
        ));
        let thread_id = proj.activate_discussion_thread(
            project::DiscussionScope::AdjustFuture,
            "milestone-1",
            "review-1",
        );
        proj.milestone_draft = Some(project::MilestoneDraft {
            draft_kind: project::MilestoneDraftKind::FutureOnly,
            source_data_revision: 4,
            candidate_milestones: vec![test_milestone(
                "milestone-2",
                "未来大阶段",
                project::MilestoneStatus::Pending,
            )],
            ..Default::default()
        });

        assert!(reconcile_workflow_closure_state(&mut proj).unwrap());
        let draft = proj.milestone_draft.as_ref().expect("未来草稿");
        assert_eq!(draft.source_thread_id, thread_id);
        assert_eq!(draft.source_data_revision, 4);
        assert!(!draft.expired);
    }

    #[test]
    fn workflow_closure_migration_archives_legacy_mid_stage_replacement() {
        let mut proj = project::Project::new("legacy-mid-stage-approval");
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::MidStageApproval;
        proj.current_milestone_id = "milestone-1".to_string();
        let mut milestone = test_milestone(
            "milestone-1",
            "当前大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone
            .mid_stages
            .push(test_mid_stage(project::MidStageStatus::Pending));
        proj.milestones.push(milestone);
        proj.mid_stage_draft = Some(project::MidStageDraft {
            milestone_id: "milestone-1".to_string(),
            candidate_mid_stages: vec![test_mid_stage(project::MidStageStatus::Pending)],
            ..Default::default()
        });

        assert!(reconcile_workflow_closure_state(&mut proj).unwrap());
        assert_eq!(proj.current_mid_stage_id, "mid-1");
        assert_eq!(
            proj.workflow_state.current_step,
            project::WorkflowStep::PlanGeneration
        );
        let draft = proj.mid_stage_draft.as_ref().expect("归档草稿仍保留审计");
        assert_eq!(draft.status, project::MidStageDraftStatus::CheckFailed);
        assert!(!draft.allow_full_replacement);
        assert_eq!(proj.milestones[0].mid_stages.len(), 1);
    }

    #[test]
    fn workflow_closure_migration_selects_existing_next_mid_stage() {
        let mut proj = project::Project::new("existing-next-mid-stage");
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
        proj.current_milestone_id = "milestone-1".to_string();
        let mut completed = test_mid_stage(project::MidStageStatus::Completed);
        completed.id = "mid-completed".to_string();
        completed.order = Some(1);
        let mut pending = test_mid_stage(project::MidStageStatus::Pending);
        pending.id = "mid-next".to_string();
        pending.order = Some(2);
        let mut milestone = test_milestone(
            "milestone-1",
            "当前大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages = vec![completed, pending];
        proj.milestones.push(milestone);

        assert!(reconcile_workflow_closure_state(&mut proj).unwrap());
        assert_eq!(proj.current_mid_stage_id, "mid-next");
        assert_eq!(
            proj.workflow_state.current_step,
            project::WorkflowStep::PlanGeneration
        );
        assert!(proj.mid_stage_draft.is_none());
    }

    fn professional_workload_profile() -> project::WorkloadProfile {
        crate::workload_policy::classify(
            project::WorkloadSignals {
                has_frontend: true,
                has_backend: true,
                has_persistence: false,
                has_auth_or_roles: false,
                external_integration_count: 0,
                independent_domain_count: 3,
                deliverable_count: 3,
                high_risk: false,
            },
            None,
            0,
        )
        .expect("professional test profile")
    }

    fn quick_workload_profile() -> project::WorkloadProfile {
        crate::workload_policy::classify(
            project::WorkloadSignals {
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
        .expect("quick test profile")
    }

    fn system_workload_profile() -> project::WorkloadProfile {
        crate::workload_policy::classify(
            project::WorkloadSignals {
                has_frontend: true,
                has_backend: true,
                has_persistence: true,
                has_auth_or_roles: true,
                external_integration_count: 2,
                independent_domain_count: 4,
                deliverable_count: 4,
                high_risk: true,
            },
            None,
            0,
        )
        .expect("system test profile")
    }

    fn activate_autopilot(proj: &mut project::Project, target: &str) {
        if proj.workload_profile.is_none() {
            proj.workload_profile = Some(professional_workload_profile());
        }
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_target_milestone_id = target.to_string();
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: target.to_string(),
            run_status: project::AutopilotRunStatus::Running,
            last_action: String::new(),
            last_action_at: String::new(),
            error_message: String::new(),
            recovery_action: project::AutopilotRecoveryAction::None,
            ..Default::default()
        });
    }

    fn attach_professional_execution_plan(
        proj: &mut project::Project,
        task_status: project::SubtaskStatus,
    ) {
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id = "mid-1".to_string();
        let mut mid_stage = test_mid_stage(project::MidStageStatus::InProgress);
        mid_stage.subtasks = vec![test_subtask(task_status)];
        mid_stage.plan_generated_at = Some("2026-08-01T00:00:00Z".to_string());
        mid_stage.plan_approved_at = Some("2026-08-01T00:00:00Z".to_string());
        mid_stage.plan_revision = 1;
        let mut milestone = test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages = vec![mid_stage];
        proj.milestones = vec![milestone];
    }

    fn quick_review_project(
        project_name: &str,
        task_status: project::SubtaskStatus,
    ) -> project::Project {
        let mut proj = project::Project::new(project_name);
        proj.workload_profile = Some(quick_workload_profile());
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        proj.current_milestone_id = "milestone-1".to_string();
        let mut milestone = test_milestone(
            "milestone-1",
            "Quick review",
            project::MilestoneStatus::InProgress,
        );
        milestone.mode = project::StageMode::Quick;
        milestone.subtasks = vec![test_subtask(task_status)];
        proj.milestones = vec![milestone];
        proj
    }

    fn professional_review_project(
        project_name: &str,
        stage_statuses: &[project::MidStageStatus],
    ) -> project::Project {
        let mut proj = project::Project::new(project_name);
        proj.workload_profile = Some(system_workload_profile());
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
        proj.current_milestone_id = "milestone-1".to_string();
        let mut milestone = test_milestone(
            "milestone-1",
            "Professional review",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages = stage_statuses
            .iter()
            .enumerate()
            .map(|(index, status)| {
                let mut stage = test_mid_stage(status.clone());
                stage.id = format!("mid-{}", index + 1);
                stage.subtasks = vec![test_subtask(project::SubtaskStatus::Passed)];
                stage
            })
            .collect();
        proj.milestones = vec![milestone];
        proj
    }

    #[tokio::test]
    async fn adaptive_execution_contract_migration_quick_completed_builds_review_boundary(
    ) -> Result<(), String> {
        let project_name = unique_project_name("migration-quick-review");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = quick_review_project(&project_name, project::SubtaskStatus::Passed);
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
        proj.workflow_state.data_revision = 20;
        proj.milestones[0].status = project::MilestoneStatus::Completed;
        proj.milestones[0].review_status = Some("approved".to_string());
        proj.milestones[0].review_conclusion = Some("A".to_string());
        activate_autopilot(&mut proj, "milestone-1");
        crate::save_project(&proj)?;

        let migrated = migrate_project_workflow(project_name).await?;
        assert_eq!(migrated.workflow_state.data_revision, 21);
        assert_eq!(
            migrated.workflow_state.current_step,
            project::WorkflowStep::MilestoneReview
        );
        assert_eq!(migrated.workflow_state.review_node_id, "milestone-1");
        assert_eq!(
            migrated.milestones[0].review_status.as_deref(),
            Some("pending_review")
        );
        assert!(migrated.milestones[0].review_conclusion.is_none());
        assert_eq!(
            migrated
                .workflow_state
                .autopilot_state
                .as_ref()
                .ok_or("migration 缺少 autopilot Review 边界".to_string())?
                .run_status,
            project::AutopilotRunStatus::WaitingMilestoneReview
        );
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_migration_professional_completed_builds_review_boundary(
    ) -> Result<(), String> {
        let project_name = unique_project_name("migration-professional-review");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = professional_review_project(
            &project_name,
            &[
                project::MidStageStatus::Completed,
                project::MidStageStatus::Completed,
            ],
        );
        proj.workflow_state.data_revision = 30;
        proj.milestones[0].review_status = Some("needs_fix".to_string());
        proj.milestones[0].review_conclusion = Some("B".to_string());
        activate_autopilot(&mut proj, "milestone-1");
        crate::save_project(&proj)?;

        let migrated = migrate_project_workflow(project_name).await?;
        assert_eq!(migrated.workflow_state.data_revision, 31);
        assert_eq!(
            migrated.workflow_state.current_step,
            project::WorkflowStep::MilestoneReview
        );
        assert_eq!(
            migrated.milestones[0].status,
            project::MilestoneStatus::Completed
        );
        assert_eq!(
            migrated.milestones[0].review_status.as_deref(),
            Some("pending_review")
        );
        assert!(migrated.milestones[0].review_conclusion.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_migration_profile_errors_do_not_persist_half_review(
    ) -> Result<(), String> {
        for stale in [false, true] {
            let project_name = unique_project_name(if stale {
                "migration-stale-profile"
            } else {
                "migration-missing-profile"
            });
            let _guard = ProjectDataGuard::new(&project_name)?;
            let mut proj = quick_review_project(&project_name, project::SubtaskStatus::Skipped);
            proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
            proj.workflow_state.data_revision = 40;
            proj.milestones[0].status = project::MilestoneStatus::Completed;
            proj.milestones[0].review_status = Some("approved".to_string());
            if stale {
                proj.discussion_revision = 1;
            } else {
                proj.workload_profile = None;
            }
            crate::save_project(&proj)?;

            let error = migrate_project_workflow(project_name.clone())
                .await
                .expect_err("invalid profile must block migration into Review");
            if stale {
                assert!(error.contains("画像已过期"));
            } else {
                assert!(error.contains("画像缺失"));
            }
            let persisted = crate::load_project(&project_name)?;
            assert_eq!(persisted.workflow_state.data_revision, 40);
            assert_eq!(
                persisted.workflow_state.current_step,
                project::WorkflowStep::MilestoneSelection
            );
            assert!(persisted.workflow_state.review_node_id.is_empty());
            assert_eq!(
                persisted.milestones[0].review_status.as_deref(),
                Some("approved")
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_quick_non_terminal_cannot_transition_to_review(
    ) -> Result<(), String> {
        let project_name = unique_project_name("quick-review-non-terminal");
        let _guard = ProjectDataGuard::new(&project_name)?;
        crate::save_project(&quick_review_project(
            &project_name,
            project::SubtaskStatus::Pending,
        ))?;

        let error = transition_workflow(
            project_name,
            "MilestoneReview".to_string(),
            "test: reject unfinished Quick milestone".to_string(),
        )
        .await
        .expect_err("未完成 Quick 任务不得进入大阶段审阅");
        assert!(error.contains("未完成的直挂任务"));
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_quick_terminal_transition_builds_complete_review_boundary(
    ) -> Result<(), String> {
        let project_name = unique_project_name("quick-review-terminal");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = quick_review_project(&project_name, project::SubtaskStatus::Skipped);
        activate_autopilot(&mut proj, "milestone-1");
        crate::save_project(&proj)?;

        let review = transition_workflow(
            project_name,
            "MilestoneReview".to_string(),
            "test: terminal Quick milestone".to_string(),
        )
        .await?;
        assert_eq!(
            review.milestones[0].status,
            project::MilestoneStatus::Completed
        );
        assert_eq!(
            review.milestones[0].review_status.as_deref(),
            Some("pending_review")
        );
        assert_eq!(review.workflow_state.review_node_id, "milestone-1");
        assert_eq!(
            review
                .workflow_state
                .autopilot_state
                .as_ref()
                .unwrap()
                .run_status,
            project::AutopilotRunStatus::WaitingMilestoneReview
        );
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_professional_incomplete_stage_cannot_transition_to_review(
    ) -> Result<(), String> {
        let project_name = unique_project_name("professional-review-incomplete");
        let _guard = ProjectDataGuard::new(&project_name)?;
        crate::save_project(&professional_review_project(
            &project_name,
            &[
                project::MidStageStatus::Completed,
                project::MidStageStatus::InProgress,
            ],
        ))?;

        let error = transition_workflow(
            project_name,
            "MilestoneReview".to_string(),
            "test: reject unfinished Professional milestone".to_string(),
        )
        .await
        .expect_err("未完成 Professional 中阶段不得进入大阶段审阅");
        assert!(error.contains("未完成的中阶段"));
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_professional_terminal_transition_builds_complete_review_boundary(
    ) -> Result<(), String> {
        let project_name = unique_project_name("professional-review-terminal");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = professional_review_project(
            &project_name,
            &[
                project::MidStageStatus::Completed,
                project::MidStageStatus::Completed,
            ],
        );
        activate_autopilot(&mut proj, "milestone-1");
        crate::save_project(&proj)?;

        let review = transition_workflow(
            project_name,
            "MilestoneReview".to_string(),
            "test: terminal Professional milestone".to_string(),
        )
        .await?;
        assert_eq!(
            review.milestones[0].status,
            project::MilestoneStatus::Completed
        );
        assert_eq!(
            review.milestones[0].review_status.as_deref(),
            Some("pending_review")
        );
        assert_eq!(review.workflow_state.review_node_id, "milestone-1");
        assert_eq!(
            review
                .workflow_state
                .autopilot_state
                .as_ref()
                .unwrap()
                .run_status,
            project::AutopilotRunStatus::WaitingMilestoneReview
        );
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_review_entry_rejects_profile_topology_mismatch(
    ) -> Result<(), String> {
        let project_name = unique_project_name("review-topology-mismatch");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = quick_review_project(&project_name, project::SubtaskStatus::Passed);
        proj.workload_profile = Some(system_workload_profile());
        crate::save_project(&proj)?;

        let error = crate::commands::milestone::enter_milestone_review(project_name)
            .await
            .expect_err("审阅入口必须拒绝与画像不一致的拓扑");
        assert!(error.contains("拓扑与工作负载画像矛盾"));
        Ok(())
    }

    fn managed_milestone_project(
        project_name: &str,
        draft_status: project::MilestoneDraftStatus,
        run_status: project::ManagedRunStatus,
    ) -> project::Project {
        let mut proj = project::Project::new(project_name);
        proj.workload_profile = Some(professional_workload_profile());
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneApproval;
        proj.workflow_state.managed_flow_state = Some(project::ManagedFlowState {
            active: true,
            managed_state: "MilestoneApproval".to_string(),
            managed_target: "MilestoneApproval".to_string(),
            last_action: "托管层已暂停".to_string(),
            last_action_at: "2026-07-22T00:00:00Z".to_string(),
            run_status,
            error_message: String::new(),
            ..Default::default()
        });
        proj.milestone_draft = Some(project::MilestoneDraft {
            status: draft_status,
            check_result: Some("检查通过".to_string()),
            candidate_milestones: vec![test_milestone(
                "milestone-1",
                "测试大阶段",
                project::MilestoneStatus::Pending,
            )],
            ..Default::default()
        });
        proj
    }

    #[tokio::test]
    async fn adaptive_execution_contract_quick_reaches_execution_and_review() -> Result<(), String>
    {
        let project_name = unique_project_name("quick-topology-contract");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let workspace = TestGitWorkspace::new("quick-topology")?;
        let mut proj = project::Project::new(&project_name);
        proj.project_path = workspace.path.to_string_lossy().to_string();
        proj.workload_profile = Some(quick_workload_profile());
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
        let mut milestone =
            test_milestone("milestone-1", "静态网页", project::MilestoneStatus::Pending);
        milestone.mode = project::StageMode::Quick;
        proj.milestones = vec![milestone];
        crate::save_project(&proj)?;

        let selected = crate::commands::milestone::select_milestone(
            project_name.clone(),
            "milestone-1".to_string(),
        )
        .await?;
        assert!(selected.current_mid_stage_id.is_empty());
        let planning =
            crate::commands::milestone::continue_current_milestone(project_name.clone()).await?;
        assert_eq!(
            planning.workflow_state.current_step,
            project::WorkflowStep::PlanGeneration
        );

        let mut seeded = crate::load_project(&project_name)?;
        let milestone = seeded
            .milestones
            .iter_mut()
            .find(|milestone| milestone.id == "milestone-1")
            .ok_or("Quick 夹具大阶段丢失".to_string())?;
        milestone.subtasks = vec![test_subtask(project::SubtaskStatus::Pending)];
        milestone.plan_generated_at = Some("2026-08-06T00:00:00Z".to_string());
        milestone.plan_draft_revision = 1;
        milestone.plan_check_result = Some(project::StagePlanCheckResult {
            passed: true,
            omissions: vec![],
            out_of_scope: vec![],
            not_executable: vec![],
            suggestions: vec![],
            checked_at: "2026-08-06T00:00:01Z".to_string(),
        });
        crate::save_project(&seeded)?;
        transition_workflow(
            project_name.clone(),
            "PlanCheck".to_string(),
            "test: Quick 计划已生成".to_string(),
        )
        .await?;
        transition_workflow(
            project_name.clone(),
            "PlanApproving".to_string(),
            "test: Quick 计划检查通过".to_string(),
        )
        .await?;
        let approved = crate::commands::milestone::approve_stage_plan(project_name.clone()).await?;
        assert_eq!(
            approved.workflow_state.current_step,
            project::WorkflowStep::Execution
        );
        assert!(approved.current_mid_stage_id.is_empty());
        assert_eq!(approved.milestones[0].mid_stages.len(), 0);
        assert!(approved.milestones[0].plan_revision > 0);

        let mut active = approved;
        activate_autopilot(&mut active, "milestone-1");
        crate::save_project(&active)?;
        let serial = autopilot_next_step(project_name.clone()).await?;
        assert_eq!(serial.command, "execute_control_action");
        assert_eq!(serial.args["request"]["task_id"], "subtask-1");

        let mut completed = crate::load_project(&project_name)?;
        completed.milestones[0].subtasks[0].status = project::SubtaskStatus::Passed;
        crate::save_project(&completed)?;
        let advance = autopilot_next_step(project_name.clone()).await?;
        assert_eq!(advance.command, "transition_workflow");
        assert_eq!(advance.args["targetStep"], "MilestoneReview");

        let mut completed = crate::load_project(&project_name)?;
        assert_eq!(
            crate::pipeline::reconcile_terminal_stage(&mut completed, "milestone-1", "")?,
            (true, true)
        );
        crate::save_project(&completed)?;
        let review = autopilot_next_step(project_name).await?;
        assert!(review.at_milestone_boundary);
        assert!(!review.is_error);
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_system_keeps_mid_stage_and_reaches_review(
    ) -> Result<(), String> {
        let project_name = unique_project_name("system-topology-contract");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let workspace = TestGitWorkspace::new("system-topology")?;
        let mut proj = project::Project::new(&project_name);
        proj.project_path = workspace.path.to_string_lossy().to_string();
        proj.workload_profile = Some(system_workload_profile());
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
        let mut milestone =
            test_milestone("milestone-1", "全栈系统", project::MilestoneStatus::Pending);
        let mut mid_stage = test_mid_stage(project::MidStageStatus::Ready);
        mid_stage.subtasks.clear();
        milestone.mid_stages = vec![mid_stage];
        proj.milestones = vec![milestone];
        crate::save_project(&proj)?;

        crate::commands::milestone::select_milestone(
            project_name.clone(),
            "milestone-1".to_string(),
        )
        .await?;
        let planning =
            crate::commands::milestone::continue_current_milestone(project_name.clone()).await?;
        assert_eq!(planning.current_mid_stage_id, "mid-1");
        assert_eq!(
            planning.workflow_state.current_step,
            project::WorkflowStep::PlanGeneration
        );

        let mut seeded = crate::load_project(&project_name)?;
        let mid_stage = &mut seeded.milestones[0].mid_stages[0];
        mid_stage.subtasks = vec![test_subtask(project::SubtaskStatus::Pending)];
        mid_stage.plan_generated_at = Some("2026-08-06T00:00:00Z".to_string());
        mid_stage.plan_draft_revision = 1;
        mid_stage.plan_check_result = Some(project::StagePlanCheckResult {
            passed: true,
            omissions: vec![],
            out_of_scope: vec![],
            not_executable: vec![],
            suggestions: vec![],
            checked_at: "2026-08-06T00:00:01Z".to_string(),
        });
        crate::save_project(&seeded)?;
        transition_workflow(
            project_name.clone(),
            "PlanCheck".to_string(),
            "test: System 计划已生成".to_string(),
        )
        .await?;
        transition_workflow(
            project_name.clone(),
            "PlanApproving".to_string(),
            "test: System 计划检查通过".to_string(),
        )
        .await?;
        let approved = crate::commands::milestone::approve_stage_plan(project_name.clone()).await?;
        assert_eq!(
            approved.workflow_state.current_step,
            project::WorkflowStep::Execution
        );
        assert_eq!(approved.current_mid_stage_id, "mid-1");
        assert!(approved.milestones[0].subtasks.is_empty());

        let mut completed = approved;
        activate_autopilot(&mut completed, "milestone-1");
        completed.milestones[0].mid_stages[0].subtasks[0].status = project::SubtaskStatus::Passed;
        assert_eq!(
            crate::pipeline::reconcile_terminal_stage(&mut completed, "milestone-1", "mid-1",)?,
            (true, true)
        );
        crate::save_project(&completed)?;
        let review = autopilot_next_step(project_name).await?;
        assert!(review.at_milestone_boundary);
        assert!(!review.is_error);
        Ok(())
    }

    fn managed_milestone_check_project(
        project_name: &str,
        draft_status: project::MilestoneDraftStatus,
    ) -> project::Project {
        let mut proj = managed_milestone_project(
            project_name,
            draft_status,
            project::ManagedRunStatus::Running,
        );
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneCheck;
        if let Some(managed) = proj.workflow_state.managed_flow_state.as_mut() {
            managed.managed_state = "MilestoneCheck".to_string();
            managed.managed_target = "MilestoneSelection".to_string();
        }
        proj
    }

    fn active_managed_plan_project(
        project_name: &str,
        step: project::WorkflowStep,
    ) -> project::Project {
        let mut proj = project::Project::new(project_name);
        proj.workload_profile = Some(
            crate::workload_policy::classify(
                project::WorkloadSignals {
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
                proj.discussion_revision,
            )
            .expect("managed plan fixture profile"),
        );
        proj.workflow_state.top_level_phase = project::TopLevelPhase::FirstDiscussion;
        proj.workflow_state.current_step = step;
        proj.workflow_state.managed_flow_state = Some(project::ManagedFlowState {
            active: true,
            managed_state: "plan".to_string(),
            managed_target: "MilestoneSelection".to_string(),
            last_action: String::new(),
            last_action_at: String::new(),
            run_status: project::ManagedRunStatus::Running,
            error_message: String::new(),
            ..Default::default()
        });
        proj
    }

    #[tokio::test]
    async fn managed_legacy_empty_plan_approval_generates_instead_of_waiting() -> Result<(), String>
    {
        let project_name = unique_project_name("managed-empty-plan");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let proj = active_managed_plan_project(&project_name, project::WorkflowStep::PlanApproval);
        crate::save_project(&proj)?;

        let next = managed_next_step(project_name.clone()).await?;
        assert_eq!(next.command, "generate_version_plan");
        assert!(!next.needs_human);
        assert_eq!(
            crate::load_project(&project_name)?
                .workflow_state
                .current_step,
            project::WorkflowStep::ProjectPlanGeneration
        );
        Ok(())
    }

    #[tokio::test]
    async fn managed_generation_reuses_existing_valid_plan_draft() -> Result<(), String> {
        let project_name = unique_project_name("managed-existing-plan");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = active_managed_plan_project(
            &project_name,
            project::WorkflowStep::ProjectPlanGeneration,
        );
        proj.plan_draft = Some(project::PlanDraft {
            plan_content: "project plan".to_string(),
            constitution_part1_draft: "constitution".to_string(),
            generation_revision: proj.discussion_revision,
            workload_profile_fingerprint: proj
                .workload_profile
                .as_ref()
                .expect("profile")
                .fingerprint
                .clone(),
            ..Default::default()
        });
        for check_type in [
            "goal_completeness",
            "reality_consistency",
            "task_executability",
        ] {
            proj.preflight_results.push(project::PreflightCheckResult {
                check_type: check_type.to_string(),
                passed: true,
                summary: String::new(),
                issues: vec![],
                suggestions: vec![],
                discussion_revision: proj.discussion_revision,
                checked_at: String::new(),
                stale: false,
                expired_at: None,
            });
        }
        crate::save_project(&proj)?;

        let next = managed_next_step(project_name.clone()).await?;
        assert_eq!(next.command, "approve_version_plan");
        assert!(!next.needs_human);
        assert_eq!(
            crate::load_project(&project_name)?
                .workflow_state
                .current_step,
            project::WorkflowStep::PlanApproval
        );
        Ok(())
    }

    #[tokio::test]
    async fn workflow_closure_e2e_managed_reaches_milestone_selection_from_seeded_results(
    ) -> Result<(), String> {
        let project_name = unique_project_name("managed-closure-e2e");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj =
            active_managed_plan_project(&project_name, project::WorkflowStep::ThreeChecks);
        for check_type in [
            "goal_completeness",
            "reality_consistency",
            "task_executability",
        ] {
            proj.preflight_results.push(project::PreflightCheckResult {
                check_type: check_type.to_string(),
                passed: true,
                summary: String::new(),
                issues: vec![],
                suggestions: vec![],
                discussion_revision: proj.discussion_revision,
                checked_at: String::new(),
                stale: false,
                expired_at: None,
            });
        }
        crate::save_project(&proj)?;

        let generate_plan = managed_next_step(project_name.clone()).await?;
        assert_eq!(generate_plan.command, "generate_version_plan");

        let mut with_plan = crate::load_project(&project_name)?;
        with_plan.plan_draft = Some(project::PlanDraft {
            plan_content: "可执行项目方案".to_string(),
            constitution_part1_draft: "项目约束".to_string(),
            generation_revision: with_plan.discussion_revision,
            workload_profile_fingerprint: with_plan
                .workload_profile
                .as_ref()
                .expect("profile")
                .fingerprint
                .clone(),
            ..Default::default()
        });
        crate::save_project(&with_plan)?;
        let approve_plan = managed_next_step(project_name.clone()).await?;
        assert_eq!(approve_plan.command, "approve_version_plan");

        let mut approved_plan = crate::load_project(&project_name)?;
        let draft = approved_plan
            .plan_draft
            .as_mut()
            .ok_or_else(|| "方案草稿缺失".to_string())?;
        draft.draft_status = project::DraftStatus::Approved;
        draft.approved = true;
        draft.approved_at = Some(chrono::Utc::now().to_rfc3339());
        approved_plan.version_plan = draft.plan_content.clone();
        crate::save_project(&approved_plan)?;
        let enter_console = managed_next_step(project_name.clone()).await?;
        assert_eq!(enter_console.command, "enter_console");

        let mut milestone_generation = crate::load_project(&project_name)?;
        milestone_generation.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        milestone_generation.workflow_state.current_step =
            project::WorkflowStep::MilestoneGeneration;
        crate::save_project(&milestone_generation)?;
        let generate_milestone = managed_next_step(project_name.clone()).await?;
        assert_eq!(generate_milestone.command, "generate_milestone_draft");

        let mut milestone_approval = crate::load_project(&project_name)?;
        milestone_approval.workflow_state.current_step = project::WorkflowStep::MilestoneApproval;
        milestone_approval.milestone_draft = Some(project::MilestoneDraft {
            status: project::MilestoneDraftStatus::CheckPassed,
            check_result: Some("检查通过".to_string()),
            candidate_milestones: vec![test_milestone(
                "milestone-1",
                "首个大阶段",
                project::MilestoneStatus::Pending,
            )],
            ..Default::default()
        });
        crate::save_project(&milestone_approval)?;
        let approve_milestone = managed_next_step(project_name.clone()).await?;
        assert_eq!(approve_milestone.command, "approve_milestone_draft");

        let completed = crate::commands::milestone::approve_milestone_draft(project_name).await?;
        assert_eq!(
            completed.workflow_state.current_step,
            project::WorkflowStep::MilestoneSelection
        );
        assert!(completed.workflow_state.managed_flow_state.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn managed_milestone_check_routes_pending_and_failed_drafts() -> Result<(), String> {
        let project_name = unique_project_name("managed-milestone-check-route");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj =
            managed_milestone_check_project(&project_name, project::MilestoneDraftStatus::Pending);
        let draft = proj
            .milestone_draft
            .as_mut()
            .ok_or_else(|| "缺少大阶段草稿".to_string())?;
        draft.check_result = None;
        let draft_id = draft.draft_id.clone();
        crate::save_project(&proj)?;

        let check = managed_next_step(project_name.clone()).await?;
        assert_eq!(check.command, "check_milestone_draft");
        assert!(!check.needs_human);

        let mut failed = crate::load_project(&project_name)?;
        let draft = failed
            .milestone_draft
            .as_mut()
            .ok_or_else(|| "缺少大阶段草稿".to_string())?;
        draft.status = project::MilestoneDraftStatus::CheckFailed;
        draft.check_result = Some("缺少验收边界".to_string());
        crate::save_project(&failed)?;

        let regenerate = managed_next_step(project_name).await?;
        assert_eq!(regenerate.command, "regenerate_milestone_draft");
        assert!(!regenerate.needs_human);
        assert_eq!(regenerate.args["currentDraftId"], draft_id);
        assert_eq!(regenerate.args["feedback"], "缺少验收边界");
        assert_eq!(regenerate.args["source"], "check_failed");
        Ok(())
    }

    #[tokio::test]
    async fn managed_milestone_check_stops_on_repeated_feedback() -> Result<(), String> {
        let project_name = unique_project_name("managed-milestone-check-repeat");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = managed_milestone_check_project(
            &project_name,
            project::MilestoneDraftStatus::CheckFailed,
        );
        let draft = proj
            .milestone_draft
            .as_mut()
            .ok_or_else(|| "缺少大阶段草稿".to_string())?;
        draft.check_result = Some("缺少 验收边界".to_string());
        draft.regeneration_count = 1;
        draft.last_regeneration_reason = Some("  缺少   验收边界  ".to_string());
        crate::save_project(&proj)?;

        let next = managed_next_step(project_name).await?;
        assert!(next.command.is_empty());
        assert!(next.needs_human);
        assert!(next.description.contains("相同检查问题"));
        Ok(())
    }

    #[tokio::test]
    async fn managed_milestone_check_stops_at_regeneration_limit() -> Result<(), String> {
        let project_name = unique_project_name("managed-milestone-check-limit");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = managed_milestone_check_project(
            &project_name,
            project::MilestoneDraftStatus::CheckFailed,
        );
        let draft = proj
            .milestone_draft
            .as_mut()
            .ok_or_else(|| "缺少大阶段草稿".to_string())?;
        draft.check_result = Some("仍有范围遗漏".to_string());
        draft.regeneration_count = crate::autopilot_policy::MAX_PLANNING_REGENERATIONS;
        draft.last_regeneration_reason = Some("上一次是其他问题".to_string());
        crate::save_project(&proj)?;

        let next = managed_next_step(project_name).await?;
        assert!(next.command.is_empty());
        assert!(next.needs_human);
        assert!(next.description.contains("两次上限"));
        Ok(())
    }

    #[tokio::test]
    async fn managed_milestone_check_missing_feedback_waits_for_human() -> Result<(), String> {
        let project_name = unique_project_name("managed-milestone-check-no-feedback");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = managed_milestone_check_project(
            &project_name,
            project::MilestoneDraftStatus::CheckFailed,
        );
        proj.milestone_draft
            .as_mut()
            .ok_or_else(|| "缺少大阶段草稿".to_string())?
            .check_result = None;
        crate::save_project(&proj)?;

        let next = managed_next_step(project_name).await?;
        assert!(next.command.is_empty());
        assert!(next.needs_human);
        assert!(next.description.contains("缺少反馈"));
        Ok(())
    }

    #[tokio::test]
    async fn managed_milestone_reconcile_repairs_legacy_state_and_is_idempotent(
    ) -> Result<(), String> {
        let project_name = unique_project_name("managed-milestone-reconcile");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let proj = managed_milestone_project(
            &project_name,
            project::MilestoneDraftStatus::CheckFailed,
            project::ManagedRunStatus::Paused,
        );
        let initial_revision = proj.workflow_state.data_revision;
        crate::save_project(&proj)?;

        let repaired = reconcile_managed_milestone_state(project_name.clone()).await?;
        assert_eq!(
            repaired.milestone_draft.as_ref().map(|draft| &draft.status),
            Some(&project::MilestoneDraftStatus::CheckPassed)
        );
        let managed = repaired
            .workflow_state
            .managed_flow_state
            .as_ref()
            .ok_or("托管状态缺失".to_string())?;
        assert_eq!(managed.run_status, project::ManagedRunStatus::Paused);
        assert_eq!(managed.managed_target, "MilestoneSelection");
        assert_eq!(repaired.workflow_state.data_revision, initial_revision + 1);

        let repeated = reconcile_managed_milestone_state(project_name).await?;
        assert_eq!(
            repeated.workflow_state.data_revision,
            repaired.workflow_state.data_revision
        );
        Ok(())
    }

    #[tokio::test]
    async fn waiting_human_preserves_reason_and_can_resume() -> Result<(), String> {
        let project_name = unique_project_name("managed-waiting-human");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = managed_milestone_project(
            &project_name,
            project::MilestoneDraftStatus::CheckPassed,
            project::ManagedRunStatus::Running,
        );
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneCheck;
        crate::save_project(&proj)?;

        let waiting = wait_managed_flow_for_human(
            project_name.clone(),
            "候选大阶段缺失，等待人工处理".to_string(),
        )
        .await?;
        let managed = waiting
            .workflow_state
            .managed_flow_state
            .as_ref()
            .ok_or("托管状态缺失".to_string())?;
        assert_eq!(managed.run_status, project::ManagedRunStatus::WaitingHuman);
        assert_eq!(managed.last_action, "候选大阶段缺失，等待人工处理");

        let resumed = resume_managed_flow_state(project_name).await?;
        assert_eq!(
            resumed
                .workflow_state
                .managed_flow_state
                .as_ref()
                .map(|managed| &managed.run_status),
            Some(&project::ManagedRunStatus::Running)
        );
        Ok(())
    }

    #[tokio::test]
    async fn managed_start_rejects_duplicate_running_job() -> Result<(), String> {
        let project_name = unique_project_name("managed-start-running");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = active_managed_plan_project(
            &project_name,
            project::WorkflowStep::ProjectPlanGeneration,
        );
        let state = proj
            .workflow_state
            .managed_flow_state
            .as_mut()
            .ok_or("托管状态缺失".to_string())?;
        crate::managed_runtime::assign_new_job_identity(state, "已有作业");
        let job_id = state.job_id.clone();
        let generation = state.job_generation;
        let revision = proj.workflow_state.data_revision;
        crate::save_project(&proj)?;

        let error = start_managed_flow_state(project_name.clone())
            .await
            .expect_err("运行中的托管不能重复启动");
        assert!(error.contains("已在运行"));
        let stored = crate::load_project(&project_name)?;
        let state = stored
            .workflow_state
            .managed_flow_state
            .ok_or("托管状态缺失")?;
        assert_eq!(state.job_id, job_id);
        assert_eq!(state.job_generation, generation);
        assert_eq!(stored.workflow_state.data_revision, revision);
        Ok(())
    }

    #[tokio::test]
    async fn managed_start_restarts_error_stopped_with_new_job_generation() -> Result<(), String> {
        let project_name = unique_project_name("managed-start-error-stopped");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = active_managed_plan_project(
            &project_name,
            project::WorkflowStep::ProjectPlanGeneration,
        );
        let state = proj
            .workflow_state
            .managed_flow_state
            .as_mut()
            .ok_or("托管状态缺失")?;
        state.run_status = project::ManagedRunStatus::ErrorStopped;
        state.error_message = "模型连接失败".to_string();
        state.current_action = "generate_version_plan".to_string();
        state.current_action_id = "old-action".to_string();
        crate::managed_runtime::assign_new_job_identity(state, "错误停止前的代次");
        state.run_status = project::ManagedRunStatus::ErrorStopped;
        state.error_message = "模型连接失败".to_string();
        state.current_action = "generate_version_plan".to_string();
        state.current_action_id = "old-action".to_string();
        let old_job_id = state.job_id.clone();
        let old_generation = state.job_generation;
        crate::save_project(&proj)?;

        let restarted = start_managed_flow_state(project_name).await?;
        let state = restarted
            .workflow_state
            .managed_flow_state
            .ok_or("托管状态缺失")?;
        assert!(state.active);
        assert_eq!(state.run_status, project::ManagedRunStatus::Running);
        assert_ne!(state.job_id, old_job_id);
        assert_eq!(state.job_generation, old_generation + 1);
        assert!(state.error_message.is_empty());
        assert!(state.current_action.is_empty());
        assert!(state.current_action_id.is_empty());
        assert!(state.last_action.contains("显式重启"));
        Ok(())
    }

    #[tokio::test]
    async fn stopping_managed_flow_preserves_milestone_approval_step() -> Result<(), String> {
        let project_name = unique_project_name("managed-stop-approval");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let proj = managed_milestone_project(
            &project_name,
            project::MilestoneDraftStatus::CheckPassed,
            project::ManagedRunStatus::Paused,
        );
        crate::save_project(&proj)?;

        let stopped = stop_managed_flow_state(project_name).await?;
        assert_eq!(
            stopped.workflow_state.current_step,
            project::WorkflowStep::MilestoneApproval
        );
        assert!(stopped.workflow_state.managed_flow_state.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn passed_milestone_flows_from_managed_approval_to_autopilot() -> Result<(), String> {
        let project_name = unique_project_name("managed-to-autopilot");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let proj = managed_milestone_project(
            &project_name,
            project::MilestoneDraftStatus::CheckPassed,
            project::ManagedRunStatus::Running,
        );
        crate::save_project(&proj)?;

        let next = managed_next_step(project_name.clone()).await?;
        assert_eq!(next.command, "approve_milestone_draft");
        assert!(!next.needs_human);

        let approved =
            crate::commands::milestone::approve_milestone_draft(project_name.clone()).await?;
        assert_eq!(
            approved.workflow_state.current_step,
            project::WorkflowStep::MilestoneSelection
        );
        assert!(approved.workflow_state.managed_flow_state.is_none());

        let autopilot = toggle_autopilot_state(project_name, true).await?;
        assert!(autopilot.workflow_state.autopilot_active);
        assert!(autopilot.workflow_state.autopilot_state.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn managed_advisor_rejects_stale_failed_status_even_with_check_result(
    ) -> Result<(), String> {
        let project_name = unique_project_name("managed-stale-failure");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let proj = managed_milestone_project(
            &project_name,
            project::MilestoneDraftStatus::CheckFailed,
            project::ManagedRunStatus::Running,
        );
        crate::save_project(&proj)?;

        let next = managed_next_step(project_name).await?;
        assert!(next.command.is_empty());
        assert!(next.needs_human);
        assert!(!next.is_error);
        Ok(())
    }

    #[tokio::test]
    async fn stopping_managed_flow_repairs_approved_legacy_step() -> Result<(), String> {
        let project_name = unique_project_name("managed-stop-approved");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let proj = managed_milestone_project(
            &project_name,
            project::MilestoneDraftStatus::Approved,
            project::ManagedRunStatus::Paused,
        );
        crate::save_project(&proj)?;

        let stopped = stop_managed_flow_state(project_name).await?;
        assert_eq!(
            stopped.workflow_state.current_step,
            project::WorkflowStep::MilestoneSelection
        );
        assert!(stopped.workflow_state.managed_flow_state.is_none());
        Ok(())
    }

    #[test]
    fn autopilot_activation_scope_starts_at_milestone_selection() {
        let rejected = [
            project::WorkflowStep::MilestoneGeneration,
            project::WorkflowStep::MilestoneCheck,
            project::WorkflowStep::MilestoneApproval,
            project::WorkflowStep::MilestoneReview,
        ];
        assert!(rejected
            .iter()
            .all(|step| !autopilot_can_activate_from(step)));

        let accepted = [
            project::WorkflowStep::MilestoneSelection,
            project::WorkflowStep::MidStageGeneration,
            project::WorkflowStep::MidStageCheck,
            project::WorkflowStep::MidStageApproval,
            project::WorkflowStep::MidStageSelection,
            project::WorkflowStep::PlanGeneration,
            project::WorkflowStep::PlanCheck,
            project::WorkflowStep::PlanApproving,
            project::WorkflowStep::Execution,
        ];
        assert!(accepted.iter().all(autopilot_can_activate_from));
    }

    #[test]
    fn autopilot_error_truncation_preserves_unicode_boundaries() {
        let long_error = "错".repeat(AUTOPILOT_ERROR_MESSAGE_MAX_LENGTH + 2);
        let truncated = truncate_autopilot_error(&long_error);
        assert_eq!(
            truncated.chars().count(),
            AUTOPILOT_ERROR_MESSAGE_MAX_LENGTH + 3
        );
        assert!(truncated.ends_with("..."));
    }

    #[tokio::test]
    async fn autopilot_inactive_returns_error_step() -> Result<(), String> {
        let project_name = unique_project_name("ap-inactive");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workload_profile = Some(
            crate::workload_policy::classify(
                project::WorkloadSignals {
                    has_frontend: true,
                    has_backend: true,
                    has_persistence: false,
                    has_auth_or_roles: false,
                    external_integration_count: 0,
                    independent_domain_count: 3,
                    deliverable_count: 3,
                    high_risk: false,
                },
                None,
                0,
            )
            .expect("professional test profile"),
        );
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        crate::save_project(&proj)?;

        let step = autopilot_next_step(project_name).await?;
        assert!(step.is_error);
        assert_eq!(
            step.result_kind,
            project::AutopilotCommandResultKind::NoResult
        );
        Ok(())
    }

    #[tokio::test]
    async fn active_recovery_routes_to_recovery_command() -> Result<(), String> {
        let project_name = unique_project_name("ap-recovery-route");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        activate_autopilot(&mut proj, "milestone-1");
        attach_professional_execution_plan(&mut proj, project::SubtaskStatus::Executing);
        proj.workflow_state.recovery_state = Some(project::RecoveryState {
            error_kind: project::RecoveryErrorKind::TestFailure,
            phase: project::RecoveryPhase::Diagnosing,
            subtask_id: "subtask-1".to_string(),
            execution_id: "execution-1".to_string(),
            ..Default::default()
        });
        crate::save_project(&proj)?;

        let step = autopilot_next_step(project_name.clone()).await?;
        assert_eq!(step.command, "run_error_recovery");
        assert_eq!(
            step.args,
            serde_json::json!({ "projectName": project_name })
        );
        assert!(!step.is_error);
        assert!(!step.waiting_for_execution);
        assert_eq!(
            step.result_kind,
            project::AutopilotCommandResultKind::ProjectState
        );
        Ok(())
    }

    #[tokio::test]
    async fn phase1_runtime_contract_explicit_shadow_uses_legacy_and_only_audits(
    ) -> Result<(), String> {
        let project_name = unique_project_name("shadow-comparison");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        crate::save_project(&proj)?;
        let revision = proj.workflow_state.data_revision;
        proj = crate::commands::task_control::set_task_control_mode(
            project_name.clone(),
            "Shadow".to_string(),
            revision,
            Some(true),
            Some("运行时契约显式回退验证".to_string()),
            Some("phase1_runtime_contract".to_string()),
        )
        .await?;
        activate_autopilot(&mut proj, "milestone-1");
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id = "mid-1".to_string();
        let task = test_subtask(project::SubtaskStatus::Pending);
        let mut mid = test_mid_stage(project::MidStageStatus::InProgress);
        mid.subtasks = vec![task];
        mid.plan_approved_at = Some("2026-07-28T00:00:00Z".to_string());
        let mut milestone = test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages = vec![mid];
        proj.milestones = vec![milestone];
        crate::save_project(&proj)?;

        let next = autopilot_next_step(project_name.clone()).await?;
        assert_ne!(next.command, "execute_control_action");
        let saved = crate::load_project(&project_name)?;
        assert_eq!(
            saved.task_control.mode,
            crate::task_control::TaskControlMode::Shadow
        );
        assert_eq!(saved.task_control.shadow_comparison.evaluated, 1);
        assert_eq!(saved.task_control.mode_change_history.len(), 1);
        assert_eq!(
            saved.task_control.mode_change_history[0].reason,
            "运行时契约显式回退验证"
        );
        assert_eq!(
            saved.task_control.shadow_comparison.comparable_matches
                + saved.task_control.shadow_comparison.comparable_differences
                + saved.task_control.shadow_comparison.uncomparable,
            1
        );
        let comparison = saved
            .task_control
            .shadow_comparison
            .latest
            .as_ref()
            .unwrap();
        assert_eq!(comparison.legacy_command, next.command);
        assert!(saved.task_control.active_action_id.is_empty());
        assert_eq!(
            crate::task_tree::find_task(&saved, "subtask-1")?
                .unwrap()
                .status,
            project::SubtaskStatus::Pending
        );
        Ok(())
    }

    #[tokio::test]
    async fn phase1_runtime_contract_serial_takeover_dispatches_control_action(
    ) -> Result<(), String> {
        let project_name = unique_project_name("serial-takeover-action");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        activate_autopilot(&mut proj, "milestone-1");
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id = "mid-1".to_string();
        proj.task_control.mode = crate::task_control::TaskControlMode::SerialTakeover;
        let mut task = test_subtask(project::SubtaskStatus::AwaitingConfirmation);
        task.acceptance_criteria = vec!["复杂业务流程需要语义审查".to_string()];
        task.acceptance_ledger = vec![project::AcceptanceLedgerItem {
            criterion_index: 1,
            criterion: task.acceptance_criteria[0].clone(),
            ..Default::default()
        }];
        assert_eq!(
            crate::provability::infer_provability(&task.acceptance_criteria[0]),
            crate::provability::Provability::SemanticReview
        );
        let mut mid = test_mid_stage(project::MidStageStatus::InProgress);
        mid.subtasks = vec![task];
        mid.plan_approved_at = Some("2026-07-28T00:00:00Z".to_string());
        let mut milestone = test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages = vec![mid];
        proj.milestones = vec![milestone];
        crate::save_project(&proj)?;

        let next = autopilot_next_step(project_name.clone()).await?;
        assert_eq!(next.command, "execute_control_action");
        let request =
            serde_json::from_value::<crate::control_action_executor::ControlActionRequest>(
                next.args["request"].clone(),
            )
            .map_err(|error| error.to_string())?;
        assert_eq!(request.task_id, "subtask-1");
        assert_eq!(
            request.action,
            crate::control_action::ControlActionKind::TargetedValidate
        );
        let saved = crate::load_project(&project_name)?;
        assert_eq!(saved.task_control.control_source, "task_controller");
        assert_eq!(request.decision_id, saved.task_control.last_decision_id);
        Ok(())
    }

    #[tokio::test]
    async fn phase1_runtime_contract_serial_without_leaf_allows_only_macro_stage_progression(
    ) -> Result<(), String> {
        let project_name = unique_project_name("serial-macro-progression");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        activate_autopilot(&mut proj, "milestone-1");
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id = "mid-1".to_string();
        proj.task_control.mode = crate::task_control::TaskControlMode::SerialTakeover;

        let mut completed_child = test_subtask(project::SubtaskStatus::Passed);
        completed_child.id = "leaf-completed".to_string();
        let mut completed_parent = test_subtask(project::SubtaskStatus::Passed);
        completed_parent.id = "parent-completed".to_string();
        completed_parent.child_tasks = vec![completed_child];
        let mut current = test_mid_stage(project::MidStageStatus::Completed);
        current.subtasks = vec![completed_parent];
        current.plan_approved_at = Some("2026-07-31T00:00:00Z".to_string());

        let mut next_mid = test_mid_stage(project::MidStageStatus::Pending);
        next_mid.id = "mid-2".to_string();
        next_mid.title = "下一中阶段".to_string();
        let mut milestone = test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages = vec![current, next_mid];
        proj.milestones = vec![milestone];
        crate::save_project(&proj)?;

        let next = autopilot_next_step(project_name.clone()).await?;
        assert_eq!(next.command, "select_mid_stage");
        assert_eq!(next.args["midStageId"], "mid-2");
        assert_ne!(next.command, "execute_current_subtask");

        let saved = crate::load_project(&project_name)?;
        assert_eq!(
            crate::task_tree::find_task(&saved, "parent-completed")?
                .ok_or("完成父任务丢失".to_string())?
                .status,
            project::SubtaskStatus::Passed
        );
        assert!(saved.execution_session.is_none());
        assert!(saved.task_control.active_action_id.is_empty());
        Ok(())
    }

    #[test]
    fn phase1_runtime_contract_serial_macro_fallback_rejects_legacy_task_commands() {
        for command in [
            "execute_current_subtask",
            "calibrate_next_subtask_command",
            "confirm_subtask_result",
            "run_error_recovery",
            "retry_current_subtask",
        ] {
            assert!(
                !serial_takeover_allows_macro_fallback(command),
                "旧任务级命令不得从串行接管回落：{}",
                command
            );
        }
        assert!(serial_takeover_allows_macro_fallback("select_mid_stage"));
        assert!(serial_takeover_allows_macro_fallback("transition_workflow"));
        assert!(serial_takeover_allows_macro_fallback(""));
    }

    #[tokio::test]
    async fn phase1_runtime_contract_serial_without_leaf_rejects_legacy_parent_execution(
    ) -> Result<(), String> {
        let project_name = unique_project_name("serial-parent-reexecution");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        activate_autopilot(&mut proj, "milestone-1");
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id = "mid-1".to_string();
        proj.task_control.mode = crate::task_control::TaskControlMode::SerialTakeover;
        proj.project_path = std::env::temp_dir().to_string_lossy().to_string();

        let mut completed_child = test_subtask(project::SubtaskStatus::Passed);
        completed_child.id = "leaf-completed".to_string();
        let mut pending_parent = test_subtask(project::SubtaskStatus::Pending);
        pending_parent.id = "parent-awaiting-aggregation".to_string();
        pending_parent.allowed_file_paths.clear();
        pending_parent.fact_snapshot = Some(crate::project_facts::capture(
            &proj.project_path,
            &[],
            Vec::new(),
        )?);
        pending_parent.child_tasks = vec![completed_child];
        let mut mid = test_mid_stage(project::MidStageStatus::InProgress);
        mid.subtasks = vec![pending_parent];
        mid.plan_approved_at = Some("2026-07-31T00:00:00Z".to_string());
        let mut milestone = test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages = vec![mid];
        proj.milestones = vec![milestone];
        crate::save_project(&proj)?;

        let next = autopilot_next_step(project_name.clone()).await?;
        assert!(next.command.is_empty());
        assert!(next.is_error);
        assert!(!next.error_message.is_empty());
        assert_ne!(next.command, "execute_current_subtask");

        let saved = crate::load_project(&project_name)?;
        assert_eq!(
            crate::task_tree::find_task(&saved, "parent-awaiting-aggregation")?
                .ok_or("待聚合父任务丢失".to_string())?
                .status,
            project::SubtaskStatus::Pending
        );
        assert_eq!(
            crate::task_tree::find_task(&saved, "leaf-completed")?
                .ok_or("完成叶子丢失".to_string())?
                .status,
            project::SubtaskStatus::Passed
        );
        assert!(saved.execution_session.is_none());
        assert_eq!(
            saved
                .workflow_state
                .autopilot_state
                .as_ref()
                .ok_or("缺少自动驾驶停止状态".to_string())?
                .run_status,
            project::AutopilotRunStatus::ErrorStopped
        );
        Ok(())
    }

    #[tokio::test]
    async fn validation_retry_is_dispatched_only_after_deadline() -> Result<(), String> {
        let project_name = unique_project_name("ap-validation-retry-deadline");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        activate_autopilot(&mut proj, "milestone-1");
        attach_professional_execution_plan(&mut proj, project::SubtaskStatus::Executing);
        proj.execution_session = Some(project::ExecutionSession {
            execution_id: "review-retry-1".to_string(),
            active: true,
            milestone_id: "milestone-1".to_string(),
            mid_stage_id: "mid-1".to_string(),
            subtask_id: "subtask-1".to_string(),
            status: "recovering".to_string(),
            ..Default::default()
        });
        proj.workflow_state.recovery_state = Some(project::RecoveryState {
            error_kind: project::RecoveryErrorKind::ReviewTransientFailure,
            phase: project::RecoveryPhase::Retesting,
            subtask_id: "subtask-1".to_string(),
            execution_id: "review-retry-1".to_string(),
            validation_retry_count: 1,
            max_validation_retries: 3,
            next_validation_retry_at: Some("2099-01-01T00:00:00Z".to_string()),
            ..Default::default()
        });
        crate::save_project(&proj)?;

        let waiting = autopilot_next_step(project_name.clone()).await?;
        assert!(waiting.waiting_for_execution);
        assert!(waiting.command.is_empty());
        assert!(waiting.description.contains("2/3"));

        let mut due = crate::load_project(&project_name)?;
        due.workflow_state
            .recovery_state
            .as_mut()
            .unwrap()
            .next_validation_retry_at = Some("2020-01-01T00:00:00Z".to_string());
        crate::save_project(&due)?;
        let ready = autopilot_next_step(project_name.clone()).await?;
        assert_eq!(ready.command, "run_error_recovery");

        let mut claimed = crate::load_project(&project_name)?;
        let state = claimed.workflow_state.autopilot_state.as_mut().unwrap();
        state.current_action_id = "current-recovery-action".to_string();
        state.current_action_kind = "run_error_recovery".to_string();
        crate::save_project(&claimed)?;
        let in_flight = autopilot_next_step(project_name).await?;
        assert!(in_flight.waiting_for_execution);
        assert!(in_flight.command.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn toggle_autopilot_requires_console_phase() -> Result<(), String> {
        let project_name = unique_project_name("ap-phase");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let proj = project::Project::new(&project_name);
        crate::save_project(&proj)?;
        let result = toggle_autopilot_state(project_name, true).await;
        assert!(result.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn toggle_autopilot_prefers_selected_incomplete_milestone() -> Result<(), String> {
        let project_name = unique_project_name("ap-target");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workload_profile = Some(professional_workload_profile());
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
        proj.current_milestone_id = "milestone-2".to_string();
        proj.milestones = vec![
            test_milestone(
                "milestone-1",
                "第一个未完成阶段",
                project::MilestoneStatus::Pending,
            ),
            test_milestone(
                "milestone-2",
                "用户已选阶段",
                project::MilestoneStatus::InProgress,
            ),
        ];
        crate::save_project(&proj)?;

        let updated = toggle_autopilot_state(project_name, true).await?;
        assert_eq!(
            updated.workflow_state.autopilot_target_milestone_id,
            "milestone-2"
        );
        let autopilot = updated
            .workflow_state
            .autopilot_state
            .as_ref()
            .ok_or("激活后缺少自动驾驶状态".to_string())?;
        assert_eq!(autopilot.target_milestone_id, "milestone-2");
        assert_eq!(autopilot.run_status, project::AutopilotRunStatus::Running);
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_toggle_autopilot_requires_fresh_profile(
    ) -> Result<(), String> {
        let missing_name = unique_project_name("ap-missing-profile");
        let _missing_guard = ProjectDataGuard::new(&missing_name)?;
        let mut missing = quick_review_project(&missing_name, project::SubtaskStatus::Pending);
        missing.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
        missing.workload_profile = None;
        crate::save_project(&missing)?;
        let missing_error = toggle_autopilot_state(missing_name, true)
            .await
            .expect_err("缺失画像不得激活 autopilot");
        assert!(missing_error.contains("重新完成目标完整性检查"));

        let stale_name = unique_project_name("ap-stale-profile");
        let _stale_guard = ProjectDataGuard::new(&stale_name)?;
        let mut stale = quick_review_project(&stale_name, project::SubtaskStatus::Pending);
        stale.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
        stale.discussion_revision = 1;
        crate::save_project(&stale)?;
        let stale_error = toggle_autopilot_state(stale_name, true)
            .await
            .expect_err("过期画像不得激活 autopilot");
        assert!(stale_error.contains("重新完成目标完整性检查"));
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_running_autopilot_blocks_without_fresh_profile(
    ) -> Result<(), String> {
        for stale in [false, true] {
            let project_name = unique_project_name(if stale {
                "ap-running-stale-profile"
            } else {
                "ap-running-missing-profile"
            });
            let _guard = ProjectDataGuard::new(&project_name)?;
            let mut proj = quick_review_project(&project_name, project::SubtaskStatus::Pending);
            proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
            activate_autopilot(&mut proj, "milestone-1");
            if stale {
                proj.discussion_revision = 1;
            } else {
                proj.workload_profile = None;
            }
            crate::save_project(&proj)?;

            let decision = autopilot_next_step(project_name.clone()).await?;
            assert!(decision.command.is_empty());
            assert!(decision.is_error);
            assert!(decision.error_message.contains("重新完成目标完整性检查"));
            let persisted = crate::load_project(&project_name)?;
            assert_eq!(
                persisted.workflow_state.current_step,
                project::WorkflowStep::MilestoneSelection
            );
            assert_eq!(
                persisted
                    .workflow_state
                    .autopilot_state
                    .as_ref()
                    .unwrap()
                    .run_status,
                project::AutopilotRunStatus::ErrorStopped
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn autopilot_terminal_error_is_persisted() -> Result<(), String> {
        let project_name = unique_project_name("ap-terminal");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.current_step = project::WorkflowStep::BranchDiscussion;
        proj.milestones.push(test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        ));
        proj.current_milestone_id = "milestone-1".to_string();
        activate_autopilot(&mut proj, "milestone-1");
        crate::save_project(&proj)?;

        let step = autopilot_next_step(project_name.clone()).await?;
        assert!(step.command.is_empty());
        assert!(step.is_error);
        let persisted = crate::load_project(&project_name)?;
        let autopilot = persisted
            .workflow_state
            .autopilot_state
            .as_ref()
            .ok_or("终止结果没有持久化自动驾驶状态".to_string())?;
        assert_eq!(
            autopilot.run_status,
            project::AutopilotRunStatus::ErrorStopped
        );
        assert!(!autopilot.error_message.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn autopilot_review_boundary_is_persisted() -> Result<(), String> {
        let project_name = unique_project_name("ap-review");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneReview;
        proj.milestones.push(test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::Completed,
        ));
        proj.current_milestone_id = "milestone-1".to_string();
        activate_autopilot(&mut proj, "milestone-1");
        crate::save_project(&proj)?;

        let step = autopilot_next_step(project_name.clone()).await?;
        assert!(step.at_milestone_boundary);
        assert!(!step.is_error);
        let persisted = crate::load_project(&project_name)?;
        let autopilot = persisted
            .workflow_state
            .autopilot_state
            .as_ref()
            .ok_or("审阅边界没有持久化自动驾驶状态".to_string())?;
        assert_eq!(
            autopilot.run_status,
            project::AutopilotRunStatus::WaitingMilestoneReview
        );
        Ok(())
    }

    #[tokio::test]
    async fn missing_target_and_rejected_subtask_persist_error_stopped() -> Result<(), String> {
        let missing_name = unique_project_name("ap-missing-target");
        let _missing_guard = ProjectDataGuard::new(&missing_name)?;
        let mut missing = project::Project::new(&missing_name);
        missing.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
        activate_autopilot(&mut missing, "missing-milestone");
        crate::save_project(&missing)?;
        let missing_step = autopilot_next_step(missing_name.clone()).await?;
        assert!(missing_step.is_error);
        let persisted_missing = crate::load_project(&missing_name)?;
        assert_eq!(
            persisted_missing
                .workflow_state
                .autopilot_state
                .as_ref()
                .ok_or("缺失目标未持久化自动驾驶状态".to_string())?
                .run_status,
            project::AutopilotRunStatus::ErrorStopped
        );

        let rejected_name = unique_project_name("ap-rejected");
        let _rejected_guard = ProjectDataGuard::new(&rejected_name)?;
        let mut rejected = project::Project::new(&rejected_name);
        rejected.workflow_state.current_step = project::WorkflowStep::Execution;
        rejected.current_milestone_id = "milestone-1".to_string();
        rejected.current_mid_stage_id = "mid-1".to_string();
        let mut mid_stage = test_mid_stage(project::MidStageStatus::InProgress);
        mid_stage.subtasks = vec![test_subtask(project::SubtaskStatus::Rejected)];
        mid_stage.plan_generated_at = Some("2026-08-01T00:00:00Z".to_string());
        mid_stage.plan_approved_at = Some("2026-08-01T00:00:00Z".to_string());
        mid_stage.plan_revision = 1;
        let mut milestone = test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages = vec![mid_stage];
        rejected.milestones = vec![milestone];
        activate_autopilot(&mut rejected, "milestone-1");
        rejected.task_control.mode = crate::task_control::TaskControlMode::Legacy;
        crate::save_project(&rejected)?;
        let rejected_step = autopilot_next_step(rejected_name.clone()).await?;
        assert!(rejected_step.is_error);
        assert!(rejected_step.command.is_empty());
        assert_eq!(
            rejected_step.result_kind,
            project::AutopilotCommandResultKind::NoResult
        );
        let persisted_rejected = crate::load_project(&rejected_name)?;
        let rejected_ap = persisted_rejected
            .workflow_state
            .autopilot_state
            .as_ref()
            .ok_or("驳回任务未持久化自动驾驶状态".to_string())?;
        assert_eq!(
            rejected_ap.run_status,
            project::AutopilotRunStatus::ErrorStopped
        );
        assert_eq!(
            rejected_ap.recovery_action,
            project::AutopilotRecoveryAction::WaitHumanDecision,
            "驳回任务不得提供必然失败的重新推进"
        );

        // 人工介入步骤 → WaitHumanDecision，不得 RetryAutopilotAdvance
        let human_name = unique_project_name("ap-human-step");
        let _human_guard = ProjectDataGuard::new(&human_name)?;
        let mut human = project::Project::new(&human_name);
        human.workflow_state.current_step = project::WorkflowStep::Discussion;
        activate_autopilot(&mut human, "milestone-1");
        crate::save_project(&human)?;
        let human_step = autopilot_next_step(human_name.clone()).await?;
        assert!(human_step.is_error);
        let persisted_human = crate::load_project(&human_name)?;
        assert_eq!(
            persisted_human
                .workflow_state
                .autopilot_state
                .as_ref()
                .ok_or("人工步骤未持久化自动驾驶状态".to_string())?
                .recovery_action,
            project::AutopilotRecoveryAction::WaitHumanDecision
        );
        Ok(())
    }

    #[tokio::test]
    async fn active_execution_session_only_returns_waiting_fact() -> Result<(), String> {
        let project_name = unique_project_name("ap-execution-wait");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id = "mid-1".to_string();
        let mut mid = test_mid_stage(project::MidStageStatus::InProgress);
        mid.subtasks = vec![test_subtask(project::SubtaskStatus::Executing)];
        let mut milestone = test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages = vec![mid];
        proj.milestones = vec![milestone];
        proj.execution_session = Some(project::ExecutionSession {
            execution_id: "execution-1".to_string(),
            active: true,
            milestone_id: "milestone-1".to_string(),
            mid_stage_id: "mid-1".to_string(),
            subtask_id: "subtask-1".to_string(),
            subtask_title: "测试小阶段".to_string(),
            status: "executing".to_string(),
            ..project::ExecutionSession::default()
        });
        activate_autopilot(&mut proj, "milestone-1");
        let revision = proj.workflow_state.data_revision;
        crate::save_project(&proj)?;

        let step = autopilot_next_step(project_name.clone()).await?;
        assert!(step.waiting_for_execution);
        assert!(step.command.is_empty());
        assert!(!step.is_error);
        let persisted = crate::load_project(&project_name)?;
        assert_eq!(persisted.workflow_state.data_revision, revision);
        assert_eq!(
            persisted
                .workflow_state
                .autopilot_state
                .as_ref()
                .ok_or("缺少自动驾驶状态".to_string())?
                .run_status,
            project::AutopilotRunStatus::Running
        );
        Ok(())
    }

    #[tokio::test]
    async fn conflicting_execution_session_requires_sync_and_close() -> Result<(), String> {
        let project_name = unique_project_name("ap-execution-conflict");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id = "mid-1".to_string();
        let mut mid = test_mid_stage(project::MidStageStatus::InProgress);
        mid.subtasks = vec![test_subtask(project::SubtaskStatus::Executing)];
        let mut milestone = test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages = vec![mid];
        proj.milestones = vec![milestone];
        proj.execution_session = Some(project::ExecutionSession {
            execution_id: "execution-conflict".to_string(),
            active: true,
            milestone_id: "milestone-1".to_string(),
            mid_stage_id: "another-mid".to_string(),
            subtask_id: "subtask-1".to_string(),
            status: "executing".to_string(),
            ..project::ExecutionSession::default()
        });
        activate_autopilot(&mut proj, "milestone-1");
        crate::save_project(&proj)?;

        let step = autopilot_next_step(project_name.clone()).await?;
        assert!(step.is_error);
        assert!(!step.waiting_for_execution);
        let persisted = crate::load_project(&project_name)?;
        let autopilot = persisted
            .workflow_state
            .autopilot_state
            .as_ref()
            .ok_or("缺少自动驾驶状态".to_string())?;
        assert_eq!(
            autopilot.run_status,
            project::AutopilotRunStatus::ErrorStopped
        );
        assert_eq!(
            autopilot.recovery_action,
            project::AutopilotRecoveryAction::SyncAndClose
        );
        Ok(())
    }

    #[tokio::test]
    async fn autopilot_resume_rejects_non_retryable_recovery_actions() -> Result<(), String> {
        for recovery_action in [
            project::AutopilotRecoveryAction::WaitHumanDecision,
            project::AutopilotRecoveryAction::SyncAndClose,
            project::AutopilotRecoveryAction::RegenerateExecutionPlan,
            project::AutopilotRecoveryAction::PrepareExecutionWorkspace,
            project::AutopilotRecoveryAction::ResolveWorkspaceChanges,
            project::AutopilotRecoveryAction::RunAutomaticRecovery,
            project::AutopilotRecoveryAction::RetryGitConfirmation,
        ] {
            let project_name = unique_project_name("ap-resume-blocked");
            let _guard = ProjectDataGuard::new(&project_name)?;
            let mut proj = project::Project::new(&project_name);
            proj.workflow_state.current_step = project::WorkflowStep::Execution;
            activate_autopilot(&mut proj, "milestone-1");
            if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
                autopilot.run_status = project::AutopilotRunStatus::ErrorStopped;
                autopilot.recovery_action = recovery_action;
            }
            crate::save_project(&proj)?;

            assert!(autopilot_resume_state(project_name).await.is_err());
        }
        Ok(())
    }

    #[tokio::test]
    async fn autopilot_mid_stage_check_failure_regenerates_then_stops_at_limit(
    ) -> Result<(), String> {
        let project_name = unique_project_name("ap-mid-stage-convergence");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::MidStageCheck;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.milestones = vec![test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        )];
        proj.mid_stage_draft = Some(project::MidStageDraft {
            draft_id: "draft-1".to_string(),
            milestone_id: "milestone-1".to_string(),
            status: project::MidStageDraftStatus::CheckFailed,
            check_result: Some("缺少验收边界".to_string()),
            regeneration_count: 0,
            purpose: project::MidStageDraftPurpose::InitialFullList,
            base_mid_stage_revision: 0,
            retained_mid_stage_ids: vec![],
            source_step: project::WorkflowStep::MidStageGeneration,
            allow_full_replacement: true,
            ..Default::default()
        });
        activate_autopilot(&mut proj, "milestone-1");
        crate::save_project(&proj)?;

        let regenerate = autopilot_next_step(project_name.clone()).await?;
        assert_eq!(regenerate.command, "regenerate_mid_stage_draft");

        let mut exhausted = crate::load_project(&project_name)?;
        exhausted
            .mid_stage_draft
            .as_mut()
            .ok_or("缺少中阶段草稿".to_string())?
            .regeneration_count = 2;
        crate::save_project(&exhausted)?;
        let stopped = autopilot_next_step(project_name.clone()).await?;
        assert!(stopped.is_error);
        assert!(stopped.command.is_empty());
        let persisted = crate::load_project(&project_name)?;
        assert_eq!(
            persisted
                .workflow_state
                .autopilot_state
                .unwrap()
                .recovery_action,
            project::AutopilotRecoveryAction::WaitHumanDecision
        );
        Ok(())
    }

    #[tokio::test]
    async fn check_convergence_plan_with_only_suggestions_skips_regeneration() -> Result<(), String>
    {
        let project_name = unique_project_name("ap-plan-suggestions");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workload_profile = Some(
            crate::workload_policy::classify(
                project::WorkloadSignals {
                    has_frontend: true,
                    has_backend: true,
                    has_persistence: false,
                    has_auth_or_roles: false,
                    external_integration_count: 0,
                    independent_domain_count: 3,
                    deliverable_count: 3,
                    high_risk: false,
                },
                None,
                0,
            )
            .expect("professional test profile"),
        );
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::PlanCheck;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id = "mid-1".to_string();
        let mut mid = test_mid_stage(project::MidStageStatus::Ready);
        mid.plan_check_result = Some(project::StagePlanCheckResult {
            passed: false,
            omissions: vec![],
            out_of_scope: vec![],
            not_executable: vec![],
            suggestions: vec!["可考虑调用 loadSearchConfig".to_string()],
            checked_at: chrono::Utc::now().to_rfc3339(),
        });
        let mut milestone = test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages = vec![mid];
        proj.milestones = vec![milestone];
        activate_autopilot(&mut proj, "milestone-1");
        crate::save_project(&proj)?;

        let decision = autopilot_next_step(project_name.clone()).await?;
        assert_eq!(decision.command, "transition_workflow");
        assert_eq!(decision.args["targetStep"], "PlanApproving");
        let transitioned = transition_workflow(
            project_name,
            "PlanApproving".to_string(),
            "test: suggestions are non-blocking".to_string(),
        )
        .await?;
        assert_eq!(
            transitioned.workflow_state.current_step,
            project::WorkflowStep::PlanApproving
        );
        assert!(transitioned.milestones[0].mid_stages[0]
            .plan_check_result
            .as_ref()
            .is_some_and(|check| check.passed));
        Ok(())
    }

    #[tokio::test]
    async fn check_convergence_plan_allows_one_no_progress_regeneration() -> Result<(), String> {
        let project_name = unique_project_name("ap-plan-one-no-progress");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::PlanCheck;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id = "mid-1".to_string();
        let mut mid = test_mid_stage(project::MidStageStatus::Ready);
        mid.plan_regeneration_count = 1;
        mid.plan_no_progress_count = 1;
        mid.plan_check_result = Some(project::StagePlanCheckResult {
            passed: true,
            omissions: vec!["缺少停止条件".to_string()],
            out_of_scope: vec![],
            not_executable: vec![],
            suggestions: vec![],
            checked_at: chrono::Utc::now().to_rfc3339(),
        });
        let mut milestone = test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages = vec![mid];
        proj.milestones = vec![milestone];
        activate_autopilot(&mut proj, "milestone-1");
        crate::save_project(&proj)?;

        let decision = autopilot_next_step(project_name).await?;
        assert_eq!(decision.command, "regenerate_execution_plan");
        Ok(())
    }

    #[tokio::test]
    async fn check_convergence_plan_stops_after_no_progress_threshold() -> Result<(), String> {
        let project_name = unique_project_name("ap-plan-convergence");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::PlanCheck;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id = "mid-1".to_string();
        let mut mid = test_mid_stage(project::MidStageStatus::Ready);
        mid.plan_regeneration_count = 1;
        mid.plan_no_progress_count = crate::autopilot_policy::MAX_PLAN_NO_PROGRESS;
        mid.plan_check_result = Some(project::StagePlanCheckResult {
            passed: false,
            omissions: vec!["缺少停止条件".to_string()],
            out_of_scope: vec![],
            not_executable: vec![],
            suggestions: vec![],
            checked_at: chrono::Utc::now().to_rfc3339(),
        });
        let mut milestone = test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages = vec![mid];
        proj.milestones = vec![milestone];
        activate_autopilot(&mut proj, "milestone-1");
        crate::save_project(&proj)?;

        let stopped = autopilot_next_step(project_name.clone()).await?;
        assert!(stopped.is_error);
        assert!(stopped.error_message.contains("没有减少硬阻断"));
        assert!(stopped.error_message.contains("缺少停止条件"));
        let persisted = crate::load_project(&project_name)?;
        assert_eq!(
            persisted.workflow_state.autopilot_state.unwrap().run_status,
            project::AutopilotRunStatus::ErrorStopped
        );
        Ok(())
    }

    #[test]
    fn workspace_without_head_uses_prepare_recovery_even_with_untracked_files() {
        let workspace = project::ExecutionWorkspaceStatus {
            path_exists: true,
            is_directory: true,
            is_git_repo: true,
            has_commits: false,
            git_user_available: true,
            git_email_available: true,
            working_tree_clean: false,
            git_metadata_ready: false,
            ready_for_new_execution: false,
            has_managed_task_changes: false,
            has_external_changes: true,
            ready: false,
            status_message: "尚无首次提交".to_string(),
            issues: vec![
                project::ExecutionWorkspaceIssue::NoCommits,
                project::ExecutionWorkspaceIssue::DirtyWorkingTree,
            ],
            changes: vec![],
        };
        assert_eq!(
            workspace_recovery_action(&workspace),
            Some(project::AutopilotRecoveryAction::PrepareExecutionWorkspace)
        );
    }

    #[tokio::test]
    async fn migration_routes_invalid_unexecuted_plan_back_to_check() -> Result<(), String> {
        let project_name = unique_project_name("invalid-plan-migration");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.current_step = project::WorkflowStep::PlanApproving;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id = "mid-1".to_string();
        let mut invalid = test_subtask(project::SubtaskStatus::Pending);
        invalid.allowed_file_paths.clear();
        let mut mid = test_mid_stage(project::MidStageStatus::Ready);
        mid.subtasks = vec![invalid];
        mid.plan_generated_at = Some("2026-07-21T00:00:00Z".to_string());
        mid.plan_approved_at = Some("2026-07-21T00:00:00Z".to_string());
        mid.plan_revision = 1;
        let mut milestone = test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages = vec![mid];
        proj.milestones = vec![milestone];
        activate_autopilot(&mut proj, "milestone-1");
        crate::save_project(&proj)?;

        let migrated = migrate_project_workflow(project_name).await?;
        let migrated_mid = &migrated.milestones[0].mid_stages[0];
        assert_eq!(
            migrated.workflow_state.current_step,
            project::WorkflowStep::PlanCheck
        );
        assert!(migrated_mid.plan_approved_at.is_none());
        assert_eq!(migrated_mid.plan_revision, 0);
        assert_eq!(
            migrated
                .workflow_state
                .autopilot_state
                .as_ref()
                .ok_or("迁移后缺少自动驾驶状态".to_string())?
                .recovery_action,
            project::AutopilotRecoveryAction::RegenerateExecutionPlan
        );
        Ok(())
    }

    #[tokio::test]
    async fn migration_preserves_invalid_plan_with_execution_facts() -> Result<(), String> {
        let project_name = unique_project_name("executed-invalid-plan-migration");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id = "mid-1".to_string();
        let mut invalid = test_subtask(project::SubtaskStatus::Passed);
        invalid.allowed_file_paths.clear();
        invalid.auto_tag = Some("metheus/auto/v0.1.1/task-1".to_string());
        let mut mid = test_mid_stage(project::MidStageStatus::InProgress);
        mid.subtasks = vec![invalid];
        mid.plan_generated_at = Some("2026-07-21T00:00:00Z".to_string());
        mid.plan_approved_at = Some("2026-07-21T00:00:00Z".to_string());
        mid.plan_revision = 1;
        let mut milestone = test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        );
        milestone.mid_stages = vec![mid];
        proj.milestones = vec![milestone];
        activate_autopilot(&mut proj, "milestone-1");
        crate::save_project(&proj)?;

        let migrated = migrate_project_workflow(project_name).await?;
        let migrated_mid = &migrated.milestones[0].mid_stages[0];
        assert!(migrated_mid.plan_approved_at.is_some());
        assert_eq!(migrated_mid.plan_revision, 1);
        assert_eq!(
            migrated
                .workflow_state
                .autopilot_state
                .as_ref()
                .ok_or("迁移后缺少自动驾驶状态".to_string())?
                .recovery_action,
            project::AutopilotRecoveryAction::WaitHumanDecision
        );
        Ok(())
    }

    #[tokio::test]
    async fn long_unicode_error_is_persisted_without_invalid_boundary() -> Result<(), String> {
        let project_name = unique_project_name("ap-unicode");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        activate_autopilot(&mut proj, "milestone-1");
        attach_professional_execution_plan(&mut proj, project::SubtaskStatus::Executing);
        crate::save_project(&proj)?;

        let long_error = "错误详情".repeat(AUTOPILOT_ERROR_MESSAGE_MAX_LENGTH);
        let updated =
            autopilot_mark_error(project_name, "自动驾驶失败".to_string(), long_error).await?;
        let autopilot = updated
            .workflow_state
            .autopilot_state
            .as_ref()
            .ok_or("长错误未持久化自动驾驶状态".to_string())?;
        let saved_error = &autopilot.error_message;
        assert_eq!(
            saved_error.chars().count(),
            AUTOPILOT_ERROR_MESSAGE_MAX_LENGTH + 3
        );
        assert!(saved_error.ends_with("..."));
        assert_eq!(
            autopilot.recovery_action,
            project::AutopilotRecoveryAction::RetryAutopilotAdvance
        );
        Ok(())
    }

    #[test]
    fn autopilot_migration_assigns_backend_identity_to_active_legacy_job() {
        let mut proj = project::Project::new("legacy-active-autopilot");
        proj.milestones = vec![test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        )];
        activate_autopilot(&mut proj, "milestone-1");
        let state = proj.workflow_state.autopilot_state.as_mut().unwrap();
        state.job_id.clear();
        state.job_generation = 0;
        state.job_owner = project::AutopilotJobOwner::None;
        state.heartbeat_at.clear();

        reconcile_autopilot_in_migration(&mut proj);

        let state = proj.workflow_state.autopilot_state.unwrap();
        assert!(!state.job_id.is_empty());
        assert_eq!(state.job_generation, 1);
        assert_eq!(state.job_owner, project::AutopilotJobOwner::BackendRuntime);
        assert!(!state.heartbeat_at.is_empty());
    }

    #[test]
    fn autopilot_migration_seeds_failed_mid_stage_convergence_fingerprints() {
        let mut proj = project::Project::new("legacy-mid-stage-failure");
        proj.milestones = vec![test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        )];
        activate_autopilot(&mut proj, "milestone-1");
        proj.mid_stage_draft = Some(project::MidStageDraft {
            status: project::MidStageDraftStatus::CheckFailed,
            check_result: Some("缺少验收边界".to_string()),
            candidate_mid_stages: vec![test_mid_stage(project::MidStageStatus::Pending)],
            purpose: project::MidStageDraftPurpose::InitialFullList,
            base_mid_stage_revision: 0,
            retained_mid_stage_ids: vec![],
            source_step: project::WorkflowStep::MidStageGeneration,
            allow_full_replacement: true,
            ..Default::default()
        });

        reconcile_autopilot_in_migration(&mut proj);

        let draft = proj.mid_stage_draft.unwrap();
        assert!(!draft.last_check_failure_fingerprint.is_empty());
        assert!(!draft.last_candidate_fingerprint.is_empty());
        assert_eq!(draft.regeneration_count, 0);
    }

    #[test]
    fn autopilot_migration_stops_legacy_plan_at_regeneration_limit() {
        let mut proj = project::Project::new("legacy-plan-limit");
        let mut milestone = test_milestone(
            "milestone-1",
            "测试大阶段",
            project::MilestoneStatus::InProgress,
        );
        let mut mid = test_mid_stage(project::MidStageStatus::Ready);
        mid.plan_regeneration_count = crate::autopilot_policy::MAX_PLANNING_REGENERATIONS;
        mid.plan_check_result = Some(project::StagePlanCheckResult {
            passed: false,
            omissions: vec!["缺少验收边界".to_string()],
            out_of_scope: vec![],
            not_executable: vec![],
            suggestions: vec![],
            checked_at: String::new(),
        });
        milestone.mid_stages.push(mid);
        proj.milestones.push(milestone);
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id = "mid-1".to_string();
        activate_autopilot(&mut proj, "milestone-1");

        reconcile_autopilot_in_migration(&mut proj);

        let state = proj.workflow_state.autopilot_state.unwrap();
        assert_eq!(state.run_status, project::AutopilotRunStatus::ErrorStopped);
        assert_eq!(
            state.recovery_action,
            project::AutopilotRecoveryAction::WaitHumanDecision
        );
        let mid = &proj.milestones[0].mid_stages[0];
        assert!(!mid.last_plan_failure_fingerprint.is_empty());
        assert_eq!(mid.last_plan_issue_count, 1);
    }
}
