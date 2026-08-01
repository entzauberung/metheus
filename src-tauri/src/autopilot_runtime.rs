use crate::commands::{milestone, workflow};
use crate::pipeline::PipelineState;
use crate::project;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep, Duration};

const MAX_GIT_CONFIRMATION_RETRIES: u32 = 2;

fn git_confirmation_retry_delay(completed_retries: u32) -> Option<i64> {
    match completed_retries {
        0 => Some(5),
        1 => Some(15),
        _ => None,
    }
}

#[derive(Default)]
pub(crate) struct AutopilotRuntime {
    jobs: Mutex<HashMap<String, AutopilotJob>>,
}

struct AutopilotJob {
    job_id: String,
    generation: u64,
    handle: JoinHandle<()>,
}

fn install_job(
    jobs: &mut HashMap<String, AutopilotJob>,
    project_name: String,
    candidate: AutopilotJob,
) -> bool {
    if let Some(existing) = jobs.get(&project_name) {
        if !existing.handle.is_finished()
            && existing.job_id == candidate.job_id
            && existing.generation == candidate.generation
        {
            candidate.handle.abort();
            return false;
        }
        if !existing.handle.is_finished() {
            existing.handle.abort();
        }
    }
    jobs.insert(project_name, candidate);
    true
}

impl AutopilotRuntime {
    pub(crate) async fn start(
        self: &Arc<Self>,
        pipeline_state: Arc<Mutex<Option<PipelineState>>>,
        project_name: String,
    ) -> Result<(), String> {
        let project = crate::load_project(&project_name)?;
        let state = project
            .workflow_state
            .autopilot_state
            .as_ref()
            .ok_or("自动驾驶状态不存在。".to_string())?;
        let job_id = state.job_id.clone();
        let generation = state.job_generation;
        let mut jobs = self.jobs.lock().await;
        if jobs.get(&project_name).is_some_and(|existing| {
            !existing.handle.is_finished()
                && existing.job_id == job_id
                && existing.generation == generation
        }) {
            return Ok(());
        }
        let task_project = project_name.clone();
        let task_job_id = job_id.clone();
        let handle = tokio::spawn(async move {
            drive_project(pipeline_state, task_project, task_job_id, generation).await;
        });
        let candidate = AutopilotJob {
            job_id,
            generation,
            handle,
        };
        install_job(&mut jobs, project_name, candidate);
        Ok(())
    }

    pub(crate) async fn start_if_active(
        self: &Arc<Self>,
        pipeline_state: Arc<Mutex<Option<PipelineState>>>,
        project_name: String,
    ) -> Result<bool, String> {
        let project = crate::load_project(&project_name)?;
        let should_start = project.workflow_state.autopilot_active
            && project
                .workflow_state
                .autopilot_state
                .as_ref()
                .is_some_and(|state| {
                    state.active && state.run_status == project::AutopilotRunStatus::Running
                });
        if should_start {
            self.start(pipeline_state, project_name).await?;
        }
        Ok(should_start)
    }
}

/// Tauri 进程重启后，为仍可安全续跑的活动项目创建新一代作业身份。
/// 返回 true 表示调用方应在项目落盘并释放流水线锁后启动运行器。
pub(crate) fn reconcile_startup_job(project: &mut project::Project) -> bool {
    let resumable_validation_recovery = project
        .workflow_state
        .recovery_state
        .as_ref()
        .is_some_and(crate::recovery::validation_retry_can_resume);
    let Some(state) = project.workflow_state.autopilot_state.as_mut() else {
        return false;
    };
    let automatically_recoverable = matches!(
        state.recovery_action,
        project::AutopilotRecoveryAction::PrepareExecutionWorkspace
            | project::AutopilotRecoveryAction::RegenerateExecutionPlan
            | project::AutopilotRecoveryAction::RetryGitConfirmation
    ) || (state.recovery_action
        == project::AutopilotRecoveryAction::RestoreExecutionBaseline
        && crate::autopilot_failure::is_transient(&state.last_failure_kind))
        || (state.recovery_action == project::AutopilotRecoveryAction::RunAutomaticRecovery
            && resumable_validation_recovery);
    let should_run = project.workflow_state.autopilot_active
        && state.active
        && (state.run_status == project::AutopilotRunStatus::Running
            || (state.run_status == project::AutopilotRunStatus::ErrorStopped
                && automatically_recoverable));
    if should_run {
        let now = chrono::Utc::now().to_rfc3339();
        state.job_id = uuid::Uuid::new_v4().to_string();
        state.job_generation = state.job_generation.saturating_add(1);
        state.job_owner = project::AutopilotJobOwner::BackendRuntime;
        state.current_action_id.clear();
        state.current_action_kind.clear();
        state.action_started_at.clear();
        state.heartbeat_at = now.clone();
        state.last_action = "应用重启对账完成，后端自动驾驶继续运行".to_string();
        state.last_action_at = now;
        true
    } else {
        state.job_owner = project::AutopilotJobOwner::None;
        state.current_action_id.clear();
        state.current_action_kind.clear();
        state.action_started_at.clear();
        false
    }
}

