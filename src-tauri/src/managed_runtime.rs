use crate::commands::{milestone, plan, workflow};
use crate::project;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep, Duration};

const MANAGED_POLL_INTERVAL: Duration = Duration::from_millis(500);
const MANAGED_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const MANAGED_BASE_ACTION_TIMEOUT_SECS: u64 = 300;
const MANAGED_ACTION_TIMEOUT_BUFFER_SECS: u64 = 30;
const MANAGED_MAX_RETRIES: u32 = 2;

fn managed_model_request_count(command: &str) -> Option<u64> {
    match command {
        "generate_version_plan" | "check_milestone_draft" => Some(1),
        "generate_milestone_draft" | "regenerate_milestone_draft" => Some(3),
        _ => None,
    }
}

fn managed_action_timeout_with_setting(command: &str, decision_timeout_secs: u64) -> Duration {
    let Some(request_count) = managed_model_request_count(command) else {
        return Duration::from_secs(MANAGED_BASE_ACTION_TIMEOUT_SECS);
    };
    let model_budget = decision_timeout_secs
        .saturating_mul(request_count)
        .saturating_add(MANAGED_ACTION_TIMEOUT_BUFFER_SECS);
    Duration::from_secs(model_budget.max(MANAGED_BASE_ACTION_TIMEOUT_SECS))
}

fn managed_action_timeout(command: &str) -> Result<Duration, String> {
    let Some(_) = managed_model_request_count(command) else {
        return Ok(Duration::from_secs(MANAGED_BASE_ACTION_TIMEOUT_SECS));
    };
    let settings = crate::settings::settings_snapshot()
        .map_err(|error| format!("读取托管动作超时设置失败：{}", error))?;
    Ok(managed_action_timeout_with_setting(
        command,
        settings.decision_model.timeout_secs,
    ))
}

#[derive(Default)]
pub(crate) struct ManagedRuntime {
    jobs: Mutex<HashMap<String, ManagedJob>>,
}

struct ManagedJob {
    job_id: String,
    generation: u64,
    handle: JoinHandle<()>,
}

fn converge_finished_job_state(
    state: &mut project::ManagedFlowState,
    job_id: &str,
    generation: u64,
) -> bool {
    if state.job_id != job_id || state.job_generation != generation {
        return false;
    }
    state.current_action.clear();
    state.current_action_id.clear();
    if state.run_status == project::ManagedRunStatus::Running {
        let message = "托管后台作业已结束且没有活跃 owner，已停止等待人工处理";
        state.run_status = project::ManagedRunStatus::ErrorStopped;
        state.error_message = message.to_string();
        state.last_action = message.to_string();
        state.last_action_at = chrono::Utc::now().to_rfc3339();
    }
    true
}

