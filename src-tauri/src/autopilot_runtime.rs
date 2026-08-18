use crate::commands::{milestone, workflow};
use crate::pipeline::PipelineState;
use crate::project;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep, Duration};

const MAX_GIT_CONFIRMATION_RETRIES: u32 = 2;

/// Business progress idle → warning (does not cancel the future).
pub(crate) const RECOVERY_PROGRESS_WARNING_SECS: i64 = 90;
/// Business progress idle → stalled (alive but no progress).
pub(crate) const RECOVERY_PROGRESS_STALLED_SECS: i64 = 300;
/// Hard action wall-clock cap for recovery dispatch (≤ control-action 12 min).
pub(crate) const RECOVERY_ACTION_HARD_TIMEOUT_SECS: u64 = 12 * 60;
const RECOVERY_OWNER_STOP_GRACE_SECS: u64 = 2;

fn git_confirmation_retry_delay(completed_retries: u32) -> Option<i64> {
    match completed_retries {
        0 => Some(5),
        1 => Some(15),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryProgressGate {
    Running,
    Warning,
    Stalled,
    HardTimeout,
}

/// Pure time policy for recovery progress. Uses injected `now`; never sleeps.
/// - Hard timeout is based on action_started_at.
/// - Warning/stalled use last_progress_at (business), falling back to action start.
/// Heartbeat must never be passed as last_progress_at.
pub(crate) fn classify_recovery_progress(
    now: chrono::DateTime<chrono::Utc>,
    last_progress_at: Option<chrono::DateTime<chrono::Utc>>,
    action_started_at: Option<chrono::DateTime<chrono::Utc>>,
) -> RecoveryProgressGate {
    if let Some(started) = action_started_at {
        let action_elapsed = now.signed_duration_since(started).num_seconds();
        if action_elapsed >= RECOVERY_ACTION_HARD_TIMEOUT_SECS as i64 {
            return RecoveryProgressGate::HardTimeout;
        }
    }
    let progress_anchor = last_progress_at.or(action_started_at);
    let Some(anchor) = progress_anchor else {
        return RecoveryProgressGate::Running;
    };
    let idle = now.signed_duration_since(anchor).num_seconds();
    if idle >= RECOVERY_PROGRESS_STALLED_SECS {
        RecoveryProgressGate::Stalled
    } else if idle >= RECOVERY_PROGRESS_WARNING_SECS {
        RecoveryProgressGate::Warning
    } else {
        RecoveryProgressGate::Running
    }
}

fn parse_rfc3339(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

fn is_recovery_dispatch_command(command: &str) -> bool {
    command == "run_error_recovery"
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

fn assign_new_job_identity(state: &mut project::AutopilotState, action: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    state.job_id = uuid::Uuid::new_v4().to_string();
    state.job_generation = state.job_generation.saturating_add(1);
    state.job_owner = project::AutopilotJobOwner::BackendRuntime;
    clear_action_claim(state);
    state.heartbeat_at = now.clone();
    state.last_action = action.to_string();
    state.last_action_at = now;
}

fn converge_finished_job_state(
    state: &mut project::AutopilotState,
    job_id: &str,
    generation: u64,
) -> bool {
    if state.job_id != job_id || state.job_generation != generation {
        return false;
    }
    state.job_owner = project::AutopilotJobOwner::None;
    clear_action_claim(state);
    if state.run_status == project::AutopilotRunStatus::Running {
        let message = "自动驾驶后台作业已结束且没有活跃 owner，已停止等待人工处理";
        state.run_status = project::AutopilotRunStatus::ErrorStopped;
        state.recovery_action = project::AutopilotRecoveryAction::WaitHumanDecision;
        state.error_message = message.to_string();
        state.last_action = message.to_string();
        state.last_action_at = chrono::Utc::now().to_rfc3339();
    }
    true
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
        let mut project = crate::load_project(&project_name)?;
        let (state_active, run_status, job_id, generation, job_owner) = {
            let state = project
                .workflow_state
                .autopilot_state
                .as_ref()
                .ok_or("自动驾驶状态不存在。".to_string())?;
            (
                state.active,
                state.run_status.clone(),
                state.job_id.clone(),
                state.job_generation,
                state.job_owner.clone(),
            )
        };
        if run_status == project::AutopilotRunStatus::Running
            && (!project.workflow_state.autopilot_active
                || !state_active
                || job_id.is_empty()
                || job_owner != project::AutopilotJobOwner::BackendRuntime)
        {
            let message = "自动驾驶处于 Running 但没有可证明的后端 owner，已停止等待人工处理";
            let state = project
                .workflow_state
                .autopilot_state
                .as_mut()
                .expect("autopilot state checked above");
            state.run_status = project::AutopilotRunStatus::ErrorStopped;
            state.job_owner = project::AutopilotJobOwner::None;
            state.recovery_action = project::AutopilotRecoveryAction::WaitHumanDecision;
            state.error_message = message.to_string();
            state.last_action = message.to_string();
            state.last_action_at = chrono::Utc::now().to_rfc3339();
            clear_action_claim(state);
            crate::save_project(&project)?;
            return Err(message.to_string());
        }
        if !state_active || run_status != project::AutopilotRunStatus::Running {
            return Ok(());
        }
        let mut jobs = self.jobs.lock().await;
        if jobs.get(&project_name).is_some_and(|existing| {
            !existing.handle.is_finished()
                && existing.job_id == job_id
                && existing.generation == generation
        }) {
            return Ok(());
        }
        let finished_same_job = jobs.get(&project_name).is_some_and(|existing| {
            existing.handle.is_finished()
                && existing.job_id == job_id
                && existing.generation == generation
        });
        let (job_id, generation) = if finished_same_job {
            let state = project
                .workflow_state
                .autopilot_state
                .as_mut()
                .expect("autopilot state checked above");
            assign_new_job_identity(state, "旧自动驾驶作业已结束，创建新的后端作业代次");
            let claimed_job_id = state.job_id.clone();
            let claimed_generation = state.job_generation;
            crate::save_project(&project)?;
            (claimed_job_id, claimed_generation)
        } else {
            (job_id, generation)
        };
        let task_project = project_name.clone();
        let task_job_id = job_id.clone();
        // Box the driver future so the large full-product state machine is heap-allocated
        // instead of inflating the Tokio worker stack during poll.
        let handle = tokio::spawn(async move {
            Box::pin(drive_project(
                pipeline_state,
                task_project,
                task_job_id,
                generation,
            ))
            .await;
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

    /// Abort registered in-process drivers without awaiting them during app exit.
    /// Tokio will also tear down any task that races this bounded best-effort pass.
    pub(crate) fn shutdown_nowait(&self) {
        let Ok(mut jobs) = self.jobs.try_lock() else {
            eprintln!("[lifecycle] 自动驾驶作业锁忙，交由 Tokio 退出收口");
            return;
        };
        for (_, job) in jobs.drain() {
            if !job.handle.is_finished() {
                job.handle.abort();
            }
        }
    }
}

/// Preserve an active backend owner fact before the application exits.
/// The owner is intentionally not cleared; startup reconciliation owns the
/// next decision and can distinguish this from a worker that vanished.
pub(crate) fn record_intentional_exit(project: &mut project::Project) -> bool {
    let Some(state) = project.workflow_state.autopilot_state.as_mut() else {
        return false;
    };
    if !state.active
        || state.job_id.is_empty()
        || state.job_owner != project::AutopilotJobOwner::BackendRuntime
        || state.last_action.starts_with("应用正常退出：")
    {
        return false;
    }
    let prior_action = if state.last_action.is_empty() {
        "当前动作未知".to_string()
    } else {
        state.last_action.clone()
    };
    state.last_action = format!(
        "应用正常退出：保留自动驾驶 owner={} generation={}；{}；重开时重新对账",
        state.job_id, state.job_generation, prior_action
    );
    state.last_action_at = chrono::Utc::now().to_rfc3339();
    true
}

/// Tauri 进程重启后，为仍可安全续跑的活动项目创建新一代作业身份。
/// 返回 true 表示调用方应在项目落盘并释放流水线锁后启动运行器。
pub(crate) fn reconcile_startup_job(project: &mut project::Project) -> bool {
    // Bound persistent recovery before deciding to resume the runtime job.
    let now = chrono::Utc::now();
    if project
        .workflow_state
        .autopilot_state
        .as_ref()
        .is_some_and(|state| {
            project.workflow_state.autopilot_active
                && state.active
                && state.run_status == project::AutopilotRunStatus::Running
                && state.job_owner == project::AutopilotJobOwner::BackendRuntime
                && state
                    .last_action
                    .starts_with("应用重启对账完成，后端自动驾驶继续运行")
        })
    {
        return true;
    }
    let stalled = crate::recovery::apply_stalled_recovery_reconciliation(project, now);
    if matches!(
        stalled,
        crate::recovery::StalledRecoveryDisposition::EnterHumanBoundary
            | crate::recovery::StalledRecoveryDisposition::MarkStalled
    ) {
        if let Some(state) = project.workflow_state.autopilot_state.as_mut() {
            state.job_owner = project::AutopilotJobOwner::None;
            state.current_action_id.clear();
            state.current_action_kind.clear();
            state.action_started_at.clear();
            if state.run_status == project::AutopilotRunStatus::Running {
                let message = "自动驾驶启动对账未找到可续跑 owner，已停止等待人工处理";
                state.run_status = project::AutopilotRunStatus::ErrorStopped;
                state.recovery_action = project::AutopilotRecoveryAction::WaitHumanDecision;
                state.error_message = message.to_string();
                state.last_action = message.to_string();
                state.last_action_at = now.to_rfc3339();
            }
        }
        return false;
    }

    let resumable_validation_recovery = project
        .workflow_state
        .recovery_state
        .as_ref()
        .is_some_and(crate::recovery::validation_retry_can_resume);
    let fresh_recovery_queue =
        project
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
                ) && !recovery.replan_execution_attempted
                    && crate::recovery::recovery_allows_automatic_claim(project, now)
            });
    let replan_already_attempted = project
        .workflow_state
        .recovery_state
        .as_ref()
        .is_some_and(|recovery| recovery.replan_attempted || recovery.replan_execution_attempted);
    let Some(state) = project.workflow_state.autopilot_state.as_mut() else {
        return false;
    };
    let automatically_recoverable = (matches!(
        state.recovery_action,
        project::AutopilotRecoveryAction::PrepareExecutionWorkspace
            | project::AutopilotRecoveryAction::RegenerateExecutionPlan
            | project::AutopilotRecoveryAction::RetryGitConfirmation
    ) && !replan_already_attempted)
        || (state.recovery_action == project::AutopilotRecoveryAction::RestoreExecutionBaseline
            && crate::autopilot_failure::is_transient(&state.last_failure_kind))
        || (state.recovery_action == project::AutopilotRecoveryAction::RunAutomaticRecovery
            && (resumable_validation_recovery || fresh_recovery_queue));
    let should_run = project.workflow_state.autopilot_active
        && state.active
        && (state.run_status == project::AutopilotRunStatus::Running
            || (state.run_status == project::AutopilotRunStatus::ErrorStopped
                && automatically_recoverable));
    if should_run {
        let now = now.to_rfc3339();
        assign_new_job_identity(state, "应用重启对账完成，后端自动驾驶继续运行");
        state.current_action_id.clear();
        state.current_action_kind.clear();
        state.action_started_at.clear();
        state.heartbeat_at = now.clone();
        true
    } else {
        state.job_owner = project::AutopilotJobOwner::None;
        state.current_action_id.clear();
        state.current_action_kind.clear();
        state.action_started_at.clear();
        if state.run_status == project::AutopilotRunStatus::Running {
            let message = "自动驾驶处于 Running 但启动对账未找到可续跑 owner，已停止等待人工处理";
            state.run_status = project::AutopilotRunStatus::ErrorStopped;
            state.recovery_action = project::AutopilotRecoveryAction::WaitHumanDecision;
            state.error_message = message.to_string();
            state.last_action = message.to_string();
            state.last_action_at = now.to_rfc3339();
        }
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
                && state.job_owner == project::AutopilotJobOwner::BackendRuntime
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

fn persist_heartbeat(
    project: &mut project::Project,
    job_id: &str,
    generation: u64,
) -> Result<(), String> {
    if !job_matches(project, job_id, generation) {
        return Err("自动驾驶 heartbeat 属于旧 owner，已拒绝回写。".to_string());
    }
    if let Some(state) = project.workflow_state.autopilot_state.as_mut() {
        state.heartbeat_at = chrono::Utc::now().to_rfc3339();
        state.job_owner = project::AutopilotJobOwner::BackendRuntime;
    }
    crate::save_project(project)
}

fn write_recovery_progress_history(
    project: &mut project::Project,
    event_type: project::ExecutionEventType,
    level: &str,
    text: String,
) {
    let (milestone_id, mid_stage_id) = project
        .execution_session
        .as_ref()
        .map(|session| (session.milestone_id.clone(), session.mid_stage_id.clone()))
        .unwrap_or_default();
    let subtask_id = project
        .workflow_state
        .recovery_state
        .as_ref()
        .map(|recovery| recovery.subtask_id.clone())
        .or_else(|| {
            project
                .execution_session
                .as_ref()
                .map(|session| session.subtask_id.clone())
        });
    crate::pipeline::write_execution_history_with_source(
        project,
        level,
        event_type,
        project::OperationSource::Recovery,
        text,
        (!milestone_id.is_empty()).then_some(milestone_id.as_str()),
        (!mid_stage_id.is_empty()).then_some(mid_stage_id.as_str()),
        subtask_id.as_deref(),
    );
}

fn update_recovery_progress_state(
    project: &mut project::Project,
    gate: RecoveryProgressGate,
) -> Option<(project::ExecutionEventType, &'static str, String)> {
    let state = project.workflow_state.autopilot_state.as_mut()?;
    let (marker, event_type, level, message) = match gate {
        RecoveryProgressGate::Warning => (
            "（警告）",
            project::ExecutionEventType::RecoveryWarning,
            "pause",
            format!(
                "恢复动作存活但超过 {} 秒无业务进展（警告）",
                RECOVERY_PROGRESS_WARNING_SECS
            ),
        ),
        RecoveryProgressGate::Stalled => (
            "（停滞）",
            project::ExecutionEventType::RecoveryStalled,
            "error",
            format!(
                "恢复动作存活但超过 {} 秒无业务进展（停滞）",
                RECOVERY_PROGRESS_STALLED_SECS
            ),
        ),
        RecoveryProgressGate::Running | RecoveryProgressGate::HardTimeout => return None,
    };
    if state.last_action.contains(marker) {
        return None;
    }
    state.last_action = message.clone();
    state.last_action_at = chrono::Utc::now().to_rfc3339();
    Some((event_type, level, message))
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
    /// Recovery (or other bounded) action hit hard wall-clock timeout.
    TimedOut {
        elapsed_secs: u64,
    },
}

async fn request_pipeline_owner_stop(
    pipeline_state: &Arc<Mutex<Option<PipelineState>>>,
    project_name: &str,
    execution_id: &str,
    reason: &str,
) {
    let mut guard = pipeline_state.lock().await;
    if let Some(pipeline) = guard.as_mut().filter(|pipeline| {
        pipeline.project_name == project_name
            && crate::pipeline::pipeline_owner_matches(Some(&**pipeline), execution_id)
    }) {
        pipeline.status = crate::pipeline::PipelineStatus::Failed;
        pipeline.last_error = Some(reason.to_string());
        crate::pipeline::append_log(pipeline, "error", reason.to_string());
    }
}

async fn dispatch_action_with_heartbeat(
    pipeline_state: &Arc<Mutex<Option<PipelineState>>>,
    project_name: &str,
    next: &workflow::AutopilotNextStep,
    job_id: &str,
    generation: u64,
    action_id: &str,
) -> DispatchOutcome {
    dispatch_action_with_heartbeat_limited(
        pipeline_state,
        project_name,
        next,
        job_id,
        generation,
        action_id,
        if is_recovery_dispatch_command(&next.command) {
            Some(Duration::from_secs(RECOVERY_ACTION_HARD_TIMEOUT_SECS))
        } else {
            None
        },
    )
    .await
}

async fn dispatch_action_with_heartbeat_limited(
    pipeline_state: &Arc<Mutex<Option<PipelineState>>>,
    project_name: &str,
    next: &workflow::AutopilotNextStep,
    job_id: &str,
    generation: u64,
    action_id: &str,
    hard_timeout: Option<Duration>,
) -> DispatchOutcome {
    let dispatch = dispatch_action(pipeline_state, project_name, next);
    tokio::pin!(dispatch);
    let mut heartbeat = interval(Duration::from_secs(1));
    // Non-recovery actions keep the historical unbounded wait; recovery uses ≤720s.
    let bound = hard_timeout.unwrap_or(Duration::from_secs(u64::MAX / 4));
    let hard_deadline = sleep(bound);
    tokio::pin!(hard_deadline);
    let started = std::time::Instant::now();
    let bounded = hard_timeout.is_some();
    loop {
        tokio::select! {
            result = &mut dispatch => return DispatchOutcome::Completed(result),
            _ = &mut hard_deadline, if bounded => {
                let elapsed_secs = started
                    .elapsed()
                    .as_secs()
                    .max(hard_timeout.map(|d| d.as_secs()).unwrap_or(0));
                let execution_id = crate::load_project(project_name)
                    .ok()
                    .filter(|project| {
                        action_claim_matches(project, job_id, generation, action_id, &next.command)
                    })
                    .and_then(|project| {
                        project
                            .workflow_state
                            .recovery_state
                            .map(|recovery| recovery.execution_id)
                    })
                    .filter(|execution_id| !execution_id.is_empty());
                let Some(execution_id) = execution_id else {
                    return DispatchOutcome::Superseded;
                };
                request_pipeline_owner_stop(
                    pipeline_state,
                    project_name,
                    &execution_id,
                    "恢复动作达到总墙钟，正在终止执行 owner",
                )
                .await;
                let _ = tokio::time::timeout(
                    Duration::from_secs(RECOVERY_OWNER_STOP_GRACE_SECS),
                    &mut dispatch,
                )
                .await;
                return DispatchOutcome::TimedOut { elapsed_secs };
            }
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
                if is_recovery_dispatch_command(&next.command) {
                    let action_started = latest
                        .workflow_state
                        .autopilot_state
                        .as_ref()
                        .and_then(|state| parse_rfc3339(&state.action_started_at));
                    let last_progress = latest
                        .workflow_state
                        .recovery_state
                        .as_ref()
                        .and_then(|recovery| parse_rfc3339(&recovery.updated_at));
                    let gate = classify_recovery_progress(
                        chrono::Utc::now(),
                        last_progress,
                        action_started,
                    );
                    if gate == RecoveryProgressGate::HardTimeout {
                        let elapsed_secs = started.elapsed().as_secs().max(
                            RECOVERY_ACTION_HARD_TIMEOUT_SECS
                        );
                        let execution_id = latest
                            .workflow_state
                            .recovery_state
                            .as_ref()
                            .map(|recovery| recovery.execution_id.as_str())
                            .filter(|execution_id| !execution_id.is_empty());
                        let Some(execution_id) = execution_id else {
                            return DispatchOutcome::Superseded;
                        };
                        request_pipeline_owner_stop(
                            pipeline_state,
                            project_name,
                            execution_id,
                            "恢复动作无业务进展并达到总墙钟，正在终止执行 owner",
                        )
                        .await;
                        let _ = tokio::time::timeout(
                            Duration::from_secs(RECOVERY_OWNER_STOP_GRACE_SECS),
                            &mut dispatch,
                        )
                        .await;
                        return DispatchOutcome::TimedOut { elapsed_secs };
                    }
                    if let Some((event_type, level, message)) =
                        update_recovery_progress_state(&mut latest, gate)
                    {
                        write_recovery_progress_history(&mut latest, event_type, level, message);
                    }
                }
                if persist_heartbeat(&mut latest, job_id, generation).is_err() {
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
            if project
                .workflow_state
                .recovery_state
                .as_ref()
                .is_some_and(|recovery| {
                    recovery.replan_attempted || recovery.replan_execution_attempted
                })
            {
                return Ok(false);
            }
            // Defensive: OutputTruncated must never whole-stage regenerate.
            // Only run_error_recovery → replan_current_subtask is legal.
            if project
                .workflow_state
                .recovery_state
                .as_ref()
                .is_some_and(|recovery| {
                    recovery.engine_failure_kind
                        == Some(project::EngineFailureKind::OutputTruncated)
                })
            {
                let mut latest = crate::load_project(&project.name)?;
                if let Some(state) = latest.workflow_state.autopilot_state.as_mut() {
                    state.run_status = project::AutopilotRunStatus::Running;
                    state.recovery_action = project::AutopilotRecoveryAction::RunAutomaticRecovery;
                    state.next_retry_at = None;
                    state.last_action = "内置执行截断已收敛到当前任务受限重规划".to_string();
                    state.last_action_at = chrono::Utc::now().to_rfc3339();
                    clear_action_claim(state);
                }
                crate::save_project(&latest)?;
                return Ok(true);
            }
            let scope = crate::plan_scope::PlanScope::resolve(project)?;
            let source =
                if project.workflow_state.current_step == project::WorkflowStep::PlanApproving {
                    "approval_rejected"
                } else {
                    "check_failed"
                };
            milestone::regenerate_execution_plan(
                project.name.clone(),
                project.workflow_state.data_revision,
                scope.plan_draft_revision(project),
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
                let _ = persist_heartbeat(&mut project, &job_id, generation);
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
            let _ = persist_heartbeat(&mut project, &job_id, generation);
            sleep(Duration::from_secs(1)).await;
            continue;
        }
        if !retry_due(project.workflow_state.autopilot_state.as_ref().unwrap()) {
            let _ = persist_heartbeat(&mut project, &job_id, generation);
            sleep(Duration::from_secs(1)).await;
            continue;
        }
        if let Some(state) = project.workflow_state.autopilot_state.as_mut() {
            state.next_retry_at = None;
        }
        if persist_heartbeat(&mut project, &job_id, generation).is_err() {
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
            DispatchOutcome::TimedOut { elapsed_secs } => {
                let mut latest = match crate::load_project(&project_name) {
                    Ok(project) => project,
                    Err(_) => break,
                };
                if !action_claim_matches(&latest, &job_id, generation, &action_id, &next.command) {
                    // Stale generation/action must not overwrite a newer owner.
                    break;
                }
                crate::recovery::apply_recovery_dispatch_timeout(
                    &mut latest,
                    &next.command,
                    elapsed_secs,
                );
                let _ = crate::save_project(&latest);
                break;
            }
            DispatchOutcome::Superseded => break,
        }
        sleep(Duration::from_secs(1)).await;
    }
    if let Ok(mut project) = crate::load_project(&project_name) {
        if let Some(state) = project.workflow_state.autopilot_state.as_mut() {
            if converge_finished_job_state(state, &job_id, generation) {
                let _ = crate::save_project(&project);
            }
        }
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

    fn ts(offset_secs: i64) -> chrono::DateTime<chrono::Utc> {
        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 8, 11, 12, 0, 0)
            .single()
            .expect("fixed test time")
            + chrono::Duration::seconds(offset_secs)
    }

    #[test]
    fn recovery_progress_gates_use_injected_time_boundaries() {
        let started = ts(0);
        // 89s idle: still running
        assert_eq!(
            classify_recovery_progress(ts(89), Some(started), Some(started)),
            RecoveryProgressGate::Running
        );
        // 90s: warning
        assert_eq!(
            classify_recovery_progress(ts(90), Some(started), Some(started)),
            RecoveryProgressGate::Warning
        );
        // 299s: still warning
        assert_eq!(
            classify_recovery_progress(ts(299), Some(started), Some(started)),
            RecoveryProgressGate::Warning
        );
        // 300s: stalled
        assert_eq!(
            classify_recovery_progress(ts(300), Some(started), Some(started)),
            RecoveryProgressGate::Stalled
        );
        // 719s action elapsed with recent progress: still running on progress, not hard yet
        assert_eq!(
            classify_recovery_progress(ts(719), Some(ts(700)), Some(started)),
            RecoveryProgressGate::Running
        );
        // 720s action elapsed: hard timeout even if progress was recent
        assert_eq!(
            classify_recovery_progress(ts(720), Some(ts(719)), Some(started)),
            RecoveryProgressGate::HardTimeout
        );
    }

    #[test]
    fn recovery_progress_events_are_emitted_once_per_boundary() {
        let mut project = active_project(project::AutopilotRunStatus::Running);
        project
            .workflow_state
            .autopilot_state
            .as_mut()
            .unwrap()
            .last_action = "恢复动作正在执行".to_string();
        project
            .workflow_state
            .autopilot_state
            .as_mut()
            .unwrap()
            .heartbeat_at = "2026-08-11T12:00:30Z".to_string();
        project.workflow_state.recovery_state = Some(project::RecoveryState {
            subtask_id: "sub-1".to_string(),
            ..Default::default()
        });

        let warning = update_recovery_progress_state(&mut project, RecoveryProgressGate::Warning)
            .expect("warning event");
        assert_eq!(
            project
                .workflow_state
                .autopilot_state
                .as_ref()
                .unwrap()
                .heartbeat_at,
            "2026-08-11T12:00:30Z"
        );
        assert_eq!(warning.0, project::ExecutionEventType::RecoveryWarning);
        write_recovery_progress_history(&mut project, warning.0, warning.1, warning.2);
        assert_eq!(project.execution_history.len(), 1);
        assert_eq!(
            project.execution_history[0].event_type,
            project::ExecutionEventType::RecoveryWarning
        );

        assert!(
            update_recovery_progress_state(&mut project, RecoveryProgressGate::Warning).is_none()
        );

        let stalled = update_recovery_progress_state(&mut project, RecoveryProgressGate::Stalled)
            .expect("stalled event");
        assert_eq!(stalled.0, project::ExecutionEventType::RecoveryStalled);
        write_recovery_progress_history(&mut project, stalled.0, stalled.1, stalled.2);
        assert_eq!(project.execution_history.len(), 2);
        assert_eq!(
            project.execution_history[1].event_type,
            project::ExecutionEventType::RecoveryStalled
        );
        assert!(
            update_recovery_progress_state(&mut project, RecoveryProgressGate::Stalled).is_none()
        );
    }

    #[test]
    fn recovery_dispatch_timeout_enters_human_boundary() {
        let mut project = active_project(project::AutopilotRunStatus::Running);
        {
            let state = project.workflow_state.autopilot_state.as_mut().unwrap();
            state.current_action_id = "action-timeout".to_string();
            state.current_action_kind = "run_error_recovery".to_string();
            state.action_started_at = "2026-08-11T12:00:00Z".to_string();
            state.heartbeat_at = "2026-08-11T12:11:59Z".to_string();
            state.recovery_action = project::AutopilotRecoveryAction::RunAutomaticRecovery;
        }
        project.workflow_state.recovery_state = Some(project::RecoveryState {
            error_kind: project::RecoveryErrorKind::PlanFailure,
            phase: project::RecoveryPhase::Replanning,
            subtask_id: "subtask-1".to_string(),
            execution_id: "exec-timeout".to_string(),
            engine_failure_kind: Some(project::EngineFailureKind::OutputTruncated),
            updated_at: "2026-08-11T12:00:00Z".to_string(),
            started_at: "2026-08-11T12:00:00Z".to_string(),
            ..Default::default()
        });
        project.execution_history = vec![];

        crate::recovery::apply_recovery_dispatch_timeout(
            &mut project,
            "run_error_recovery",
            RECOVERY_ACTION_HARD_TIMEOUT_SECS,
        );

        let recovery = project.workflow_state.recovery_state.as_ref().unwrap();
        assert_eq!(recovery.phase, project::RecoveryPhase::WaitingHuman);
        assert_eq!(
            recovery.error_kind,
            project::RecoveryErrorKind::HumanRequired
        );
        assert!(recovery.last_repair_summary.contains("超时"));
        assert!(recovery.last_repair_summary.contains("run_error_recovery"));

        let autopilot = project.workflow_state.autopilot_state.as_ref().unwrap();
        assert_eq!(
            autopilot.run_status,
            project::AutopilotRunStatus::ErrorStopped
        );
        assert_eq!(
            autopilot.recovery_action,
            project::AutopilotRecoveryAction::WaitHumanDecision
        );
        assert!(autopilot.current_action_id.is_empty());
        assert!(autopilot.current_action_kind.is_empty());
        assert!(autopilot.action_started_at.is_empty());
        assert_eq!(autopilot.job_owner, project::AutopilotJobOwner::None);
        // Last heartbeat preserved for diagnosis; not cleared.
        assert_eq!(autopilot.heartbeat_at, "2026-08-11T12:11:59Z");
        assert!(autopilot.next_retry_at.is_none());

        assert!(project
            .execution_history
            .iter()
            .any(|event| event.event_type == project::ExecutionEventType::RecoveryExhausted));

        // Stale action must not be applied by caller when claim mismatches — policy after
        // timeout must not auto-dispatch run_error_recovery.
        let decision = crate::autopilot_policy::decide_next_step(
            &project,
            "autopilot-runtime-test",
            &crate::autopilot_policy::AutopilotPolicyFacts {
                precondition_block: None,
                quality_gate: crate::autopilot_policy::QualityGateFact::NotApplicable,
                needs_calibration: false,
            },
        );
        assert_ne!(decision.next.command, "run_error_recovery");
        assert!(decision.next.is_error || decision.next.command.is_empty());
    }

    #[test]
    fn startup_replaces_stale_running_job_identity() {
        let mut project = active_project(project::AutopilotRunStatus::Running);
        assert!(reconcile_startup_job(&mut project));
        let state = project.workflow_state.autopilot_state.as_ref().unwrap();
        assert_ne!(state.job_id, "old-job");
        assert_eq!(state.job_generation, 8);
        assert_eq!(state.job_owner, project::AutopilotJobOwner::BackendRuntime);
        assert!(state.current_action_id.is_empty());
        assert!(state.current_action_kind.is_empty());
        assert!(!state.heartbeat_at.is_empty());

        let job_id = state.job_id.clone();
        let generation = state.job_generation;
        assert!(reconcile_startup_job(&mut project));
        let state = project.workflow_state.autopilot_state.unwrap();
        assert_eq!(state.job_id, job_id);
        assert_eq!(state.job_generation, generation);
    }

    #[test]
    fn startup_does_not_rerun_an_attempted_replan() {
        let mut project = active_project(project::AutopilotRunStatus::ErrorStopped);
        let state = project.workflow_state.autopilot_state.as_mut().unwrap();
        state.recovery_action = project::AutopilotRecoveryAction::RegenerateExecutionPlan;
        project.workflow_state.recovery_state = Some(project::RecoveryState {
            phase: project::RecoveryPhase::Diagnosing,
            replan_attempted: true,
            subtask_id: "subtask-1".to_string(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            ..Default::default()
        });

        assert!(!reconcile_startup_job(&mut project));
        let state = project.workflow_state.autopilot_state.unwrap();
        assert_eq!(state.run_status, project::AutopilotRunStatus::ErrorStopped);
        assert_eq!(state.job_owner, project::AutopilotJobOwner::None);
    }

    #[test]
    fn intentional_exit_keeps_autopilot_owner_and_generation() {
        let mut project = active_project(project::AutopilotRunStatus::Running);
        let before = project
            .workflow_state
            .autopilot_state
            .as_ref()
            .unwrap()
            .job_id
            .clone();

        assert!(record_intentional_exit(&mut project));
        assert!(!record_intentional_exit(&mut project));
        let state = project.workflow_state.autopilot_state.unwrap();
        assert_eq!(state.job_id, before);
        assert_eq!(state.job_owner, project::AutopilotJobOwner::BackendRuntime);
        assert!(state.last_action.starts_with("应用正常退出："));
    }

    #[test]
    fn autopilot_job_match_requires_current_generation_and_backend_owner() {
        let mut project = active_project(project::AutopilotRunStatus::Running);

        assert!(job_matches(&project, "old-job", 7));
        assert!(!job_matches(&project, "old-job", 6));
        assert!(!job_matches(&project, "stale-job", 7));

        project
            .workflow_state
            .autopilot_state
            .as_mut()
            .unwrap()
            .job_owner = project::AutopilotJobOwner::None;
        assert!(!job_matches(&project, "old-job", 7));

        project
            .workflow_state
            .autopilot_state
            .as_mut()
            .unwrap()
            .job_owner = project::AutopilotJobOwner::BackendRuntime;
        project.workflow_state.autopilot_active = false;
        assert!(!job_matches(&project, "old-job", 7));
    }

    #[test]
    fn finished_job_converges_only_its_own_generation() {
        let mut state = project::AutopilotState {
            active: true,
            run_status: project::AutopilotRunStatus::Running,
            job_id: "job-new".to_string(),
            job_generation: 2,
            job_owner: project::AutopilotJobOwner::BackendRuntime,
            current_action_id: "action-new".to_string(),
            current_action_kind: "execute_current_subtask".to_string(),
            ..Default::default()
        };

        assert!(!converge_finished_job_state(&mut state, "job-old", 1));
        assert_eq!(state.run_status, project::AutopilotRunStatus::Running);
        assert_eq!(state.job_owner, project::AutopilotJobOwner::BackendRuntime);

        assert!(converge_finished_job_state(&mut state, "job-new", 2));
        assert_eq!(state.run_status, project::AutopilotRunStatus::ErrorStopped);
        assert_eq!(state.job_owner, project::AutopilotJobOwner::None);
        assert!(state.current_action_id.is_empty());
        assert!(state.current_action_kind.is_empty());
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