fn job_matches(project: &project::Project, job_id: &str, generation: u64) -> bool {
    project
        .workflow_state
        .autopilot_state
        .as_ref()
        .is_some_and(|state| {
            project.workflow_state.autopilot_active
                && state.active
                && state.job_id == job_id
                && state.job_generation == generation
        })
}

fn retry_due(state: &project::AutopilotState) -> bool {
    state
        .next_retry_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .is_none_or(|deadline| deadline <= chrono::Utc::now())
}

fn clear_action_state(state: &mut project::AutopilotState) {
    clear_action_claim(state);
    state.transient_retry_count = 0;
    state.next_retry_at = None;
    state.last_failure_kind = project::AutopilotFailureKind::None;
    state.last_failure_fingerprint.clear();
    state.consecutive_no_progress = 0;
    if state.recovery_action == project::AutopilotRecoveryAction::RetryAutopilotAdvance {
        state.recovery_action = project::AutopilotRecoveryAction::None;
    }
}

fn clear_action_claim(state: &mut project::AutopilotState) {
    state.current_action_id.clear();
    state.current_action_kind.clear();
    state.action_started_at.clear();
}

fn persist_heartbeat(project: &mut project::Project) -> Result<(), String> {
    if let Some(state) = project.workflow_state.autopilot_state.as_mut() {
        state.heartbeat_at = chrono::Utc::now().to_rfc3339();
        state.job_owner = project::AutopilotJobOwner::BackendRuntime;
    }
    crate::save_project(project)
}

fn validation_retry_waiting(project: &project::Project) -> Option<String> {
    let recovery = project.workflow_state.recovery_state.as_ref()?;
    if !crate::recovery::validation_retry_can_resume(recovery)
        || crate::recovery::validation_retry_due(recovery)
    {
        return None;
    }
    Some(format!(
        "等待第 {}/{} 次 AI 审查验证重试",
        recovery.validation_retry_count.saturating_add(1),
        recovery.max_validation_retries
    ))
}

fn action_claim_matches(
    project: &project::Project,
    job_id: &str,
    generation: u64,
    action_id: &str,
    action_kind: &str,
) -> bool {
    job_matches(project, job_id, generation)
        && project
            .workflow_state
            .autopilot_state
            .as_ref()
            .is_some_and(|state| {
                state.current_action_id == action_id && state.current_action_kind == action_kind
            })
}

enum DispatchOutcome {
    Completed(Result<(), String>),
    Superseded,
}

async fn dispatch_action_with_heartbeat(
    pipeline_state: &Arc<Mutex<Option<PipelineState>>>,
    project_name: &str,
    next: &workflow::AutopilotNextStep,
    job_id: &str,
    generation: u64,
    action_id: &str,
) -> DispatchOutcome {
    let dispatch = dispatch_action(pipeline_state, project_name, next);
    tokio::pin!(dispatch);
    let mut heartbeat = interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            result = &mut dispatch => return DispatchOutcome::Completed(result),
            _ = heartbeat.tick() => {
                let mut latest = match crate::load_project(project_name) {
                    Ok(project) => project,
                    Err(error) => return DispatchOutcome::Completed(Err(error)),
                };
                if !action_claim_matches(
                    &latest,
                    job_id,
                    generation,
                    action_id,
                    &next.command,
                ) {
                    return DispatchOutcome::Superseded;
                }
                if persist_heartbeat(&mut latest).is_err() {
                    return DispatchOutcome::Completed(Err(
                        "自动驾驶动作心跳持久化失败。".to_string(),
                    ));
                }
            }
        }
    }
}