impl ManagedRuntime {
    pub(crate) async fn start(self: &Arc<Self>, project_name: String) -> Result<(), String> {
        let mut project = crate::load_project(&project_name)?;
        let (active, run_status, job_id, generation) = {
            let state = project
                .workflow_state
                .managed_flow_state
                .as_ref()
                .ok_or_else(|| "托管状态不存在。".to_string())?;
            (
                state.active,
                state.run_status.clone(),
                state.job_id.clone(),
                state.job_generation,
            )
        };
        if run_status == project::ManagedRunStatus::Running && (!active || job_id.is_empty()) {
            let message = "托管层处于 Running 但没有可证明的后端 owner，已停止等待人工处理";
            let state = project
                .workflow_state
                .managed_flow_state
                .as_mut()
                .expect("managed state checked above");
            state.run_status = project::ManagedRunStatus::ErrorStopped;
            state.current_action.clear();
            state.current_action_id.clear();
            state.error_message = message.to_string();
            state.last_action = message.to_string();
            state.last_action_at = chrono::Utc::now().to_rfc3339();
            crate::save_project(&project)?;
            return Err(message.to_string());
        }
        if !active || run_status != project::ManagedRunStatus::Running {
            return Ok(());
        }
        let mut jobs = self.jobs.lock().await;
        if jobs.get(&project_name).is_some_and(|job| {
            !job.handle.is_finished() && job.job_id == job_id && job.generation == generation
        }) {
            return Ok(());
        }
        let finished_same_job = jobs.get(&project_name).is_some_and(|job| {
            job.handle.is_finished() && job.job_id == job_id && job.generation == generation
        });
        let (job_id, generation) = if finished_same_job {
            let state = project
                .workflow_state
                .managed_flow_state
                .as_mut()
                .expect("managed state checked above");
            assign_new_job_identity(state, "旧托管作业已结束，创建新的后端作业代次");
            let claimed_job_id = state.job_id.clone();
            let claimed_generation = state.job_generation;
            crate::save_project(&project)?;
            (claimed_job_id, claimed_generation)
        } else {
            (job_id, generation)
        };
        if let Some(existing) = jobs.remove(&project_name) {
            if !existing.handle.is_finished() {
                existing.handle.abort();
            }
        }

        let task_project = project_name.clone();
        let task_job_id = job_id.clone();
        let handle = tokio::spawn(async move {
            Box::pin(drive_project(task_project, task_job_id, generation)).await;
        });
        jobs.insert(
            project_name,
            ManagedJob {
                job_id,
                generation,
                handle,
            },
        );
        Ok(())
    }

    pub(crate) async fn start_if_active(
        self: &Arc<Self>,
        project_name: String,
    ) -> Result<bool, String> {
        let project = crate::load_project(&project_name)?;
        let should_start = project
            .workflow_state
            .managed_flow_state
            .as_ref()
            .is_some_and(|state| {
                state.active && state.run_status == project::ManagedRunStatus::Running
            });
        if should_start {
            self.start(project_name).await?;
        }
        Ok(should_start)
    }

    pub(crate) async fn stop(&self, project_name: &str) {
        if let Some(job) = self.jobs.lock().await.remove(project_name) {
            if !job.handle.is_finished() {
                job.handle.abort();
            }
        }
    }

    /// Abort registered in-process drivers without awaiting them during app exit.
    pub(crate) fn shutdown_nowait(&self) {
        let Ok(mut jobs) = self.jobs.try_lock() else {
            eprintln!("[lifecycle] 托管作业锁忙，交由 Tokio 退出收口");
            return;
        };
        for (_, job) in jobs.drain() {
            if !job.handle.is_finished() {
                job.handle.abort();
            }
        }
    }
}

/// Preserve an active managed owner fact before the application exits.
/// Do not clear the job identity; startup reconciliation must decide whether
/// it can create a new owner after reopening.
pub(crate) fn record_intentional_exit(project: &mut project::Project) -> bool {
    let Some(state) = project.workflow_state.managed_flow_state.as_mut() else {
        return false;
    };
    if !state.active
        || state.job_id.is_empty()
        || state.run_status != project::ManagedRunStatus::Running
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
        "应用正常退出：保留托管 owner={} generation={}；{}；重开时重新对账",
        state.job_id, state.job_generation, prior_action
    );
    state.last_action_at = chrono::Utc::now().to_rfc3339();
    true
}

pub(crate) fn assign_new_job_identity(state: &mut project::ManagedFlowState, action: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    state.job_id = uuid::Uuid::new_v4().to_string();
    state.job_generation = state.job_generation.saturating_add(1);
    state.current_action.clear();
    state.current_action_id.clear();
    state.heartbeat_at = now.clone();
    state.retry_count = 0;
    state.last_action = action.to_string();
    state.last_action_at = now;
    state.error_message.clear();
}

