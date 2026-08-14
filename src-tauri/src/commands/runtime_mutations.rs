use crate::pipeline::PipelineState;
use crate::runtime_snapshot::{
    mutation_result, RecoveryResultSummary, RuntimeActionSummary, RuntimeMutationResult,
};

fn finish(
    project_name: &str,
    pipeline: Option<PipelineState>,
    action: &str,
) -> Result<RuntimeMutationResult, String> {
    mutation_result(
        project_name,
        pipeline,
        RuntimeActionSummary::silent(action),
        false,
    )
}

fn recovery_action(
    action: &str,
    title: &str,
    message: &str,
    baseline: Option<String>,
    discarded_files: Vec<String>,
    background_job_started: bool,
    next_step: String,
) -> RuntimeActionSummary {
    let baseline_summary = baseline
        .as_ref()
        .map(|value| format!("恢复到提交：{}", value))
        .unwrap_or_default();
    let discarded_files_summary = if discarded_files.is_empty() {
        String::new()
    } else {
        format!("已丢弃 {} 个文件的执行期改动", discarded_files.len())
    };
    let background_job_summary = if background_job_started {
        "后台作业：已重新启动".to_string()
    } else {
        "后台作业：未自动启动".to_string()
    };
    let next_step_summary = format!("下一步：{}", next_step);
    RuntimeActionSummary {
        action: action.to_string(),
        message: message.to_string(),
        notify_user: true,
        recovery_result: Some(RecoveryResultSummary {
            title: title.to_string(),
            message: message.to_string(),
            baseline,
            baseline_summary,
            discarded_files,
            discarded_files_summary,
            background_job_started,
            background_job_summary,
            next_step,
            next_step_summary,
        }),
    }
}

fn background_job_active(project: &crate::project::Project) -> bool {
    project.workflow_state.autopilot_active
        && project
            .workflow_state
            .autopilot_state
            .as_ref()
            .is_some_and(|value| value.active)
}

fn managed_action(
    project_name: &str,
    pipeline: Option<PipelineState>,
    action: &str,
    background_job_started: bool,
) -> Result<RuntimeMutationResult, String> {
    let latest = crate::load_project(project_name)?;
    let Some(managed) = latest.workflow_state.managed_flow_state.as_ref() else {
        return mutation_result(
            project_name,
            pipeline,
            recovery_action(
                action,
                "托管已停止",
                "托管控制权已释放，当前没有后台托管状态。",
                None,
                Vec::new(),
                false,
                "请在人工模式下选择下一步。".to_string(),
            ),
            false,
        );
    };

    let actual_job_started = background_job_started
        && managed.active
        && managed.run_status == crate::project::ManagedRunStatus::Running;
    let (title, message, next_step) = match managed.run_status {
        crate::project::ManagedRunStatus::Running => (
            "托管已运行",
            format!(
                "托管状态为 Running，job_id={}，job_generation={}。",
                managed.job_id, managed.job_generation
            ),
            if actual_job_started {
                "后台托管作业已启动或继续运行。"
            } else {
                "后台托管作业未自动启动，请人工检查作业状态。"
            },
        ),
        crate::project::ManagedRunStatus::Paused => (
            "托管已暂停",
            "托管状态为 Paused，当前动作不会继续执行。".to_string(),
            "使用恢复托管或停止托管回到人工模式。",
        ),
        crate::project::ManagedRunStatus::WaitingHuman => (
            "托管等待人工",
            format!("{}。", managed.last_action),
            "请处理人工决策，或停止托管回到人工模式。",
        ),
        crate::project::ManagedRunStatus::ErrorStopped => (
            "托管因错误停止",
            if managed.error_message.is_empty() {
                "托管已停止，但没有新的错误摘要。".to_string()
            } else {
                format!("托管已停止：{}", managed.error_message)
            },
            "请重新启动托管、停止托管或转人工处理。",
        ),
    };

    let action_summary = recovery_action(
        action,
        title,
        &message,
        None,
        Vec::new(),
        actual_job_started,
        next_step.to_string(),
    );
    mutation_result(project_name, pipeline, action_summary, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase1_runtime_contract_recovery_result_contains_complete_backend_narrative() {
        let action = recovery_action(
            "recover",
            "恢复完成",
            "阻断已经清理。",
            Some("abc123".to_string()),
            vec!["src/app.ts".to_string()],
            true,
            "后台将继续当前任务。".to_string(),
        );
        let result = action.recovery_result.expect("恢复结果");

        assert_eq!(result.baseline_summary, "恢复到提交：abc123");
        assert_eq!(
            result.discarded_files_summary,
            "已丢弃 1 个文件的执行期改动"
        );
        assert_eq!(result.background_job_summary, "后台作业：已重新启动");
        assert_eq!(result.next_step_summary, "下一步：后台将继续当前任务。");
    }
}

#[tauri::command]
pub(crate) async fn select_milestone_runtime(
    project_name: String,
    milestone_id: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::select_milestone(project_name.clone(), milestone_id).await?;
    finish(&project_name, None, "select_milestone")
}

#[tauri::command]
pub(crate) async fn select_mid_stage_runtime(
    project_name: String,
    mid_stage_id: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::select_mid_stage(project_name.clone(), mid_stage_id).await?;
    finish(&project_name, None, "select_mid_stage")
}

#[tauri::command]
pub(crate) async fn generate_version_plan_runtime(
    project_name: String,
    expected_discussion_revision: u64,
    expected_data_revision: u64,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::plan::generate_version_plan(
        project_name.clone(),
        expected_discussion_revision,
        expected_data_revision,
    )
    .await?;
    finish(&project_name, None, "generate_version_plan")
}

#[tauri::command]
pub(crate) async fn approve_version_plan_runtime(
    project_name: String,
    draft_id: String,
    generation_revision: u64,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::plan::approve_version_plan(
        project_name.clone(),
        draft_id,
        generation_revision,
    )
    .await?;
    finish(&project_name, None, "approve_version_plan")
}

#[tauri::command]
pub(crate) async fn reject_version_plan_runtime(
    project_name: String,
    draft_id: String,
    feedback: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::plan::reject_version_plan(project_name.clone(), draft_id, feedback).await?;
    finish(&project_name, None, "reject_version_plan")
}

#[tauri::command]
pub(crate) async fn enter_console_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::plan::enter_console(project_name.clone()).await?;
    finish(&project_name, None, "enter_console")
}