async fn recover_stopped_action(
    pipeline_state: &Arc<Mutex<Option<PipelineState>>>,
    project: &project::Project,
) -> Result<bool, String> {
    let Some(state) = project.workflow_state.autopilot_state.as_ref() else {
        return Ok(false);
    };
    let preserve_retry_state =
        state.recovery_action == project::AutopilotRecoveryAction::RestoreExecutionBaseline;
    match state.recovery_action {
        project::AutopilotRecoveryAction::RestoreExecutionBaseline => {
            if !crate::autopilot_failure::is_transient(&state.last_failure_kind)
                || state.transient_retry_count > crate::autopilot_failure::MAX_TRANSIENT_RETRIES
            {
                return Ok(false);
            }
            crate::pipeline::acknowledge_execution_recovery_with_pipeline(
                pipeline_state,
                project.name.clone(),
            )
            .await?;
        }
        project::AutopilotRecoveryAction::PrepareExecutionWorkspace => {
            let workspace =
                crate::pipeline::get_execution_workspace_status_inner(&project.project_path)?;
            if workspace.has_commits && workspace.has_external_changes {
                return Ok(false);
            }
            crate::pipeline::prepare_execution_workspace_inner(project.name.clone()).await?;
        }
        project::AutopilotRecoveryAction::RegenerateExecutionPlan => {
            let mid = project
                .milestones
                .iter()
                .find(|milestone| milestone.id == project.current_milestone_id)
                .and_then(|milestone| {
                    milestone
                        .mid_stages
                        .iter()
                        .find(|mid| mid.id == project.current_mid_stage_id)
                })
                .ok_or("当前中阶段不存在。".to_string())?;
            let source =
                if project.workflow_state.current_step == project::WorkflowStep::PlanApproving {
                    "approval_rejected"
                } else {
                    "check_failed"
                };
            milestone::regenerate_execution_plan(
                project.name.clone(),
                project.workflow_state.data_revision,
                mid.plan_draft_revision,
                String::new(),
                source.to_string(),
            )
            .await?;
        }
        project::AutopilotRecoveryAction::RetryGitConfirmation => {
            if state.transient_retry_count >= MAX_GIT_CONFIRMATION_RETRIES {
                return Ok(false);
            }
            crate::pipeline::retry_git_confirmation_with_source(
                pipeline_state,
                project.name.clone(),
                project::OperationSource::Autopilot,
            )
            .await?;
        }
        _ => return Ok(false),
    }
    let mut latest = crate::load_project(&project.name)?;
    if let Some(state) = latest.workflow_state.autopilot_state.as_mut() {
        state.run_status = project::AutopilotRunStatus::Running;
        state.error_message.clear();
        state.recovery_action = project::AutopilotRecoveryAction::None;
        if preserve_retry_state {
            clear_action_claim(state);
            state.next_retry_at = None;
        } else {
            clear_action_state(state);
        }
    }
    crate::save_project(&latest)?;
    Ok(true)
}

async fn schedule_git_retry(project_name: &str, message: &str) -> Result<bool, String> {
    let mut project = crate::load_project(project_name)?;
    let Some(state) = project.workflow_state.autopilot_state.as_mut() else {
        return Ok(false);
    };
    if state.recovery_action != project::AutopilotRecoveryAction::RetryGitConfirmation {
        return Ok(false);
    }
    let Some(delay) = git_confirmation_retry_delay(state.transient_retry_count) else {
        return Ok(false);
    };
    state.transient_retry_count = state.transient_retry_count.saturating_add(1);
    state.next_retry_at =
        Some((chrono::Utc::now() + chrono::Duration::seconds(delay)).to_rfc3339());
    state.run_status = project::AutopilotRunStatus::Running;
    state.last_action = format!("Git 确认暂时失败；将在 {} 秒后续跑同一事务", delay);
    state.last_action_at = chrono::Utc::now().to_rfc3339();
    state.error_message = message.to_string();
    state.last_failure_kind = project::AutopilotFailureKind::ProviderUnavailable;
    crate::save_project(&project)?;
    Ok(true)
}