pub(crate) fn reconcile_startup_job(project: &mut project::Project) -> bool {
    let Some(state) = project.workflow_state.managed_flow_state.as_mut() else {
        return false;
    };
    if project.workflow_state.autopilot_active {
        state.run_status = project::ManagedRunStatus::ErrorStopped;
        state.error_message = "托管与自动驾驶同时激活，已停止托管等待人工对账。".to_string();
        state.current_action.clear();
        state.current_action_id.clear();
        return false;
    }
    if state.active && state.run_status == project::ManagedRunStatus::Running {
        if !state.job_id.is_empty()
            && state
                .last_action
                .starts_with("应用重启对账完成，后端托管继续运行")
        {
            return true;
        }
        assign_new_job_identity(state, "应用重启对账完成，后端托管继续运行");
        return true;
    }
    state.current_action.clear();
    state.current_action_id.clear();
    if state.run_status == project::ManagedRunStatus::Running {
        let message = "托管启动对账未找到可续跑 owner，已停止等待人工处理";
        state.run_status = project::ManagedRunStatus::ErrorStopped;
        state.error_message = message.to_string();
        state.last_action = message.to_string();
        state.last_action_at = chrono::Utc::now().to_rfc3339();
    }
    false
}

fn job_matches(project: &project::Project, job_id: &str, generation: u64) -> bool {
    project
        .workflow_state
        .managed_flow_state
        .as_ref()
        .is_some_and(|state| {
            state.active
                && state.run_status == project::ManagedRunStatus::Running
                && state.job_id == job_id
                && state.job_generation == generation
        })
}

fn persist_heartbeat(
    project: &mut project::Project,
    job_id: &str,
    generation: u64,
) -> Result<(), String> {
    if !job_matches(project, job_id, generation) {
        return Err("托管 heartbeat 属于旧 owner，已拒绝回写。".to_string());
    }
    if let Some(state) = project.workflow_state.managed_flow_state.as_mut() {
        state.heartbeat_at = chrono::Utc::now().to_rfc3339();
    }
    crate::save_project(project)
}

async fn dispatch_action(project_name: &str, command: &str) -> Result<(), String> {
    match command {
        "generate_version_plan" => {
            let project = crate::load_project(project_name)?;
            plan::generate_version_plan(
                project_name.to_string(),
                project.discussion_revision,
                project.workflow_state.data_revision,
            )
            .await?;
        }
        "approve_version_plan" => {
            let project = crate::load_project(project_name)?;
            let draft = project
                .plan_draft
                .as_ref()
                .ok_or_else(|| "没有可批准的项目方案草稿。".to_string())?;
            plan::approve_version_plan(
                project_name.to_string(),
                draft.draft_id.clone(),
                draft.generation_revision,
            )
            .await?;
        }
        "enter_console" => {
            plan::enter_console(project_name.to_string()).await?;
        }
        "generate_milestone_draft" => {
            milestone::generate_milestone_draft(project_name.to_string()).await?;
        }
        "check_milestone_draft" => {
            milestone::check_milestone_draft(project_name.to_string()).await?;
        }
        "regenerate_milestone_draft" => {
            let project = crate::load_project(project_name)?;
            let draft = project
                .milestone_draft
                .as_ref()
                .ok_or_else(|| "没有可重新生成的大阶段草稿。".to_string())?;
            let feedback = draft
                .check_result
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "大阶段检查失败后缺少重生成反馈。".to_string())?;
            milestone::regenerate_milestone_draft(
                project_name.to_string(),
                draft.draft_id.clone(),
                project.workflow_state.data_revision,
                feedback.to_string(),
                "check_failed".to_string(),
            )
            .await?;
        }
        "approve_milestone_draft" => {
            milestone::approve_milestone_draft(project_name.to_string()).await?;
        }
        other => return Err(format!("托管运行器不支持动作：{other}")),
    }
    Ok(())
}

enum DispatchOutcome {
    Completed(Result<(), String>),
    Superseded,
    TimedOut(u64),
}