#[tauri::command]
pub(crate) async fn start_preflight_check_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::workflow::start_preflight_check(project_name.clone()).await?;
    finish(&project_name, None, "start_preflight_check")
}

#[tauri::command]
pub(crate) async fn analyze_existing_project_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::project_analysis::analyze_existing_project(project_name.clone()).await?;
    finish(&project_name, None, "analyze_existing_project")
}

#[tauri::command]
pub(crate) async fn approve_existing_baseline_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::project_analysis::approve_existing_baseline(project_name.clone()).await?;
    finish(&project_name, None, "approve_existing_baseline")
}

#[tauri::command]
pub(crate) async fn update_execution_profile_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
    expected_data_revision: u64,
    execution_profile: crate::project::ExecutionProfile,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::commands::project_ops::update_execution_profile(
        state,
        project_name.clone(),
        expected_data_revision,
        execution_profile,
    )
    .await?;
    let pipeline = pipeline_state.lock().await.clone();
    finish(&project_name, pipeline, "update_execution_profile")
}

#[tauri::command]
pub(crate) async fn migrate_project_workflow_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::workflow::migrate_project_workflow(project_name.clone()).await?;
    finish(&project_name, None, "migrate_project_workflow")
}

#[tauri::command]
pub(crate) async fn reconcile_on_startup_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::pipeline::reconcile_on_startup(state, project_name.clone()).await?;
    let pipeline = pipeline_state.lock().await.clone();
    finish(&project_name, pipeline, "reconcile_on_startup")
}

#[tauri::command]
pub(crate) async fn return_to_discussion_runtime(
    project_name: String,
    source_step: String,
    reason: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::workflow::return_to_discussion(project_name.clone(), source_step, reason)
        .await?;
    finish(&project_name, None, "return_to_discussion")
}

#[tauri::command]
pub(crate) async fn resume_plan_approval_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::workflow::resume_plan_approval(project_name.clone()).await?;
    finish(&project_name, None, "resume_plan_approval")
}

#[tauri::command]
pub(crate) async fn restart_discussion_from_approved_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::workflow::restart_discussion_from_approved(project_name.clone()).await?;
    finish(&project_name, None, "restart_discussion_from_approved")
}

#[tauri::command]
pub(crate) async fn restart_checks_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::workflow::restart_checks(project_name.clone()).await?;
    finish(&project_name, None, "restart_checks")
}