async fn drive_project(
    pipeline_state: Arc<Mutex<Option<PipelineState>>>,
    project_name: String,
    job_id: String,
    generation: u64,
) {
    loop {
        let mut project = match crate::load_project(&project_name) {
            Ok(project) => project,
            Err(_) => break,
        };
        if !job_matches(&project, &job_id, generation) {
            break;
        }
        let run_status = project
            .workflow_state
            .autopilot_state
            .as_ref()
            .map(|state| state.run_status.clone())
            .unwrap_or(project::AutopilotRunStatus::ErrorStopped);
        if run_status == project::AutopilotRunStatus::ErrorStopped {
            let retry_is_due = project
                .workflow_state
                .autopilot_state
                .as_ref()
                .is_some_and(retry_due);
            if !retry_is_due {
                let _ = persist_heartbeat(&mut project);
                sleep(Duration::from_secs(1)).await;
                continue;
            }
            match recover_stopped_action(&pipeline_state, &project).await {
                Ok(true) => continue,
                Ok(false) => break,
                Err(error) => {
                    if schedule_git_retry(&project_name, &error)
                        .await
                        .unwrap_or(false)
                    {
                        sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                    let _ = workflow::autopilot_mark_error(
                        project_name.clone(),
                        "自动恢复失败".to_string(),
                        error,
                    )
                    .await;
                    let retrying = crate::load_project(&project_name)
                        .ok()
                        .and_then(|project| project.workflow_state.autopilot_state)
                        .is_some_and(|state| {
                            state.run_status == project::AutopilotRunStatus::Running
                        });
                    if retrying {
                        sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                    break;
                }
            }
        }
        if run_status != project::AutopilotRunStatus::Running {
            break;
        }
        if let Some(description) = validation_retry_waiting(&project) {
            if let Some(state) = project.workflow_state.autopilot_state.as_mut() {
                if state.last_action != description {
                    state.last_action = description;
                    state.last_action_at = chrono::Utc::now().to_rfc3339();
                }
            }
            let _ = persist_heartbeat(&mut project);
            sleep(Duration::from_secs(1)).await;
            continue;
        }
        if !retry_due(project.workflow_state.autopilot_state.as_ref().unwrap()) {
            let _ = persist_heartbeat(&mut project);
            sleep(Duration::from_secs(1)).await;
            continue;
        }
        if let Some(state) = project.workflow_state.autopilot_state.as_mut() {
            state.next_retry_at = None;
        }
        if persist_heartbeat(&mut project).is_err() {
            break;
        }

        let next = match workflow::autopilot_next_step(project_name.clone()).await {
            Ok(next) => next,
            Err(error) => {
                let _ = workflow::autopilot_mark_error(
                    project_name.clone(),
                    "自动驾驶决策失败".to_string(),
                    error,
                )
                .await;
                sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        if next.waiting_for_execution {
            sleep(Duration::from_secs(1)).await;
            continue;
        }
        if next.command.is_empty() {
            if next.is_error {
                continue;
            }
            break;
        }

        let mut claimed = match crate::load_project(&project_name) {
            Ok(project) => project,
            Err(_) => break,
        };
        if !job_matches(&claimed, &job_id, generation) {
            break;
        }
        let action_id = uuid::Uuid::new_v4().to_string();
        if let Some(state) = claimed.workflow_state.autopilot_state.as_mut() {
            // 执行引擎重试已经成功推进到确认阶段时，Git 确认必须拥有独立的两次额度。
            // Git 自身重试会保留 RetryGitConfirmation，因此不会在每轮确认前被重置。
            if next.command == "confirm_subtask_result"
                && state.recovery_action != project::AutopilotRecoveryAction::RetryGitConfirmation
            {
                state.transient_retry_count = 0;
                state.next_retry_at = None;
                state.last_failure_kind = project::AutopilotFailureKind::None;
                state.last_failure_fingerprint.clear();
            }
            state.current_action_id = action_id.clone();
            state.current_action_kind = next.command.clone();
            state.action_started_at = chrono::Utc::now().to_rfc3339();
            state.heartbeat_at = state.action_started_at.clone();
            state.last_action = next.description.clone();
            state.last_action_at = state.action_started_at.clone();
        }
        if crate::save_project(&claimed).is_err() {
            break;
        }

        match dispatch_action_with_heartbeat(
            &pipeline_state,
            &project_name,
            &next,
            &job_id,
            generation,
            &action_id,
        )
        .await
        {
            DispatchOutcome::Completed(Ok(())) => {
                if let Ok(mut latest) = crate::load_project(&project_name) {
                    if action_claim_matches(&latest, &job_id, generation, &action_id, &next.command)
                    {
                        if let Some(state) = latest.workflow_state.autopilot_state.as_mut() {
                            // 执行命令只负责派发后台任务。基础设施重试状态必须保留到
                            // 执行真正完成并进入确认，否则第二次失败会被误判为首次失败。
                            if next.command == "execute_current_subtask"
                                || next.command == "execute_control_action"
                            {
                                clear_action_claim(state);
                            } else {
                                clear_action_state(state);
                            }
                            state.heartbeat_at = chrono::Utc::now().to_rfc3339();
                        }
                        let _ = crate::save_project(&latest);
                    }
                }
            }
            DispatchOutcome::Completed(Err(error)) => {
                let still_current = crate::load_project(&project_name)
                    .ok()
                    .is_some_and(|latest| {
                        action_claim_matches(
                            &latest,
                            &job_id,
                            generation,
                            &action_id,
                            &next.command,
                        )
                    });
                if !still_current {
                    break;
                }
                if schedule_git_retry(&project_name, &error)
                    .await
                    .unwrap_or(false)
                {
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
                let _ =
                    workflow::autopilot_mark_error(project_name.clone(), next.description, error)
                        .await;
            }
            DispatchOutcome::Superseded => break,
        }
        sleep(Duration::from_secs(1)).await;
    }
}

fn string_arg(next: &workflow::AutopilotNextStep, key: &str) -> Result<String, String> {
    next.args
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| format!("自动驾驶动作 {} 缺少参数 {}", next.command, key))
}

fn u64_arg(next: &workflow::AutopilotNextStep, key: &str) -> Result<u64, String> {
    next.args
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("自动驾驶动作 {} 缺少参数 {}", next.command, key))
}

async fn dispatch_action(
    pipeline_state: &Arc<Mutex<Option<PipelineState>>>,
    project_name: &str,
    next: &workflow::AutopilotNextStep,
) -> Result<(), String> {
    match next.command.as_str() {
        "execute_control_action" => {
            let request = next
                .args
                .get("request")
                .cloned()
                .ok_or_else(|| "控制动作缺少 request 参数".to_string())?;
            let request = serde_json::from_value::<
                crate::control_action_executor::ControlActionRequest,
            >(request)
            .map_err(|error| format!("控制动作参数无效：{}", error))?;
            crate::control_action_executor::execute(
                pipeline_state.clone(),
                project_name.to_string(),
                request,
            )
            .await?;
        }
        "select_milestone" => {
            milestone::select_milestone(project_name.to_string(), string_arg(next, "milestoneId")?)
                .await?;
        }
        "transition_workflow" => {
            workflow::transition_workflow(
                project_name.to_string(),
                string_arg(next, "targetStep")?,
                string_arg(next, "reason")?,
            )
            .await?;
        }
        "generate_mid_stage_draft" => {
            milestone::generate_mid_stage_draft(project_name.to_string()).await?;
        }
        "regenerate_mid_stage_draft" => {
            milestone::regenerate_mid_stage_draft(
                project_name.to_string(),
                string_arg(next, "currentDraftId")?,
                u64_arg(next, "expectedDataRevision")?,
                string_arg(next, "feedback")?,
                string_arg(next, "source")?,
            )
            .await?;
        }
        "check_mid_stage_draft" => {
            milestone::check_mid_stage_draft(project_name.to_string()).await?;
        }
        "approve_mid_stage_draft" => {
            milestone::approve_mid_stage_draft(project_name.to_string()).await?;
        }
        "select_mid_stage" => {
            milestone::select_mid_stage(project_name.to_string(), string_arg(next, "midStageId")?)
                .await?;
        }
        "generate_execution_plan" => {
            milestone::generate_execution_plan(project_name.to_string()).await?;
        }
        "regenerate_execution_plan" => {
            milestone::regenerate_execution_plan(
                project_name.to_string(),
                u64_arg(next, "expectedDataRevision")?,
                u64_arg(next, "expectedPlanDraftRevision")?,
                next.args
                    .get("feedback")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                string_arg(next, "source")?,
            )
            .await?;
        }
        "check_stage_plan" => {
            milestone::check_stage_plan(project_name.to_string()).await?;
        }
        "approve_stage_plan" => {
            milestone::approve_stage_plan(project_name.to_string()).await?;
        }
        "calibrate_next_subtask_command" => {
            crate::plan_calibration::calibrate_next_subtask_with_pipeline(
                pipeline_state.clone(),
                project_name.to_string(),
            )
            .await?;
        }
        "execute_current_subtask" => {
            crate::pipeline::execute_current_subtask_with_source(
                pipeline_state.clone(),
                project_name.to_string(),
                project::OperationSource::Autopilot,
            )
            .await?;
        }
        "confirm_subtask_result" => {
            crate::pipeline::confirm_subtask_result_with_source(
                pipeline_state,
                project_name.to_string(),
                project::OperationSource::Autopilot,
            )
            .await?;
        }
        "run_error_recovery" => {
            crate::recovery::run_error_recovery_with_pipeline(
                pipeline_state.clone(),
                project_name.to_string(),
            )
            .await?;
        }
        "prepare_execution_workspace" => {
            crate::pipeline::prepare_execution_workspace_inner(project_name.to_string()).await?;
        }
        "refresh_execution_workspace" => {
            crate::pipeline::refresh_execution_workspace_inner(project_name.to_string()).await?;
        }
        command => return Err(format!("后端自动驾驶不支持原子动作：{}", command)),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active_project(status: project::AutopilotRunStatus) -> project::Project {
        let mut value = project::Project::new("autopilot-runtime-test");
        value.workflow_state.autopilot_active = true;
        value.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            run_status: status,
            job_id: "old-job".to_string(),
            job_generation: 7,
            job_owner: project::AutopilotJobOwner::BackendRuntime,
            current_action_id: "old-action".to_string(),
            current_action_kind: "generate_execution_plan".to_string(),
            action_started_at: "2026-07-25T00:00:00Z".to_string(),
            ..Default::default()
        });
        value
    }

    #[test]
    fn startup_replaces_stale_running_job_identity() {
        let mut project = active_project(project::AutopilotRunStatus::Running);
        assert!(reconcile_startup_job(&mut project));
        let state = project.workflow_state.autopilot_state.unwrap();
        assert_ne!(state.job_id, "old-job");
        assert_eq!(state.job_generation, 8);
        assert_eq!(state.job_owner, project::AutopilotJobOwner::BackendRuntime);
        assert!(state.current_action_id.is_empty());
        assert!(state.current_action_kind.is_empty());
        assert!(!state.heartbeat_at.is_empty());
    }

    #[test]
    fn startup_does_not_resume_human_boundary() {
        let mut project = active_project(project::AutopilotRunStatus::ErrorStopped);
        project
            .workflow_state
            .autopilot_state
            .as_mut()
            .unwrap()
            .recovery_action = project::AutopilotRecoveryAction::WaitHumanDecision;
        assert!(!reconcile_startup_job(&mut project));
        let state = project.workflow_state.autopilot_state.unwrap();
        assert_eq!(state.job_id, "old-job");
        assert_eq!(state.job_generation, 7);
        assert_eq!(state.job_owner, project::AutopilotJobOwner::None);
        assert!(state.current_action_id.is_empty());
    }

    #[test]
    fn startup_resumes_only_safe_stopped_recovery() {
        let mut project = active_project(project::AutopilotRunStatus::ErrorStopped);
        project
            .workflow_state
            .autopilot_state
            .as_mut()
            .unwrap()
            .recovery_action = project::AutopilotRecoveryAction::RetryGitConfirmation;
        assert!(reconcile_startup_job(&mut project));
        assert_eq!(
            project
                .workflow_state
                .autopilot_state
                .unwrap()
                .job_generation,
            8
        );
    }

    #[test]
    fn startup_resumes_bounded_validation_retry() {
        let mut project = active_project(project::AutopilotRunStatus::ErrorStopped);
        project
            .workflow_state
            .autopilot_state
            .as_mut()
            .unwrap()
            .recovery_action = project::AutopilotRecoveryAction::RunAutomaticRecovery;
        project.workflow_state.recovery_state = Some(project::RecoveryState {
            error_kind: project::RecoveryErrorKind::ReviewTransientFailure,
            phase: project::RecoveryPhase::Retesting,
            validation_retry_count: 1,
            max_validation_retries: 3,
            next_validation_retry_at: Some("2099-01-01T00:00:00Z".to_string()),
            ..Default::default()
        });

        assert!(reconcile_startup_job(&mut project));
        let state = project.workflow_state.autopilot_state.unwrap();
        assert_eq!(state.job_generation, 8);
        assert!(state.current_action_id.is_empty());
        assert_eq!(state.job_owner, project::AutopilotJobOwner::BackendRuntime);
    }

    #[test]
    fn validation_retry_waits_for_deadline_and_action_claim_is_exact() {
        let mut project = active_project(project::AutopilotRunStatus::Running);
        project.workflow_state.recovery_state = Some(project::RecoveryState {
            error_kind: project::RecoveryErrorKind::ReviewProtocolFailure,
            phase: project::RecoveryPhase::Retesting,
            validation_retry_count: 1,
            max_validation_retries: 2,
            next_validation_retry_at: Some("2099-01-01T00:00:00Z".to_string()),
            ..Default::default()
        });
        assert!(validation_retry_waiting(&project)
            .as_deref()
            .is_some_and(|message| message.contains("2/2")));
        assert!(action_claim_matches(
            &project,
            "old-job",
            7,
            "old-action",
            "generate_execution_plan"
        ));
        assert!(!action_claim_matches(
            &project,
            "old-job",
            7,
            "stale-action",
            "generate_execution_plan"
        ));

        project
            .workflow_state
            .recovery_state
            .as_mut()
            .unwrap()
            .next_validation_retry_at = Some("2020-01-01T00:00:00Z".to_string());
        assert!(validation_retry_waiting(&project).is_none());
    }

    #[test]
    fn autopilot_git_confirmation_retry_stops_after_two_attempts() {
        assert_eq!(git_confirmation_retry_delay(0), Some(5));
        assert_eq!(git_confirmation_retry_delay(1), Some(15));
        assert_eq!(git_confirmation_retry_delay(2), None);
        assert_eq!(git_confirmation_retry_delay(3), None);
    }

    #[tokio::test]
    async fn installing_new_generation_aborts_unfinished_stale_job() {
        let mut jobs = HashMap::new();
        let stale = tokio::spawn(std::future::pending::<()>());
        let stale_abort = stale.abort_handle();
        jobs.insert(
            "project-1".to_string(),
            AutopilotJob {
                job_id: "job-old".to_string(),
                generation: 1,
                handle: stale,
            },
        );
        let current = tokio::spawn(std::future::pending::<()>());

        assert!(install_job(
            &mut jobs,
            "project-1".to_string(),
            AutopilotJob {
                job_id: "job-new".to_string(),
                generation: 2,
                handle: current,
            },
        ));

        tokio::task::yield_now().await;
        assert!(stale_abort.is_finished());
        let installed = jobs.get("project-1").unwrap();
        assert_eq!(installed.job_id, "job-new");
        assert_eq!(installed.generation, 2);
        assert!(!installed.handle.is_finished());
        installed.handle.abort();
    }

    #[tokio::test]
    async fn installing_same_job_identity_keeps_one_runtime_job() {
        let mut jobs = HashMap::new();
        let installed = tokio::spawn(std::future::pending::<()>());
        let installed_abort = installed.abort_handle();
        jobs.insert(
            "project-1".to_string(),
            AutopilotJob {
                job_id: "job-current".to_string(),
                generation: 3,
                handle: installed,
            },
        );
        let duplicate = tokio::spawn(std::future::pending::<()>());
        let duplicate_abort = duplicate.abort_handle();

        assert!(!install_job(
            &mut jobs,
            "project-1".to_string(),
            AutopilotJob {
                job_id: "job-current".to_string(),
                generation: 3,
                handle: duplicate,
            },
        ));

        tokio::task::yield_now().await;
        assert!(!installed_abort.is_finished());
        assert!(duplicate_abort.is_finished());
        assert_eq!(jobs.len(), 1);
        jobs.get("project-1").unwrap().handle.abort();
    }
}