async fn dispatch_with_heartbeat(
    project_name: &str,
    command: &str,
    job_id: &str,
    generation: u64,
    action_id: &str,
) -> DispatchOutcome {
    let action_timeout = match managed_action_timeout(command) {
        Ok(timeout) => timeout,
        Err(error) => return DispatchOutcome::Completed(Err(error)),
    };
    let dispatch = dispatch_action(project_name, command);
    tokio::pin!(dispatch);
    let deadline = sleep(action_timeout);
    tokio::pin!(deadline);
    let mut heartbeat = interval(MANAGED_HEARTBEAT_INTERVAL);

    loop {
        tokio::select! {
            result = &mut dispatch => return DispatchOutcome::Completed(result),
            _ = &mut deadline => return DispatchOutcome::TimedOut(action_timeout.as_secs()),
            _ = heartbeat.tick() => {
                let mut latest = match crate::load_project(project_name) {
                    Ok(project) => project,
                    Err(error) => return DispatchOutcome::Completed(Err(error)),
                };
                let claim_matches = job_matches(&latest, job_id, generation)
                    && latest.workflow_state.managed_flow_state.as_ref().is_some_and(|state| {
                        state.current_action_id == action_id && state.current_action == command
                    });
                if !claim_matches {
                    return DispatchOutcome::Superseded;
                }
                if persist_heartbeat(&mut latest, job_id, generation).is_err() {
                    return DispatchOutcome::Completed(Err("托管动作心跳持久化失败。".to_string()));
                }
            }
        }
    }
}

fn persist_waiting(
    project_name: &str,
    job_id: &str,
    generation: u64,
    reason: &str,
) -> Result<(), String> {
    let mut project = crate::load_project(project_name)?;
    if !job_matches(&project, job_id, generation) {
        return Ok(());
    }
    if let Some(state) = project.workflow_state.managed_flow_state.as_mut() {
        state.run_status = project::ManagedRunStatus::WaitingHuman;
        state.last_action = reason.to_string();
        state.last_action_at = chrono::Utc::now().to_rfc3339();
        state.current_action.clear();
        state.current_action_id.clear();
    }
    crate::save_project(&project)
}

fn persist_failure(
    project_name: &str,
    job_id: &str,
    generation: u64,
    error: &str,
) -> Result<bool, String> {
    let mut project = crate::load_project(project_name)?;
    if !job_matches(&project, job_id, generation) {
        return Ok(false);
    }
    let Some(state) = project.workflow_state.managed_flow_state.as_mut() else {
        return Ok(false);
    };
    state.retry_count = state.retry_count.saturating_add(1);
    state.error_message = error.to_string();
    state.current_action.clear();
    state.current_action_id.clear();
    state.last_action_at = chrono::Utc::now().to_rfc3339();
    let retry = state.retry_count <= MANAGED_MAX_RETRIES;
    if retry {
        state.last_action = format!(
            "托管动作失败，准备第 {}/{} 次重试",
            state.retry_count, MANAGED_MAX_RETRIES
        );
    } else {
        state.run_status = project::ManagedRunStatus::ErrorStopped;
        state.last_action = "托管动作重试耗尽，已停止".to_string();
    }
    crate::save_project(&project)?;
    Ok(retry)
}

fn persist_terminal_failure(
    project_name: &str,
    job_id: &str,
    generation: u64,
    error: &str,
) -> Result<(), String> {
    let mut project = crate::load_project(project_name)?;
    if !job_matches(&project, job_id, generation) {
        return Ok(());
    }
    let Some(state) = project.workflow_state.managed_flow_state.as_mut() else {
        return Ok(());
    };
    state.run_status = project::ManagedRunStatus::ErrorStopped;
    state.error_message = error.chars().take(2_000).collect();
    state.current_action.clear();
    state.current_action_id.clear();
    state.last_action = "托管流程无法继续，已停止，等待人工处理".to_string();
    state.last_action_at = chrono::Utc::now().to_rfc3339();
    crate::save_project(&project)
}