#[tauri::command]
pub(crate) async fn run_preflight_check_runtime(
    project_name: String,
    check_type: String,
    frontend_discussion_revision: u64,
    frontend_data_revision: u64,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::checks::run_preflight_check(
        project_name.clone(),
        check_type,
        frontend_discussion_revision,
        frontend_data_revision,
    )
    .await?;
    finish(&project_name, None, "run_preflight_check")
}

#[tauri::command]
pub(crate) async fn reconcile_managed_milestone_state_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    let pipeline = pipeline_state.lock().await.clone();
    crate::commands::workflow::reconcile_managed_milestone_state_with_pipeline(
        &project_name,
        pipeline.as_ref(),
    )?;
    finish(&project_name, pipeline, "reconcile_managed_milestone_state")
}

#[tauri::command]
pub(crate) async fn resolve_pause_decision_runtime(
    project_name: String,
    action: String,
) -> Result<RuntimeMutationResult, String> {
    crate::pipeline::resolve_pause_decision(project_name.clone(), action).await?;
    finish(&project_name, None, "resolve_pause_decision")
}

#[tauri::command]
pub(crate) async fn confirm_rollback_runtime(
    project_name: String,
    checkpoint_subtask_id: String,
) -> Result<RuntimeMutationResult, String> {
    crate::pipeline::confirm_rollback(project_name.clone(), checkpoint_subtask_id).await?;
    finish(&project_name, None, "confirm_rollback")
}

#[tauri::command]
pub(crate) async fn regenerate_execution_plan_runtime(
    project_name: String,
    expected_data_revision: u64,
    expected_plan_draft_revision: u64,
    feedback: String,
    source: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::regenerate_execution_plan(
        project_name.clone(),
        expected_data_revision,
        expected_plan_draft_revision,
        feedback,
        source,
    )
    .await?;
    finish(&project_name, None, "regenerate_execution_plan")
}

#[tauri::command]
pub(crate) async fn generate_future_milestone_draft_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::generate_future_milestone_draft(project_name.clone()).await?;
    finish(&project_name, None, "generate_future_milestone_draft")
}

#[tauri::command]
pub(crate) async fn approve_future_milestones_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::approve_future_milestones(project_name.clone()).await?;
    finish(&project_name, None, "approve_future_milestones")
}

#[tauri::command]
pub(crate) async fn generate_milestone_draft_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::generate_milestone_draft(project_name.clone()).await?;
    finish(&project_name, None, "generate_milestone_draft")
}

#[tauri::command]
pub(crate) async fn regenerate_milestone_draft_runtime(
    project_name: String,
    current_draft_id: String,
    expected_data_revision: u64,
    feedback: String,
    source: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::regenerate_milestone_draft(
        project_name.clone(),
        current_draft_id,
        expected_data_revision,
        feedback,
        source,
    )
    .await?;
    finish(&project_name, None, "regenerate_milestone_draft")
}

#[tauri::command]
pub(crate) async fn check_milestone_draft_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::check_milestone_draft(project_name.clone()).await?;
    finish(&project_name, None, "check_milestone_draft")
}

#[tauri::command]
pub(crate) async fn approve_milestone_draft_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::approve_milestone_draft(project_name.clone()).await?;
    finish(&project_name, None, "approve_milestone_draft")
}

#[tauri::command]
pub(crate) async fn continue_current_milestone_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::continue_current_milestone(project_name.clone()).await?;
    finish(&project_name, None, "continue_current_milestone")
}

#[tauri::command]
pub(crate) async fn generate_mid_stage_draft_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::generate_mid_stage_draft(project_name.clone()).await?;
    finish(&project_name, None, "generate_mid_stage_draft")
}

#[tauri::command]
pub(crate) async fn regenerate_mid_stage_draft_runtime(
    project_name: String,
    current_draft_id: String,
    expected_data_revision: u64,
    feedback: String,
    source: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::regenerate_mid_stage_draft(
        project_name.clone(),
        current_draft_id,
        expected_data_revision,
        feedback,
        source,
    )
    .await?;
    finish(&project_name, None, "regenerate_mid_stage_draft")
}

#[tauri::command]
pub(crate) async fn check_mid_stage_draft_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::check_mid_stage_draft(project_name.clone()).await?;
    finish(&project_name, None, "check_mid_stage_draft")
}

#[tauri::command]
pub(crate) async fn approve_mid_stage_draft_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::approve_mid_stage_draft(project_name.clone()).await?;
    finish(&project_name, None, "approve_mid_stage_draft")
}

