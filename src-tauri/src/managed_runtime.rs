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

impl ManagedRuntime {
    pub(crate) async fn start(self: &Arc<Self>, project_name: String) -> Result<(), String> {
        let project = crate::load_project(&project_name)?;
        let state = project
            .workflow_state
            .managed_flow_state
            .as_ref()
            .ok_or_else(|| "托管状态不存在。".to_string())?;
        if !state.active || state.run_status != project::ManagedRunStatus::Running {
            return Ok(());
        }
        if state.job_id.is_empty() {
            return Err("托管作业身份缺失，请重新启动托管。".to_string());
        }

        let job_id = state.job_id.clone();
        let generation = state.job_generation;
        let mut jobs = self.jobs.lock().await;
        if jobs.get(&project_name).is_some_and(|job| {
            !job.handle.is_finished() && job.job_id == job_id && job.generation == generation
        }) {
            return Ok(());
        }
        if let Some(existing) = jobs.remove(&project_name) {
            if !existing.handle.is_finished() {
                existing.handle.abort();
            }
        }

        let task_project = project_name.clone();
        let task_job_id = job_id.clone();
        let handle = tokio::spawn(async move {
            drive_project(task_project, task_job_id, generation).await;
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
        assign_new_job_identity(state, "应用重启对账完成，后端托管继续运行");
        return true;
    }
    state.current_action.clear();
    state.current_action_id.clear();
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

fn persist_heartbeat(project: &mut project::Project) -> Result<(), String> {
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
                if persist_heartbeat(&mut latest).is_err() {
                    return DispatchOutcome::Completed(Err("托管动作心跳持久化失败。".to_string()));
                }
            }
        }
    }
}

fn persist_waiting(project_name: &str, reason: &str) -> Result<(), String> {
    let mut project = crate::load_project(project_name)?;
    if let Some(state) = project.workflow_state.managed_flow_state.as_mut() {
        state.run_status = project::ManagedRunStatus::WaitingHuman;
        state.last_action = reason.to_string();
        state.last_action_at = chrono::Utc::now().to_rfc3339();
        state.current_action.clear();
        state.current_action_id.clear();
    }
    crate::save_project(&project)
}

fn persist_failure(project_name: &str, error: &str) -> Result<bool, String> {
    let mut project = crate::load_project(project_name)?;
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

async fn drive_project(project_name: String, job_id: String, generation: u64) {
    loop {
        let mut project = match crate::load_project(&project_name) {
            Ok(project) => project,
            Err(_) => break,
        };
        if !job_matches(&project, &job_id, generation) {
            break;
        }
        if persist_heartbeat(&mut project).is_err() {
            break;
        }

        let next = match workflow::managed_next_step(project_name.clone()).await {
            Ok(next) => next,
            Err(error) => {
                if !persist_failure(&project_name, &error).unwrap_or(false) {
                    break;
                }
                sleep(MANAGED_POLL_INTERVAL).await;
                continue;
            }
        };

        if next.reached_target {
            let _ = workflow::stop_managed_flow_state(project_name.clone()).await;
            break;
        }
        if next.needs_human {
            let _ = persist_waiting(&project_name, &next.description);
            break;
        }
        if next.is_error {
            let error = if next.error_message.is_empty() {
                next.description
            } else {
                next.error_message
            };
            let _ = persist_failure(&project_name, &error);
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
                let Some(state) = latest.workflow_state.managed_flow_state.as_mut() else {
                    break;
                };
                if state.job_id != job_id || state.job_generation != generation {
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
                if !persist_failure(&project_name, &error).unwrap_or(false) {
                    break;
                }
            }
            DispatchOutcome::TimedOut(timeout_secs) => {
                let error = format!("托管动作执行超时（超过 {} 秒）", timeout_secs);
                if !persist_failure(&project_name, &error).unwrap_or(false) {
                    break;
                }
            }
            DispatchOutcome::Superseded => break,
        }
        sleep(MANAGED_POLL_INTERVAL).await;
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
        let state = project.workflow_state.managed_flow_state.unwrap();
        assert_ne!(state.job_id, "legacy");
        assert_eq!(state.job_generation, 4);
        assert!(!state.heartbeat_at.is_empty());
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