async fn drive_project(project_name: String, job_id: String, generation: u64) {
    loop {
        let mut project = match crate::load_project(&project_name) {
            Ok(project) => project,
            Err(_) => break,
        };
        if !job_matches(&project, &job_id, generation) {
            break;
        }
        if persist_heartbeat(&mut project, &job_id, generation).is_err() {
            break;
        }

        let next = match workflow::managed_next_step(project_name.clone()).await {
            Ok(next) => next,
            Err(error) => {
                if !persist_failure(&project_name, &job_id, generation, &error).unwrap_or(false) {
                    break;
                }
                sleep(MANAGED_POLL_INTERVAL).await;
                continue;
            }
        };

        let latest = match crate::load_project(&project_name) {
            Ok(project) => project,
            Err(_) => break,
        };
        if !job_matches(&latest, &job_id, generation) {
            break;
        }

        if next.reached_target {
            let _ = workflow::stop_managed_flow_state(project_name.clone()).await;
            break;
        }
        if next.needs_human {
            let _ = persist_waiting(&project_name, &job_id, generation, &next.description);
            break;
        }
        if next.is_error {
            let error = if next.error_message.is_empty() {
                next.description
            } else {
                next.error_message
            };
            let _ = persist_terminal_failure(&project_name, &job_id, generation, &error);
            break;
        }
        if next.command.is_empty() {
            sleep(MANAGED_POLL_INTERVAL).await;
            continue;
        }

        let mut claimed = match crate::load_project(&project_name) {
            Ok(project) => project,
            Err(_) => break,
        };
        if !job_matches(&claimed, &job_id, generation) {
            break;
        }
        let action_id = uuid::Uuid::new_v4().to_string();
        if let Some(state) = claimed.workflow_state.managed_flow_state.as_mut() {
            state.current_action = next.command.clone();
            state.current_action_id = action_id.clone();
            state.last_action = next.description.clone();
            state.last_action_at = chrono::Utc::now().to_rfc3339();
            state.heartbeat_at = state.last_action_at.clone();
        }
        if crate::save_project(&claimed).is_err() {
            break;
        }

        match dispatch_with_heartbeat(
            &project_name,
            &next.command,
            &job_id,
            generation,
            &action_id,
        )
        .await
        {
            DispatchOutcome::Completed(Ok(())) => {
                let Ok(mut latest) = crate::load_project(&project_name) else {
                    break;
                };
                if !job_matches(&latest, &job_id, generation) {
                    break;
                }
                let Some(state) = latest.workflow_state.managed_flow_state.as_mut() else {
                    break;
                };
                if state.current_action_id != action_id
                    || state.current_action.as_str() != next.command.as_str()
                {
                    break;
                }
                state.last_completed_action = next.command;
                state.current_action.clear();
                state.current_action_id.clear();
                state.retry_count = 0;
                state.error_message.clear();
                state.heartbeat_at = chrono::Utc::now().to_rfc3339();
                state.managed_state = format!("{:?}", latest.workflow_state.current_step);
                if crate::save_project(&latest).is_err() {
                    break;
                }
            }
            DispatchOutcome::Completed(Err(error)) => {
                if !persist_failure(&project_name, &job_id, generation, &error).unwrap_or(false) {
                    break;
                }
            }
            DispatchOutcome::TimedOut(timeout_secs) => {
                let error = format!("托管动作执行超时（超过 {} 秒）", timeout_secs);
                if !persist_failure(&project_name, &job_id, generation, &error).unwrap_or(false) {
                    break;
                }
            }
            DispatchOutcome::Superseded => break,
        }
        sleep(MANAGED_POLL_INTERVAL).await;
    }
    if let Ok(mut project) = crate::load_project(&project_name) {
        if let Some(state) = project.workflow_state.managed_flow_state.as_mut() {
            if converge_finished_job_state(state, &job_id, generation) {
                let _ = crate::save_project(&project);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_reconciliation_assigns_a_new_backend_job_identity() {
        let mut project = project::Project::new("managed-startup");
        project.workflow_state.managed_flow_state = Some(project::ManagedFlowState {
            active: true,
            run_status: project::ManagedRunStatus::Running,
            job_id: "legacy".to_string(),
            job_generation: 3,
            ..Default::default()
        });

        assert!(reconcile_startup_job(&mut project));
        let state = project.workflow_state.managed_flow_state.as_ref().unwrap();
        assert_ne!(state.job_id, "legacy");
        assert_eq!(state.job_generation, 4);
        assert!(!state.heartbeat_at.is_empty());

        let job_id = state.job_id.clone();
        let generation = state.job_generation;
        assert!(reconcile_startup_job(&mut project));
        let state = project.workflow_state.managed_flow_state.unwrap();
        assert_eq!(state.job_id, job_id);
        assert_eq!(state.job_generation, generation);
    }

    #[test]
    fn intentional_exit_keeps_managed_owner_and_generation() {
        let mut project = project::Project::new("managed-intentional-exit");
        project.workflow_state.managed_flow_state = Some(project::ManagedFlowState {
            active: true,
            run_status: project::ManagedRunStatus::Running,
            job_id: "managed-job".to_string(),
            job_generation: 4,
            last_action: "生成方案".to_string(),
            ..Default::default()
        });

        assert!(record_intentional_exit(&mut project));
        assert!(!record_intentional_exit(&mut project));
        let state = project.workflow_state.managed_flow_state.unwrap();
        assert_eq!(state.job_id, "managed-job");
        assert_eq!(state.job_generation, 4);
        assert!(state.last_action.starts_with("应用正常退出："));
    }

    #[test]
    fn finished_job_converges_only_its_own_generation() {
        let mut state = project::ManagedFlowState {
            active: true,
            run_status: project::ManagedRunStatus::Running,
            job_id: "job-new".to_string(),
            job_generation: 2,
            current_action: "approve_milestone_draft".to_string(),
            current_action_id: "action-new".to_string(),
            ..Default::default()
        };

        assert!(!converge_finished_job_state(&mut state, "job-old", 1));
        assert_eq!(state.run_status, project::ManagedRunStatus::Running);
        assert_eq!(state.current_action_id, "action-new");

        assert!(converge_finished_job_state(&mut state, "job-new", 2));
        assert_eq!(state.run_status, project::ManagedRunStatus::ErrorStopped);
        assert!(state.current_action.is_empty());
        assert!(state.current_action_id.is_empty());
    }

    #[test]
    fn managed_job_matches_rejects_old_generation() {
        let mut project = project::Project::new("managed-generation");
        project.workflow_state.managed_flow_state = Some(project::ManagedFlowState {
            active: true,
            run_status: project::ManagedRunStatus::Running,
            job_id: "job-new".to_string(),
            job_generation: 2,
            ..Default::default()
        });

        assert!(!job_matches(&project, "job-old", 1));
        assert!(!job_matches(&project, "job-new", 1));
        assert!(job_matches(&project, "job-new", 2));

        let state = project.workflow_state.managed_flow_state.as_mut().unwrap();
        state.run_status = project::ManagedRunStatus::ErrorStopped;
        assert!(!job_matches(&project, "job-new", 2));
        let state = project.workflow_state.managed_flow_state.as_mut().unwrap();
        state.run_status = project::ManagedRunStatus::Running;
        state.active = false;
        assert!(!job_matches(&project, "job-new", 2));
    }

    #[test]
    fn managed_action_timeout_tracks_model_setting_and_request_budget() {
        assert_eq!(
            managed_action_timeout_with_setting("check_milestone_draft", 120).as_secs(),
            300
        );
        assert_eq!(
            managed_action_timeout_with_setting("generate_version_plan", 3_600).as_secs(),
            3_630
        );
        assert_eq!(
            managed_action_timeout_with_setting("generate_milestone_draft", 120).as_secs(),
            390
        );
        assert_eq!(
            managed_action_timeout_with_setting("regenerate_milestone_draft", 3_600).as_secs(),
            10_830
        );
        assert_eq!(
            managed_action_timeout_with_setting("approve_milestone_draft", 3_600).as_secs(),
            300
        );
    }

    #[tokio::test]
    async fn managed_runtime_supports_milestone_regeneration_action() -> Result<(), String> {
        let project_name = format!("test-managed-regeneration-{}", uuid::Uuid::new_v4());
        let path = crate::project_data_path(&project_name)?;
        let mut project = project::Project::new(&project_name);
        project.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        project.workflow_state.current_step = project::WorkflowStep::MilestoneCheck;
        crate::save_project(&project)?;

        let error = dispatch_action(&project_name, "regenerate_milestone_draft")
            .await
            .expect_err("缺少草稿时应由重生成分支返回业务错误");
        assert!(error.contains("没有可重新生成的大阶段草稿"));
        assert!(!error.contains("不支持动作"));
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[tokio::test]
    async fn terminal_managed_failure_stops_without_leaving_running_state() -> Result<(), String> {
        let project_name = format!("test-managed-terminal-failure-{}", uuid::Uuid::new_v4());
        let path = crate::project_data_path(&project_name)?;
        let mut project = project::Project::new(&project_name);
        project.workflow_state.managed_flow_state = Some(project::ManagedFlowState {
            active: true,
            run_status: project::ManagedRunStatus::Running,
            ..Default::default()
        });
        assign_new_job_identity(
            project.workflow_state.managed_flow_state.as_mut().unwrap(),
            "test",
        );
        let job_id = project
            .workflow_state
            .managed_flow_state
            .as_ref()
            .unwrap()
            .job_id
            .clone();
        crate::save_project(&project)?;

        let state = project
            .workflow_state
            .managed_flow_state
            .as_ref()
            .ok_or("托管状态缺失")?;
        persist_terminal_failure(
            &project_name,
            &state.job_id,
            state.job_generation,
            "模型连接失败：等待人工处理",
        )?;

        let stored = crate::load_project(&project_name)?;
        let state = stored
            .workflow_state
            .managed_flow_state
            .ok_or("托管状态缺失")?;
        assert_eq!(state.run_status, project::ManagedRunStatus::ErrorStopped);
        assert!(state.active);
        assert_eq!(state.job_id, job_id);
        assert_eq!(state.error_message, "模型连接失败：等待人工处理");
        assert!(state.current_action.is_empty());
        assert!(state.current_action_id.is_empty());
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[tokio::test]
    async fn backend_runtime_finishes_a_reached_target_without_a_ui_loop() -> Result<(), String> {
        let project_name = format!("test-managed-runtime-{}", uuid::Uuid::new_v4());
        let path = crate::project_data_path(&project_name)?;
        let mut project = project::Project::new(&project_name);
        project.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        project.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
        project.workflow_state.managed_flow_state = Some(project::ManagedFlowState {
            active: true,
            run_status: project::ManagedRunStatus::Running,
            ..Default::default()
        });
        assign_new_job_identity(
            project.workflow_state.managed_flow_state.as_mut().unwrap(),
            "test",
        );
        crate::save_project(&project)?;

        let runtime = Arc::new(ManagedRuntime::default());
        runtime.start(project_name.clone()).await?;
        runtime.start(project_name.clone()).await?;
        sleep(Duration::from_millis(150)).await;
        let stored = crate::load_project(&project_name)?;
        assert!(stored.workflow_state.managed_flow_state.is_none());
        runtime.stop(&project_name).await;
        let _ = std::fs::remove_file(path);
        Ok(())
    }
}