#[tauri::command]
pub(crate) async fn generate_execution_plan_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::generate_execution_plan(project_name.clone()).await?;
    finish(&project_name, None, "generate_execution_plan")
}

#[tauri::command]
pub(crate) async fn check_stage_plan_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::check_stage_plan(project_name.clone()).await?;
    finish(&project_name, None, "check_stage_plan")
}

#[tauri::command]
pub(crate) async fn approve_stage_plan_runtime(
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    crate::commands::milestone::approve_stage_plan(project_name.clone()).await?;
    finish(&project_name, None, "approve_stage_plan")
}

#[tauri::command]
pub(crate) async fn prepare_execution_workspace_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::pipeline::prepare_execution_workspace(state, project_name.clone()).await?;
    let pipeline = pipeline_state.lock().await.clone();
    finish(&project_name, pipeline, "prepare_execution_workspace")
}

#[tauri::command]
pub(crate) async fn refresh_execution_workspace_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::pipeline::refresh_execution_workspace(state, project_name.clone()).await?;
    let pipeline = pipeline_state.lock().await.clone();
    finish(&project_name, pipeline, "refresh_execution_workspace")
}

#[tauri::command]
pub(crate) async fn execute_current_subtask_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::pipeline::execute_current_subtask(state, project_name.clone()).await?;
    let pipeline = pipeline_state.lock().await.clone();
    finish(&project_name, pipeline, "execute_current_subtask")
}

#[tauri::command]
pub(crate) async fn pause_managed_flow_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::commands::workflow::pause_managed_flow(project_name.clone()).await?;
    let pipeline = pipeline_state.lock().await.clone();
    managed_action(&project_name, pipeline, "pause_managed_flow", false)
}

#[tauri::command]
pub(crate) async fn resume_managed_flow_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::commands::workflow::resume_managed_flow(state, project_name.clone()).await?;
    let pipeline = pipeline_state.lock().await.clone();
    managed_action(&project_name, pipeline, "resume_managed_flow", true)
}

#[tauri::command]
pub(crate) async fn start_managed_flow_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::commands::workflow::start_managed_flow(state, project_name.clone()).await?;
    let pipeline = pipeline_state.lock().await.clone();
    managed_action(&project_name, pipeline, "start_managed_flow", true)
}

#[tauri::command]
pub(crate) async fn stop_managed_flow_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::commands::workflow::stop_managed_flow(state, project_name.clone()).await?;
    let pipeline = pipeline_state.lock().await.clone();
    managed_action(&project_name, pipeline, "stop_managed_flow", false)
}

#[tauri::command]
pub(crate) async fn toggle_autopilot_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
    active: bool,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::commands::workflow::toggle_autopilot(state, project_name.clone(), active).await?;
    let pipeline = pipeline_state.lock().await.clone();
    finish(&project_name, pipeline, "toggle_autopilot")
}

#[tauri::command]
pub(crate) async fn autopilot_pause_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::commands::workflow::autopilot_pause(state, project_name.clone()).await?;
    let pipeline = pipeline_state.lock().await.clone();
    finish(&project_name, pipeline, "autopilot_pause")
}

#[tauri::command]
pub(crate) async fn autopilot_resume_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::commands::workflow::autopilot_resume(state, project_name.clone()).await?;
    let pipeline = pipeline_state.lock().await.clone();
    finish(&project_name, pipeline, "autopilot_resume")
}

#[tauri::command]
pub(crate) async fn request_in_stop_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::pipeline::request_in_stop(state, project_name.clone()).await?;
    let pipeline = pipeline_state.lock().await.clone();
    finish(&project_name, pipeline, "request_in_stop")
}

#[tauri::command]
pub(crate) async fn request_ed_stop_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::pipeline::request_ed_stop(state, project_name.clone()).await?;
    let pipeline = pipeline_state.lock().await.clone();
    finish(&project_name, pipeline, "request_ed_stop")
}

#[tauri::command]
pub(crate) async fn confirm_subtask_result_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::pipeline::confirm_subtask_result(state, project_name.clone()).await?;
    let pipeline = pipeline_state.lock().await.clone();
    let action = recovery_action(
        "confirm_subtask_result",
        "任务确认完成",
        "代码与质量结果已确认，Git 稳定点已经写入。",
        None,
        Vec::new(),
        false,
        "系统将进入下一个合法工作流步骤。".to_string(),
    );
    mutation_result(&project_name, pipeline, action, false)
}

#[tauri::command]
pub(crate) async fn retry_git_confirmation_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::pipeline::retry_git_confirmation(state, project_name.clone()).await?;
    let pipeline = pipeline_state.lock().await.clone();
    let latest = crate::load_project(&project_name)?;
    let background_job_started = background_job_active(&latest);
    let action = recovery_action(
        "retry_git_confirmation",
        "Git 确认已恢复",
        "已保留当前代码与质量结果，并续跑原确认事务。",
        None,
        Vec::new(),
        background_job_started,
        if background_job_started {
            "后台作业将从已确认的小阶段继续。".to_string()
        } else {
            "Git 事务已确认，请在控制中心确认下一步。".to_string()
        },
    );
    mutation_result(&project_name, pipeline, action, false)
}

#[tauri::command]
pub(crate) async fn reject_subtask_result_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
    reason: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::pipeline::reject_subtask_result(state, project_name.clone(), reason).await?;
    let pipeline = pipeline_state.lock().await.clone();
    finish(&project_name, pipeline, "reject_subtask_result")
}

#[tauri::command]
pub(crate) async fn run_error_recovery_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::recovery::run_error_recovery(state, project_name.clone()).await?;
    let pipeline = pipeline_state.lock().await.clone();
    let latest = crate::load_project(&project_name)?;
    let recovery_pending = latest.workflow_state.recovery_state.is_some();
    let background_job_started = background_job_active(&latest);
    let action = recovery_action(
        "run_error_recovery",
        if recovery_pending {
            "自动恢复已运行"
        } else {
            "自动恢复完成"
        },
        if recovery_pending {
            "自动恢复已完成本轮处理，但最新状态仍需人工处理。"
        } else {
            "自动恢复已完成，旧阻断状态已经清理。"
        },
        None,
        Vec::new(),
        background_job_started,
        if recovery_pending {
            "请查看最新恢复说明并选择后续动作。".to_string()
        } else if background_job_started {
            "后台作业将继续推进。".to_string()
        } else {
            "请在控制中心确认下一步。".to_string()
        },
    );
    mutation_result(&project_name, pipeline, action, false)
}

#[tauri::command]
pub(crate) async fn acknowledge_execution_recovery_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
    expected_state_fingerprint: Option<String>,
) -> Result<RuntimeMutationResult, String> {
    let impact = crate::pipeline::preview_execution_recovery_impact_inner(&project_name).ok();
    let pipeline_state = state.pipeline_state.clone();
    crate::pipeline::acknowledge_execution_recovery(
        state,
        project_name.clone(),
        expected_state_fingerprint,
    )
    .await?;
    let pipeline = pipeline_state.lock().await.clone();
    let latest = crate::load_project(&project_name)?;
    let background_job_started = background_job_active(&latest);
    let action = recovery_action(
        "acknowledge_execution_recovery",
        "执行基线已恢复",
        if background_job_started {
            "执行基线已恢复，后台作业已重新接续。"
        } else {
            "执行基线已恢复，当前未自动继续。"
        },
        impact.as_ref().map(|value| value.baseline_commit.clone()),
        impact
            .as_ref()
            .map(|value| value.discarded_files.clone())
            .unwrap_or_default(),
        background_job_started,
        if background_job_started {
            "后台将重新执行当前任务。".to_string()
        } else {
            "请在控制中心确认下一步。".to_string()
        },
    );
    mutation_result(&project_name, pipeline, action, false)
}

#[tauri::command]
pub(crate) async fn resolve_human_recovery_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
    resolution: String,
    reason: String,
    accepted_criteria: Option<Vec<u32>>,
    expected_state_fingerprint: Option<String>,
) -> Result<RuntimeMutationResult, String> {
    let impact = if matches!(resolution.as_str(), "restore_and_retry" | "skip_task") {
        let project = crate::load_project(&project_name)?;
        crate::pipeline::verify_execution_recovery_preview(
            &project,
            &resolution,
            expected_state_fingerprint.as_deref(),
        )
        .ok()
    } else {
        None
    };
    let pipeline_state = state.pipeline_state.clone();
    crate::recovery::resolve_human_recovery(
        state,
        project_name.clone(),
        resolution.clone(),
        reason,
        accepted_criteria,
        expected_state_fingerprint,
    )
    .await?;
    let pipeline = pipeline_state.lock().await.clone();
    let latest = crate::load_project(&project_name)?;
    let background_job_started = background_job_active(&latest);
    let (title, message) = match resolution.as_str() {
        "revalidate" => ("重新验证完成", "代码未回退，验证结果已重新写入。"),
        "retest" => ("重新测试完成", "代码未回退，测试结果已重新写入。"),
        "restore_and_retry" => ("执行基线已恢复", "已恢复基线并准备重试当前任务。"),
        "regenerate_plan" => ("重新规划已安排", "当前代码已保留，系统将重新规划任务。"),
        "confirm_actual_pass" => ("人工通过已记录", "当前代码已保留，人工验证证据已写入。"),
        "accept_deviation" => (
            "验收偏差已记录",
            "当前代码已保留，偏差约束将传递到后续任务。",
        ),
        "skip_task" => ("任务已跳过", "系统将依据显式依赖契约决定是否继续。"),
        _ => ("恢复动作已完成", "后端已应用最新恢复决策。"),
    };
    let action = recovery_action(
        "resolve_human_recovery",
        title,
        message,
        impact.as_ref().map(|value| value.baseline_commit.clone()),
        impact
            .as_ref()
            .map(|value| value.discarded_files.clone())
            .unwrap_or_default(),
        background_job_started,
        if latest.workflow_state.recovery_state.is_some() {
            "恢复仍需人工处理，请查看最新恢复说明。".to_string()
        } else if background_job_started {
            "后台作业将继续推进。".to_string()
        } else {
            "请在控制中心确认下一步。".to_string()
        },
    );
    mutation_result(&project_name, pipeline, action, false)
}

#[tauri::command]
pub(crate) async fn approve_milestone_outcome_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
    branch: Option<String>,
    submission: Option<crate::commands::milestone::MilestoneReviewSubmission>,
) -> Result<RuntimeMutationResult, String> {
    let pipeline_state = state.pipeline_state.clone();
    crate::commands::milestone::approve_milestone_outcome(
        state,
        project_name.clone(),
        branch,
        submission,
    )
    .await?;
    let pipeline = pipeline_state.lock().await.clone();
    finish(&project_name, pipeline, "approve_milestone_outcome")
}

#[tauri::command]
pub(crate) async fn update_human_review_policy_runtime(
    state: tauri::State<'_, crate::AppState>,
    project_name: String,
    expected_revision: u64,
    human_review_cadence: crate::project::HumanReviewCadence,
    vision_review_enabled: bool,
) -> Result<RuntimeMutationResult, String> {
    let pipeline = state.pipeline_state.lock().await.clone();
    if pipeline.as_ref().is_some_and(|pipeline| {
        pipeline.project_name == project_name
            && pipeline.status == crate::pipeline::PipelineStatus::Running
    }) {
        return Err("执行正在运行，不能修改人工确认或视觉审查策略".to_string());
    }
    crate::mutate_project_for_control(&project_name, |project| {
        if project.workflow_state.data_revision != expected_revision {
            return Err(format!(
                "项目修订冲突：请求={}，磁盘={}",
                expected_revision, project.workflow_state.data_revision
            ));
        }
        if project
            .execution_session
            .as_ref()
            .is_some_and(|session| session.active)
        {
            return Err("活动执行会话期间不能修改人工确认或视觉审查策略".to_string());
        }
        if project.milestones.iter().any(|milestone| {
            milestone.human_review_items.iter().any(|item| {
                item.review_cycle == milestone.human_review_cycle
                    && item.human_decision == crate::project::MilestoneHumanDecision::Pending
            })
        }) {
            return Err("存在未决的大阶段人工确认清单，处理完成前不能切换策略".to_string());
        }
        if project.human_review_cadence == human_review_cadence
            && project.vision_review_enabled == vision_review_enabled
        {
            return Ok(((), false));
        }
        project.human_review_cadence = human_review_cadence;
        project.vision_review_enabled = vision_review_enabled;
        project.workflow_state.data_revision =
            project.workflow_state.data_revision.saturating_add(1);
        project.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
        Ok(((), true))
    })?;
    finish(&project_name, pipeline, "update_human_review_policy")
}

#[tauri::command]
pub(crate) async fn summarize_milestone_runtime(
    project_name: String,
    milestone_id: String,
) -> Result<RuntimeMutationResult, String> {
    let summary =
        crate::commands::milestone::summarize_milestone(project_name.clone(), milestone_id).await?;
    let mut action = RuntimeActionSummary::silent("summarize_milestone");
    action.message = summary;
    mutation_result(&project_name, None, action, true)
}
