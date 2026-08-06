use crate::control_action::{ControlActionKind, ControlActionLifecycle};
use crate::pipeline::PipelineState;
use crate::project;
use crate::task_control::{ControlActionLease, TaskControlState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) const CONTROL_ACTION_HEARTBEAT_INTERVAL_SECS: u64 = 2;
pub(crate) const CONTROL_ACTION_STALE_AFTER_SECS: i64 = 15;
pub(crate) const CONTROL_ACTION_MAX_EXECUTION_SECS: u64 = 12 * 60;

const CONTROL_ACTION_SIMPLE_EXPECTED_SECS: u64 = 120;
const CONTROL_ACTION_VALIDATION_EXPECTED_SECS: u64 = 5 * 60;
const CONTROL_ACTION_EXECUTION_EXPECTED_SECS: u64 = 10 * 60;
const CONTROL_ACTION_REPAIR_EXPECTED_SECS: u64 = 10 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ControlActionOccupancy {
    Unoccupied,
    ActiveLocal(ControlActionLease),
    ActiveForeign(ControlActionLease),
    Stale {
        lease: Option<ControlActionLease>,
        reason: String,
    },
}

pub(crate) fn expected_action_duration_secs(action: ControlActionKind) -> u64 {
    match action {
        ControlActionKind::Execute => CONTROL_ACTION_EXECUTION_EXPECTED_SECS,
        ControlActionKind::LocalValidate
        | ControlActionKind::AutomatedValidate
        | ControlActionKind::TargetedValidate
        | ControlActionKind::GitConfirm => CONTROL_ACTION_VALIDATION_EXPECTED_SECS,
        ControlActionKind::Repair => CONTROL_ACTION_REPAIR_EXPECTED_SECS,
        ControlActionKind::Split
        | ControlActionKind::Recompile
        | ControlActionKind::AcceptDeviation
        | ControlActionKind::Wait
        | ControlActionKind::Human => CONTROL_ACTION_SIMPLE_EXPECTED_SECS,
    }
}

pub(crate) fn classify_control_action_occupancy(
    state: &TaskControlState,
    current_process_start_id: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> ControlActionOccupancy {
    let Some(lease) = state.active_action_lease.as_ref() else {
        return if state.active_action_id.is_empty() {
            ControlActionOccupancy::Unoccupied
        } else {
            ControlActionOccupancy::Stale {
                lease: None,
                reason: "旧格式控制锁缺少持有者与心跳".to_string(),
            }
        };
    };
    if lease.action_id.trim().is_empty()
        || lease.owner_process_start_id.trim().is_empty()
        || lease.action_kind.trim().is_empty()
        || lease.started_at.trim().is_empty()
        || lease.heartbeat_at.trim().is_empty()
    {
        return ControlActionOccupancy::Stale {
            lease: Some(lease.clone()),
            reason: "控制锁租约字段不完整".to_string(),
        };
    }
    if (!state.active_action_id.is_empty() && state.active_action_id != lease.action_id)
        || (!state.active_action_kind.is_empty() && state.active_action_kind != lease.action_kind)
        || (!state.active_action_task_id.is_empty() && state.active_action_task_id != lease.task_id)
    {
        return ControlActionOccupancy::Stale {
            lease: Some(lease.clone()),
            reason: "结构化租约与兼容锁字段不一致".to_string(),
        };
    }
    let heartbeat = match chrono::DateTime::parse_from_rfc3339(&lease.heartbeat_at) {
        Ok(value) => value.with_timezone(&chrono::Utc),
        Err(_) => {
            return ControlActionOccupancy::Stale {
                lease: Some(lease.clone()),
                reason: "控制锁心跳时间无效".to_string(),
            };
        }
    };
    let started = match chrono::DateTime::parse_from_rfc3339(&lease.started_at) {
        Ok(value) => value.with_timezone(&chrono::Utc),
        Err(_) => {
            return ControlActionOccupancy::Stale {
                lease: Some(lease.clone()),
                reason: "控制锁开始时间无效".to_string(),
            };
        }
    };
    if now.signed_duration_since(heartbeat).num_seconds() > CONTROL_ACTION_STALE_AFTER_SECS {
        return ControlActionOccupancy::Stale {
            lease: Some(lease.clone()),
            reason: "控制动作心跳已超时".to_string(),
        };
    }
    let expected = lease
        .expected_max_duration_secs
        .max(1)
        .min(CONTROL_ACTION_MAX_EXECUTION_SECS);
    if now.signed_duration_since(started).num_seconds() > expected as i64 {
        return ControlActionOccupancy::Stale {
            lease: Some(lease.clone()),
            reason: "控制动作已超过预期最长执行时长".to_string(),
        };
    }
    if lease.owner_process_start_id == current_process_start_id {
        ControlActionOccupancy::ActiveLocal(lease.clone())
    } else {
        ControlActionOccupancy::ActiveForeign(lease.clone())
    }
}

fn install_action_lease(
    state: &mut TaskControlState,
    request: &ControlActionRequest,
    owner_process_start_id: &str,
    now: &str,
) {
    let lease = ControlActionLease {
        action_id: request.action_id.clone(),
        owner_process_start_id: owner_process_start_id.to_string(),
        action_kind: request.action.as_str().to_string(),
        task_id: request.task_id.clone(),
        started_at: now.to_string(),
        heartbeat_at: now.to_string(),
        expected_max_duration_secs: expected_action_duration_secs(request.action),
    };
    state.active_action_id = lease.action_id.clone();
    state.active_action_kind = lease.action_kind.clone();
    state.active_action_task_id = lease.task_id.clone();
    state.active_action_lease = Some(lease);
}

pub(crate) fn clear_action_lease(state: &mut TaskControlState, reason: &str, cleared_at: &str) {
    state.active_action_lease = None;
    state.active_action_id.clear();
    state.active_action_kind.clear();
    state.active_action_task_id.clear();
    state.last_action_clear_reason = reason.to_string();
    state.last_action_cleared_at = Some(cleared_at.to_string());
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ControlActionLockReconciliation {
    Unchanged,
    Cleared {
        action_id: String,
        completed: bool,
        needs_human_confirmation: bool,
        reason: String,
        post_task_state: String,
    },
}

impl ControlActionLockReconciliation {
    pub(crate) fn changed(&self) -> bool {
        matches!(self, Self::Cleared { .. })
    }
}

pub(crate) fn stale_control_action_can_be_cleared(
    lease: Option<&ControlActionLease>,
    _current_process_start_id: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    !lease.is_some_and(|value| {
        chrono::DateTime::parse_from_rfc3339(&value.heartbeat_at)
            .ok()
            .map(|heartbeat| {
                now.signed_duration_since(heartbeat.with_timezone(&chrono::Utc))
                    .num_seconds()
                    <= CONTROL_ACTION_STALE_AFTER_SECS
            })
            .unwrap_or(false)
    })
}

fn known_control_action_kind(kind: &str) -> bool {
    matches!(
        kind,
        "split"
            | "execute"
            | "local_validate"
            | "automated_validate"
            | "targeted_validate"
            | "repair"
            | "recompile"
            | "accept_deviation"
            | "git_confirm"
            | "wait"
            | "human"
    )
}

fn task_state_label(project: &project::Project, task_id: &str) -> String {
    if task_id.is_empty() {
        return "unscoped".to_string();
    }
    match crate::task_tree::find_task(project, task_id) {
        Ok(Some(task)) => format!("{:?}", task.status),
        Ok(None) => "task_missing".to_string(),
        Err(_) => "task_state_unavailable".to_string(),
    }
}

fn has_persisted_action_completion(
    project: &project::Project,
    action_id: &str,
    action_kind: &str,
    task_id: &str,
    made_progress: bool,
) -> bool {
    if !action_id.is_empty()
        && (project.task_control.last_completed_action_id == action_id
            || project.execution_history.iter().rev().any(|entry| {
                entry.action_id.as_deref() == Some(action_id) && entry.level == "success"
            }))
    {
        return true;
    }
    if action_kind == ControlActionKind::GitConfirm.as_str() {
        return crate::task_tree::find_task(project, task_id)
            .ok()
            .flatten()
            .is_some_and(|task| task.status == project::SubtaskStatus::Passed);
    }
    made_progress
        && matches!(
            action_kind,
            "split"
                | "execute"
                | "local_validate"
                | "automated_validate"
                | "targeted_validate"
                | "recompile"
                | "accept_deviation"
                | "wait"
                | "human"
        )
}

/// 对已加载项目中的控制动作锁做后端裁决。调用方必须在控制动作文件锁下持久化结果。
/// 新鲜的其他进程租约始终保留；本进程仍有新鲜心跳的超时动作也先等待正常收口。
pub(crate) fn reconcile_stale_control_action_lock(
    project: &mut project::Project,
    current_process_start_id: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> ControlActionLockReconciliation {
    let occupancy =
        classify_control_action_occupancy(&project.task_control, current_process_start_id, now);
    let (lease, reason) = match occupancy {
        ControlActionOccupancy::Unoccupied
        | ControlActionOccupancy::ActiveLocal(_)
        | ControlActionOccupancy::ActiveForeign(_) => {
            return ControlActionLockReconciliation::Unchanged;
        }
        ControlActionOccupancy::Stale { lease, reason } => (lease, reason),
    };

    if !stale_control_action_can_be_cleared(lease.as_ref(), current_process_start_id, now) {
        return ControlActionLockReconciliation::Unchanged;
    }

    let action_id = lease
        .as_ref()
        .map(|value| value.action_id.clone())
        .unwrap_or_else(|| project.task_control.active_action_id.clone());
    let action_kind = lease
        .as_ref()
        .map(|value| value.action_kind.clone())
        .unwrap_or_else(|| project.task_control.active_action_kind.clone());
    let task_id = lease
        .as_ref()
        .map(|value| value.task_id.clone())
        .unwrap_or_else(|| project.task_control.active_action_task_id.clone());
    let owner_process_start_id = lease
        .as_ref()
        .map(|value| value.owner_process_start_id.clone());
    let heartbeat_at = lease.as_ref().map(|value| value.heartbeat_at.clone());
    let after_fingerprint = control_fingerprint(project, &task_id).unwrap_or_default();
    let made_progress = !project
        .task_control
        .last_action_before_fingerprint
        .is_empty()
        && !after_fingerprint.is_empty()
        && project.task_control.last_action_before_fingerprint != after_fingerprint;
    let completed =
        has_persisted_action_completion(project, &action_id, &action_kind, &task_id, made_progress);
    let needs_human_confirmation = lease.is_none()
        || action_id.is_empty()
        || !known_control_action_kind(&action_kind)
        || (action_kind == ControlActionKind::Repair.as_str() && made_progress);

    let cleared_at = now.to_rfc3339();
    let clear_reason = format!("stale_reconciliation: {}", reason);
    clear_action_lease(&mut project.task_control, &clear_reason, &cleared_at);
    if completed {
        project.task_control.last_completed_action_id = action_id.clone();
        project.task_control.last_completed_action_kind = action_kind.clone();
        project.task_control.last_completed_action_task_id = task_id.clone();
        project.task_control.last_action_result =
            "陈旧锁已清理，磁盘完成事实已由对账确认".to_string();
        project.task_control.last_action_made_progress = made_progress;
        project.task_control.last_action_after_fingerprint = after_fingerprint;
        project.task_control.last_action_at = Some(cleared_at.clone());
    } else {
        project.task_control.last_decision = None;
        project.task_control.last_decision_id.clear();
        project.task_control.last_decision_fingerprint.clear();
    }
    if needs_human_confirmation {
        if let Some(autopilot) = project.workflow_state.autopilot_state.as_mut() {
            autopilot.run_status = project::AutopilotRunStatus::ErrorStopped;
            autopilot.recovery_action = project::AutopilotRecoveryAction::WaitHumanDecision;
            autopilot.error_message =
                "陈旧控制锁已释放，但原动作完成事实无法确认，请检查任务状态。".to_string();
        }
    }
    let post_task_state = task_state_label(project, &task_id);
    project
        .execution_history
        .push(project::ExecutionHistoryEntry {
            timestamp: cleared_at.clone(),
            level: if needs_human_confirmation {
                "pause"
            } else if completed {
                "success"
            } else {
                "info"
            }
            .to_string(),
            event_type: if needs_human_confirmation {
                project::ExecutionEventType::StaleControlActionNeedsHumanConfirmation
            } else {
                project::ExecutionEventType::StaleControlLockCleared
            },
            source: project::OperationSource::System,
            text: if completed {
                "陈旧控制动作锁已清理；磁盘完成事实已确认。".to_string()
            } else if needs_human_confirmation {
                "陈旧控制动作锁已清理；原动作需人工确认。".to_string()
            } else {
                "陈旧控制动作锁已清理；任务已恢复为可重新决策。".to_string()
            },
            milestone_id: (!project.current_milestone_id.is_empty())
                .then(|| project.current_milestone_id.clone()),
            mid_stage_id: (!project.current_mid_stage_id.is_empty())
                .then(|| project.current_mid_stage_id.clone()),
            subtask_id: (!task_id.is_empty()).then(|| task_id.clone()),
            criterion_index: None,
            decision_id: None,
            action_id: (!action_id.is_empty()).then(|| action_id.clone()),
            validator_id: None,
            model_call_id: None,
            control_lock_owner_process_start_id: owner_process_start_id,
            control_lock_heartbeat_at: heartbeat_at,
            control_lock_clear_reason: Some(reason.clone()),
            control_lock_post_task_state: Some(post_task_state.clone()),
        });
    if project.execution_history.len() > project::MAX_EXECUTION_HISTORY {
        let excess = project.execution_history.len() - project::MAX_EXECUTION_HISTORY;
        project.execution_history.drain(0..excess);
    }
    project.workflow_state.data_revision = project.workflow_state.data_revision.saturating_add(1);
    project.workflow_state.last_transition_at = cleared_at;

    ControlActionLockReconciliation::Cleared {
        action_id,
        completed,
        needs_human_confirmation,
        reason,
        post_task_state,
    }
}

fn touch_action_heartbeat(
    project_name: &str,
    action_id: &str,
    owner_process_start_id: &str,
) -> Result<bool, String> {
    crate::mutate_project_for_control(project_name, |project| {
        let Some(lease) = project.task_control.active_action_lease.as_mut() else {
            return Ok((false, false));
        };
        if lease.action_id != action_id || lease.owner_process_start_id != owner_process_start_id {
            return Ok((false, false));
        }
        lease.heartbeat_at = chrono::Utc::now().to_rfc3339();
        Ok((true, true))
    })
}

struct ActionHeartbeatGuard {
    stop: Option<std::sync::mpsc::Sender<()>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ActionHeartbeatGuard {
    fn start(project_name: String, action_id: String, owner_process_start_id: String) -> Self {
        let (stop, receiver) = std::sync::mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("metheus-control-heartbeat".to_string())
            .spawn(move || loop {
                match receiver.recv_timeout(std::time::Duration::from_secs(
                    CONTROL_ACTION_HEARTBEAT_INTERVAL_SECS,
                )) {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        match touch_action_heartbeat(
                            &project_name,
                            &action_id,
                            &owner_process_start_id,
                        ) {
                            Ok(true) => {}
                            Ok(false) => break,
                            Err(_) => {
                                eprintln!("控制动作心跳持久化失败（action_kind=runtime_control）")
                            }
                        }
                    }
                }
            })
            .ok();
        Self {
            stop: Some(stop),
            handle,
        }
    }
}

impl Drop for ActionHeartbeatGuard {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub fn ensure_serial_takeover_actions_available() -> Result<(), String> {
    use ControlActionKind::*;
    let required = [
        Split,
        Execute,
        LocalValidate,
        AutomatedValidate,
        TargetedValidate,
        Repair,
        Recompile,
        AcceptDeviation,
        GitConfirm,
        Wait,
        Human,
    ];
    if required.iter().all(|action| {
        matches!(
            action,
            Split
                | Execute
                | LocalValidate
                | AutomatedValidate
                | TargetedValidate
                | Repair
                | Recompile
                | AcceptDeviation
                | GitConfirm
                | Wait
                | Human
        )
    }) {
        Ok(())
    } else {
        Err("串行接管动作执行器覆盖不完整".to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ControlActionRequest {
    #[serde(default)]
    pub action_id: String,
    pub action: ControlActionKind,
    #[serde(default)]
    pub task_id: String,
    #[serde(default)]
    pub decision_id: String,
    #[serde(default)]
    pub expected_project_revision: Option<u64>,
    #[serde(default)]
    pub expected_tree_revision: Option<u64>,
    #[serde(default)]
    pub contract_fingerprint: String,
    #[serde(default)]
    pub criterion_indexes: Vec<u32>,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub source: project::OperationSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlActionExecutionResult {
    pub action_id: String,
    pub action: ControlActionKind,
    pub task_id: String,
    pub lifecycle: ControlActionLifecycle,
    pub idempotent: bool,
    pub queued: bool,
    pub made_progress: bool,
    pub before_fingerprint: String,
    pub after_fingerprint: String,
    pub project_revision: u64,
    pub tree_revision: u64,
    pub message: String,
}

pub async fn execute(
    pipeline_state: Arc<Mutex<Option<PipelineState>>>,
    project_name: String,
    mut request: ControlActionRequest,
) -> Result<ControlActionExecutionResult, String> {
    if request.action_id.trim().is_empty() {
        request.action_id = uuid::Uuid::new_v4().to_string();
    }
    enum ClaimOutcome {
        Immediate(ControlActionExecutionResult),
        Claimed {
            before_fingerprint: String,
            project_revision: u64,
            tree_revision: u64,
        },
    }
    let owner_process_start_id = crate::project_state_bus::process_start_id().to_string();
    let claim = crate::mutate_project_for_control(&project_name, |project| {
        validate_request(project, &request)?;
        if project.task_control.last_completed_action_id == request.action_id {
            return Ok((
                ClaimOutcome::Immediate(previous_result(project, &request)),
                false,
            ));
        }
        match classify_control_action_occupancy(
            &project.task_control,
            &owner_process_start_id,
            chrono::Utc::now(),
        ) {
            ControlActionOccupancy::Unoccupied => {}
            ControlActionOccupancy::ActiveLocal(lease)
            | ControlActionOccupancy::ActiveForeign(lease)
                if lease.action_id == request.action_id =>
            {
                return Ok((
                    ClaimOutcome::Immediate(ControlActionExecutionResult {
                        action_id: request.action_id.clone(),
                        action: request.action,
                        task_id: request.task_id.clone(),
                        lifecycle: ControlActionLifecycle::Running,
                        idempotent: true,
                        queued: true,
                        made_progress: false,
                        before_fingerprint: project
                            .task_control
                            .last_action_before_fingerprint
                            .clone(),
                        after_fingerprint: String::new(),
                        project_revision: project.workflow_state.data_revision,
                        tree_revision: project.task_control.tree_revision,
                        message: "同一控制动作正在执行".to_string(),
                    }),
                    false,
                ));
            }
            ControlActionOccupancy::ActiveLocal(lease) => {
                return Err(format!("已有控制动作正在执行：{}", lease.action_id));
            }
            ControlActionOccupancy::ActiveForeign(lease) => {
                return Err(format!(
                    "另一 Metheus 进程正在执行控制动作：{}，请等待其完成",
                    lease.action_id
                ));
            }
            ControlActionOccupancy::Stale { reason, .. } => {
                return Err(format!(
                    "检测到陈旧控制动作锁：{}；请先同步项目状态",
                    reason
                ));
            }
        }

        let before_fingerprint = control_fingerprint(project, &request.task_id)?;
        let now = chrono::Utc::now().to_rfc3339();
        install_action_lease(
            &mut project.task_control,
            &request,
            &owner_process_start_id,
            &now,
        );
        project.task_control.last_action_before_fingerprint = before_fingerprint.clone();
        project.task_control.last_action_at = Some(now);
        project.workflow_state.data_revision =
            project.workflow_state.data_revision.saturating_add(1);
        Ok((
            ClaimOutcome::Claimed {
                before_fingerprint,
                project_revision: project.workflow_state.data_revision,
                tree_revision: project.task_control.tree_revision,
            },
            true,
        ))
    })?;
    let (before_fingerprint, claimed_project_revision, claimed_tree_revision) = match claim {
        ClaimOutcome::Immediate(result) => return Ok(result),
        ClaimOutcome::Claimed {
            before_fingerprint,
            project_revision,
            tree_revision,
        } => (before_fingerprint, project_revision, tree_revision),
    };
    let heartbeat = ActionHeartbeatGuard::start(
        project_name.clone(),
        request.action_id.clone(),
        owner_process_start_id,
    );
    let dispatched = dispatch(
        pipeline_state,
        &project_name,
        &request,
        claimed_project_revision,
        claimed_tree_revision,
    )
    .await;
    drop(heartbeat);
    match dispatched {
        Ok(message) => finish_action(&project_name, &request, before_fingerprint, message, true),
        Err(error) => {
            let _ = finish_action(
                &project_name,
                &request,
                before_fingerprint,
                error.clone(),
                false,
            );
            Err(error)
        }
    }
}

fn validate_request(
    project: &project::Project,
    request: &ControlActionRequest,
) -> Result<(), String> {
    if let Some(expected) = request.expected_project_revision {
        if project.workflow_state.data_revision != expected {
            return Err(format!(
                "项目修订冲突：请求={}，磁盘={}",
                expected, project.workflow_state.data_revision
            ));
        }
    }
    if let Some(expected) = request.expected_tree_revision {
        if project.task_control.tree_revision != expected {
            return Err(format!(
                "任务树修订冲突：请求={}，磁盘={}",
                expected, project.task_control.tree_revision
            ));
        }
    }
    if request.action == ControlActionKind::Wait {
        return Ok(());
    }
    if request.action == ControlActionKind::Human
        && request.task_id.is_empty()
        && request.criterion_indexes.is_empty()
    {
        return Ok(());
    }
    if request.task_id.is_empty() {
        return Err("控制动作必须指定任务节点".to_string());
    }
    let address = crate::task_tree::locate_task(project, &request.task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", request.task_id))?;
    let task = crate::task_tree::find_task(project, &request.task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", request.task_id))?;
    if request.source == project::OperationSource::User {
        let current = crate::task_tree::select_current_leaf(project)?
            .is_some_and(|address| address.task_id == request.task_id);
        let recovery_bound = project
            .workflow_state
            .recovery_state
            .as_ref()
            .zip(project.execution_session.as_ref())
            .is_some_and(|(recovery, session)| {
                recovery.phase == project::RecoveryPhase::WaitingHuman
                    && recovery.subtask_id == request.task_id
                    && session.subtask_id == request.task_id
            });
        if !current && !recovery_bound {
            return Err("只能操作当前叶子或当前人工恢复会话绑定的任务".to_string());
        }
    }
    if !request.contract_fingerprint.is_empty() {
        let workload = crate::workload_policy::current_profile(project)?;
        let contract = crate::task_compiler::compile(
            task,
            address.ancestor_task_ids.last().map(String::as_str),
            address.depth,
            workload,
        )
        .contract;
        if contract.fingerprint != request.contract_fingerprint {
            return Err("任务合同指纹已变化，拒绝旧控制动作".to_string());
        }
    }
    match request.action {
        ControlActionKind::Split | ControlActionKind::Recompile => {
            if project.execution_session.as_ref().is_some_and(|session| {
                session.active
                    && (session.subtask_id == task.id
                        || session.task_path.iter().any(|id| id == &task.id))
            }) {
                return Err("执行中的任务及其祖先不能拆分或重编译".to_string());
            }
            if crate::task_tree::is_terminal(&task.status) {
                return Err("已完成任务不能拆分或重编译".to_string());
            }
        }
        ControlActionKind::Execute => {
            if !task.child_tasks.is_empty() || task.status != project::SubtaskStatus::Pending {
                return Err("执行动作只能作用于 Pending 叶子任务".to_string());
            }
            if !address.dependencies_satisfied {
                return Err("叶子任务依赖尚未满足".to_string());
            }
        }
        ControlActionKind::LocalValidate
        | ControlActionKind::AutomatedValidate
        | ControlActionKind::TargetedValidate => {
            if !task.child_tasks.is_empty() || crate::task_tree::is_terminal(&task.status) {
                return Err("验证动作只能作用于未完成叶子任务".to_string());
            }
        }
        ControlActionKind::Human if !request.criterion_indexes.is_empty() => {
            if !task.child_tasks.is_empty() || crate::task_tree::is_terminal(&task.status) {
                return Err("人工审查只能作用于未完成叶子任务".to_string());
            }
            validation_targets_for_mode(
                task,
                &request.criterion_indexes,
                crate::validator_contract::VerificationMode::HumanReview,
            )?;
        }
        ControlActionKind::Repair => {
            if !task
                .acceptance_ledger
                .iter()
                .any(|item| item.status == project::AcceptanceStatus::Unsatisfied)
            {
                return Err("修复动作需要明确未满足的验收证据".to_string());
            }
        }
        ControlActionKind::AcceptDeviation => {
            crate::human_action_policy::authorize(
                project,
                &request.task_id,
                crate::human_action_policy::HumanTerminalAction::AcceptDeviation,
                &request.criterion_indexes,
                &request.reason,
            )?;
        }
        ControlActionKind::GitConfirm => {
            if task.status != project::SubtaskStatus::AwaitingConfirmation {
                return Err("Git 确认只能作用于待确认叶子任务".to_string());
            }
        }
        ControlActionKind::Wait | ControlActionKind::Human => {}
    }
    Ok(())
}

async fn dispatch(
    pipeline_state: Arc<Mutex<Option<PipelineState>>>,
    project_name: &str,
    request: &ControlActionRequest,
    claimed_project_revision: u64,
    claimed_tree_revision: u64,
) -> Result<String, String> {
    let claimed_project = crate::load_project(project_name)?;
    validate_claimed_dispatch(
        &claimed_project,
        request,
        claimed_project_revision,
        claimed_tree_revision,
    )?;
    match request.action {
        ControlActionKind::Split => {
            let mut project = claimed_project;
            crate::commands::task_control::split_task(&mut project, &request.task_id)?;
            project.workflow_state.data_revision =
                project.workflow_state.data_revision.saturating_add(1);
            crate::save_project(&project)?;
            Ok("任务已按独立产物拆分".to_string())
        }
        ControlActionKind::Execute => {
            crate::pipeline::execute_task_with_source(
                pipeline_state,
                project_name.to_string(),
                Some(request.task_id.clone()),
                request.source,
            )
            .await?;
            Ok("叶子任务执行已派发".to_string())
        }
        ControlActionKind::LocalValidate => {
            run_local_validation(project_name, request)?;
            Ok("本地确定性验证已完成".to_string())
        }
        ControlActionKind::AutomatedValidate => {
            let status = run_automated_validation(project_name, request)?;
            Ok(match status {
                project::AutomatedTestStatus::Passed => "自动化测试验证已通过".to_string(),
                project::AutomatedTestStatus::Failed => "自动化测试验证发现失败".to_string(),
                project::AutomatedTestStatus::NotConfigured => {
                    "项目未配置自动化测试，验收项保持未证明".to_string()
                }
                project::AutomatedTestStatus::Unavailable => {
                    "自动化测试环境不可用，已进入人工边界".to_string()
                }
                project::AutomatedTestStatus::Unknown => {
                    "自动化测试状态未知，验收项保持未证明".to_string()
                }
            })
        }
        ControlActionKind::TargetedValidate => {
            run_targeted_validation(project_name, request).await?;
            Ok("定向验证已完成".to_string())
        }
        ControlActionKind::Repair => {
            let mut project = claimed_project;
            let automatic = crate::recovery::ensure_quality_recovery(
                &mut project,
                if request.reason.is_empty() {
                    "控制器发现明确未满足验收项"
                } else {
                    &request.reason
                },
            )?;
            crate::save_project(&project)?;
            if !automatic {
                return Err("当前修复需要人工处理".to_string());
            }
            crate::recovery::run_error_recovery_with_pipeline(
                pipeline_state,
                project_name.to_string(),
            )
            .await?;
            Ok("受限修复已执行".to_string())
        }
        ControlActionKind::Recompile => {
            let mut project = claimed_project;
            crate::commands::task_control::recompile_task(&mut project, &request.task_id)?;
            project.workflow_state.data_revision =
                project.workflow_state.data_revision.saturating_add(1);
            crate::save_project(&project)?;
            Ok("当前任务合同已重编译".to_string())
        }
        ControlActionKind::AcceptDeviation => {
            let mut project = claimed_project;
            let claimed_lease = project.task_control.active_action_lease.take();
            let claimed_action_id = std::mem::take(&mut project.task_control.active_action_id);
            let claimed_action_kind = std::mem::take(&mut project.task_control.active_action_kind);
            let claimed_action_task_id =
                std::mem::take(&mut project.task_control.active_action_task_id);
            let accepted = crate::commands::task_control::accept_deviation(
                &mut project,
                &request.task_id,
                &request.criterion_indexes,
                &request.reason,
            );
            project.task_control.active_action_lease = claimed_lease;
            project.task_control.active_action_id = claimed_action_id;
            project.task_control.active_action_kind = claimed_action_kind;
            project.task_control.active_action_task_id = claimed_action_task_id;
            accepted?;
            let mut stage_reconciled = false;
            if crate::task_tree::find_task(&project, &request.task_id)?
                .is_some_and(|task| crate::task_tree::is_terminal(&task.status))
            {
                crate::task_aggregation::aggregate_ancestors(&mut project, &request.task_id)?;
                let address = crate::task_tree::locate_task(&project, &request.task_id)?
                    .ok_or_else(|| format!("任务节点不存在：{}", request.task_id))?;
                stage_reconciled = crate::pipeline::reconcile_terminal_stage(
                    &mut project,
                    &address.milestone_id,
                    &address.mid_stage_id,
                )?
                .0;
            }
            if !stage_reconciled {
                project.workflow_state.data_revision =
                    project.workflow_state.data_revision.saturating_add(1);
            }
            crate::save_project_if_revision(
                &project,
                claimed_project_revision,
                claimed_tree_revision,
            )?;
            Ok("验收偏差已按任务和验收项记录".to_string())
        }
        ControlActionKind::GitConfirm => {
            crate::pipeline::confirm_subtask_result_with_source(
                &pipeline_state,
                project_name.to_string(),
                request.source,
            )
            .await?;
            Ok("当前叶子 Git 确认已完成".to_string())
        }
        ControlActionKind::Wait => Ok("控制器等待新的项目事实".to_string()),
        ControlActionKind::Human => {
            let mut project = claimed_project;
            let message = enter_human_boundary(&mut project, request)?;
            crate::save_project(&project)?;
            Ok(message)
        }
    }
}

fn enter_human_boundary(
    project: &mut project::Project,
    request: &ControlActionRequest,
) -> Result<String, String> {
    if !request.task_id.is_empty() {
        let task = crate::task_tree::find_task(project, &request.task_id)?
            .ok_or_else(|| format!("任务节点不存在：{}", request.task_id))?
            .clone();
        let human_targets = if request.criterion_indexes.is_empty() {
            crate::acceptance::revalidation_target_indexes(&task, &[])?
                .into_iter()
                .filter(|index| {
                    crate::validator_registry::verification_mode_for(&task, *index)
                        == crate::validator_contract::VerificationMode::HumanReview
                })
                .collect::<Vec<_>>()
        } else {
            request.criterion_indexes.clone()
        };
        if !human_targets.is_empty() {
            return crate::recovery::enter_human_review_boundary(
                project,
                &request.task_id,
                &human_targets,
                &request.reason,
            );
        }
    }
    let message = if request.criterion_indexes.is_empty() {
        "控制器已进入人工边界".to_string()
    } else {
        format!(
            "验收项 {} 已进入人工审查边界，等待显式人工结论",
            request
                .criterion_indexes
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    if let Some(state) = project.workflow_state.autopilot_state.as_mut() {
        state.run_status = project::AutopilotRunStatus::ErrorStopped;
        state.recovery_action = project::AutopilotRecoveryAction::WaitHumanDecision;
        state.error_message = if request.reason.trim().is_empty() {
            message.clone()
        } else {
            request.reason.clone()
        };
    }
    Ok(message)
}

fn validate_claimed_dispatch(
    project: &project::Project,
    request: &ControlActionRequest,
    claimed_project_revision: u64,
    claimed_tree_revision: u64,
) -> Result<(), String> {
    if project.workflow_state.data_revision != claimed_project_revision
        || project.task_control.tree_revision != claimed_tree_revision
    {
        return Err("控制动作认领后项目状态已变化，拒绝旧动作".to_string());
    }
    let lease_matches = project
        .task_control
        .active_action_lease
        .as_ref()
        .is_some_and(|lease| {
            lease.action_id == request.action_id
                && lease.action_kind == request.action.as_str()
                && lease.task_id == request.task_id
                && lease.owner_process_start_id == crate::project_state_bus::process_start_id()
        });
    if !lease_matches {
        return Err("控制动作认领已被新的项目状态取代".to_string());
    }
    let mut revalidated = request.clone();
    revalidated.expected_project_revision = Some(claimed_project_revision);
    revalidated.expected_tree_revision = Some(claimed_tree_revision);
    // The exact claimed lease was verified above. Re-run business authorization on a
    // read-only copy without that same lease so a human terminal action does not reject
    // itself as concurrent occupancy; foreign or replaced leases never reach this point.
    let mut business_view = project.clone();
    business_view.task_control.active_action_lease = None;
    business_view.task_control.active_action_id.clear();
    business_view.task_control.active_action_kind.clear();
    business_view.task_control.active_action_task_id.clear();
    validate_request(&business_view, &revalidated)
}

fn run_local_validation(project_name: &str, request: &ControlActionRequest) -> Result<(), String> {
    let mut project = crate::load_project(project_name)?;
    let task = crate::task_tree::find_task(&project, &request.task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", request.task_id))?
        .clone();
    let authorized = crate::plan_contract::validate_subtask(&task, "本地验证任务")?;
    let targets = validation_targets_for_mode(
        &task,
        &request.criterion_indexes,
        crate::validator_contract::VerificationMode::Deterministic,
    )?;
    let mut updates = Vec::new();
    for index in targets {
        let criterion = task
            .acceptance_criteria
            .get(index.saturating_sub(1) as usize)
            .ok_or_else(|| format!("验收项不存在：{}", index))?;
        let Some(batch) = crate::validator_registry::try_validate_locally(
            &project.project_path,
            std::slice::from_ref(criterion),
            &authorized,
        ) else {
            updates.push(project::AcceptanceLedgerItem {
                criterion_index: index,
                criterion: criterion.clone(),
                status: project::AcceptanceStatus::Unknown,
                evidence: "local_unprovable:本地验证器无法保守证明，转入定向语义审查".to_string(),
                confidence: 0.0,
                updated_at: chrono::Utc::now().to_rfc3339(),
                ..Default::default()
            });
            continue;
        };
        let Some(review) = batch.criterion_reviews.first() else {
            continue;
        };
        let evidence = batch
            .validator_runs
            .first()
            .map(|run| {
                format!(
                    "{}@{}:{}",
                    run.validator, run.version, run.evidence_fingerprint
                )
            })
            .unwrap_or_else(|| "本地确定性验证".to_string());
        updates.push(project::AcceptanceLedgerItem {
            criterion_index: index,
            criterion: criterion.clone(),
            status: match review.conclusion {
                project::CriterionReviewConclusion::Satisfied => {
                    project::AcceptanceStatus::Satisfied
                }
                project::CriterionReviewConclusion::Unsatisfied => {
                    project::AcceptanceStatus::Unsatisfied
                }
                project::CriterionReviewConclusion::EvidenceInsufficient => {
                    project::AcceptanceStatus::Unknown
                }
            },
            evidence,
            evidence_references: review.evidence_references.clone(),
            confidence: review.confidence,
            updated_at: chrono::Utc::now().to_rfc3339(),
        });
    }
    merge_ledger_updates(&mut project, &request.task_id, updates)?;
    crate::save_project(&project)
}

fn validation_targets_for_mode(
    task: &project::Subtask,
    requested: &[u32],
    mode: crate::validator_contract::VerificationMode,
) -> Result<Vec<u32>, String> {
    let candidates = crate::acceptance::revalidation_target_indexes(task, requested)?;
    let targets = candidates
        .iter()
        .copied()
        .filter(|index| {
            let configured = crate::validator_registry::verification_mode_for(task, *index);
            configured == mode
                || (mode == crate::validator_contract::VerificationMode::SemanticReview
                    && configured == crate::validator_contract::VerificationMode::Deterministic
                    && task.acceptance_ledger.iter().any(|item| {
                        item.criterion_index == *index
                            && item.status == project::AcceptanceStatus::Unknown
                            && item.evidence.starts_with("local_unprovable:")
                    }))
        })
        .collect::<Vec<_>>();
    if !requested.is_empty() && targets.len() != candidates.len() {
        return Err(format!("请求包含不属于 {:?} 通道的验收项", mode));
    }
    if targets.is_empty() {
        return Err(format!("当前任务没有需要 {:?} 验证的验收项", mode));
    }
    Ok(targets)
}

fn automated_ledger_updates(
    task: &project::Subtask,
    targets: &[u32],
    evidence: &crate::automated_validation::AutomatedTestEvidence,
) -> Result<Vec<project::AcceptanceLedgerItem>, String> {
    let mut updates = Vec::new();
    for index in targets {
        if crate::validator_registry::verification_mode_for(task, *index)
            != crate::validator_contract::VerificationMode::AutomatedTest
        {
            return Err(format!("验收项 {} 不是自动化测试验证模式", index));
        }
        let criterion = task
            .acceptance_criteria
            .get(index.saturating_sub(1) as usize)
            .ok_or_else(|| format!("验收项不存在：{}", index))?;
        let status = match evidence.status {
            project::AutomatedTestStatus::Passed => project::AcceptanceStatus::Satisfied,
            project::AutomatedTestStatus::Failed => project::AcceptanceStatus::Unsatisfied,
            project::AutomatedTestStatus::NotConfigured
            | project::AutomatedTestStatus::Unavailable
            | project::AutomatedTestStatus::Unknown => project::AcceptanceStatus::Unknown,
        };
        updates.push(project::AcceptanceLedgerItem {
            criterion_index: *index,
            criterion: criterion.clone(),
            status,
            evidence: format!(
                "automated_test_runner: command={} status={:?} exit_code={:?}; {}",
                evidence.command, evidence.status, evidence.exit_code, evidence.output_summary
            ),
            evidence_references: vec![],
            confidence: if matches!(
                evidence.status,
                project::AutomatedTestStatus::Passed | project::AutomatedTestStatus::Failed
            ) {
                1.0
            } else {
                0.0
            },
            updated_at: chrono::Utc::now().to_rfc3339(),
        });
    }
    Ok(updates)
}

fn run_automated_validation(
    project_name: &str,
    request: &ControlActionRequest,
) -> Result<project::AutomatedTestStatus, String> {
    let project = crate::load_project(project_name)?;
    let task = crate::task_tree::find_task(&project, &request.task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", request.task_id))?
        .clone();
    let targets = validation_targets_for_mode(
        &task,
        &request.criterion_indexes,
        crate::validator_contract::VerificationMode::AutomatedTest,
    )?;
    let evidence = crate::automated_validation::run_project_tests(&project.project_path);
    let updates = automated_ledger_updates(&task, &targets, &evidence)?;
    let mut project = crate::load_project(project_name)?;
    merge_ledger_updates(&mut project, &request.task_id, updates)?;
    let task = crate::task_tree::find_task_mut(&mut project, &request.task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", request.task_id))?;
    let mut result = task.test_result.clone().unwrap_or_default();
    result.test_command = evidence.command.clone();
    result.test_exit_code = evidence.exit_code;
    result.test_output_summary = evidence.output_summary.clone();
    result.automated_test_status = evidence.status.clone();
    result.verification_kind = project::VerificationKind::AutomatedTestOnly;
    result.acceptance_results = task.acceptance_ledger.clone();
    result.passed = evidence.status == project::AutomatedTestStatus::Passed;
    task.test_result = Some(result);
    if evidence.status == project::AutomatedTestStatus::Unavailable {
        let state = project
            .workflow_state
            .autopilot_state
            .get_or_insert_with(project::AutopilotState::default);
        state.run_status = project::AutopilotRunStatus::ErrorStopped;
        state.recovery_action = project::AutopilotRecoveryAction::WaitHumanDecision;
        state.error_message = "自动化测试环境不可用，不代表代码失败".to_string();
    }
    crate::save_project(&project)?;
    Ok(evidence.status)
}

async fn run_targeted_validation(
    project_name: &str,
    request: &ControlActionRequest,
) -> Result<(), String> {
    let project = crate::load_project(project_name)?;
    let task = crate::task_tree::find_task(&project, &request.task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", request.task_id))?
        .clone();
    let authorized = crate::plan_contract::validate_subtask(&task, "定向验证任务")?;
    let targets = validation_targets_for_mode(
        &task,
        &request.criterion_indexes,
        crate::validator_contract::VerificationMode::SemanticReview,
    )?;
    let previous_test = task.test_result.clone().unwrap_or_default();
    let result = crate::test_runner::review_subtask_with_context_and_model(
        &project.project_path,
        if task.goal.is_empty() {
            &task.title
        } else {
            &task.goal
        },
        &task.id,
        &project.current_milestone_id,
        &project.current_mid_stage_id,
        Some(task.acceptance_criteria.clone()),
        Some(authorized.clone()),
        Some(crate::plan_compiler::compile_execution_prompt(&task)),
        Some(crate::review_evidence::ReviewEvidenceRequest::for_task(
            &task,
            project::ReviewEvidenceStrategy::Targeted,
            targets.clone(),
        )),
        &previous_test,
        Some(crate::cost_ledger::ModelCallContext {
            project_name: project.name.clone(),
            milestone_id: project.current_milestone_id.clone(),
            stage_id: crate::plan_scope::PlanScope::resolve(&project)
                .map(|scope| scope.target_id(&project).to_string())
                .unwrap_or_default(),
            task_id: task.id.clone(),
            purpose: Some(crate::cost_ledger::ModelCallPurpose::EvidenceSupplement),
            decision_id: request.decision_id.clone(),
            action_id: request.action_id.clone(),
        }),
    )
    .await?;
    let ledger = crate::acceptance::build_ledger(&task.acceptance_criteria, &result, &authorized)
        .into_iter()
        .filter(|item| targets.contains(&item.criterion_index))
        .collect();
    let mut project = crate::load_project(project_name)?;
    merge_ledger_updates(&mut project, &request.task_id, ledger)?;
    crate::save_project(&project)
}

fn merge_ledger_updates(
    project: &mut project::Project,
    task_id: &str,
    updates: Vec<project::AcceptanceLedgerItem>,
) -> Result<(), String> {
    let task = crate::task_tree::find_task_mut(project, task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", task_id))?;
    if task.acceptance_ledger.is_empty() {
        task.acceptance_ledger = task
            .acceptance_criteria
            .iter()
            .enumerate()
            .map(|(index, criterion)| project::AcceptanceLedgerItem {
                criterion_index: index as u32 + 1,
                criterion: criterion.clone(),
                ..Default::default()
            })
            .collect();
    }
    for update in updates {
        if let Some(current) = task
            .acceptance_ledger
            .iter_mut()
            .find(|item| item.criterion_index == update.criterion_index)
        {
            *current = update;
        }
    }
    Ok(())
}

fn finish_action(
    project_name: &str,
    request: &ControlActionRequest,
    before_fingerprint: String,
    message: String,
    succeeded: bool,
) -> Result<ControlActionExecutionResult, String> {
    let owner_process_start_id = crate::project_state_bus::process_start_id().to_string();
    crate::mutate_project_for_control(project_name, |project| {
        let lease_matches = project
            .task_control
            .active_action_lease
            .as_ref()
            .is_some_and(|lease| {
                lease.action_id == request.action_id
                    && lease.owner_process_start_id == owner_process_start_id
            });
        if !lease_matches {
            return Err("控制动作已被新的项目状态取代".to_string());
        }
        let after_fingerprint = control_fingerprint(project, &request.task_id)?;
        let made_progress = succeeded && after_fingerprint != before_fingerprint;
        let now = chrono::Utc::now().to_rfc3339();
        clear_action_lease(
            &mut project.task_control,
            if succeeded {
                "normal_completion"
            } else {
                "action_failed"
            },
            &now,
        );
        project.task_control.last_completed_action_id = request.action_id.clone();
        project.task_control.last_completed_action_kind = request.action.as_str().to_string();
        project.task_control.last_completed_action_task_id = request.task_id.clone();
        project.task_control.last_action_result = message.clone();
        project.task_control.last_action_made_progress = made_progress;
        project.task_control.last_action_before_fingerprint = before_fingerprint.clone();
        project.task_control.last_action_after_fingerprint = after_fingerprint.clone();
        project.task_control.last_action_at = Some(now);
        if succeeded
            && !matches!(
                request.action,
                ControlActionKind::Wait | ControlActionKind::Human
            )
        {
            if let Some(state) = project.workflow_state.autopilot_state.as_mut() {
                state.consecutive_no_progress = if made_progress {
                    0
                } else {
                    state.consecutive_no_progress.saturating_add(1)
                };
            }
        }
        append_control_event(project, request, &message, succeeded);
        project.workflow_state.data_revision =
            project.workflow_state.data_revision.saturating_add(1);
        Ok((
            ControlActionExecutionResult {
                action_id: request.action_id.clone(),
                action: request.action,
                task_id: request.task_id.clone(),
                lifecycle: if succeeded {
                    ControlActionLifecycle::Completed
                } else {
                    ControlActionLifecycle::Failed
                },
                idempotent: false,
                queued: false,
                made_progress,
                before_fingerprint: before_fingerprint.clone(),
                after_fingerprint,
                project_revision: project.workflow_state.data_revision,
                tree_revision: project.task_control.tree_revision,
                message: message.clone(),
            },
            true,
        ))
    })
}

fn append_control_event(
    project: &mut project::Project,
    request: &ControlActionRequest,
    message: &str,
    succeeded: bool,
) {
    let model_call_id = project
        .cost_ledger
        .calls
        .iter()
        .rev()
        .find(|call| call.action_id == request.action_id)
        .map(|call| call.call_id.clone());
    let validator_id = match request.action {
        ControlActionKind::LocalValidate => Some("local_validator_registry".to_string()),
        ControlActionKind::AutomatedValidate => Some("automated_test_runner".to_string()),
        ControlActionKind::TargetedValidate => Some("semantic_review".to_string()),
        ControlActionKind::Human if !request.criterion_indexes.is_empty() => {
            Some("human_boundary_review".to_string())
        }
        _ => None,
    };
    project
        .execution_history
        .push(project::ExecutionHistoryEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            level: if succeeded { "success" } else { "error" }.to_string(),
            event_type: project::ExecutionEventType::SystemAdvance,
            source: request.source,
            text: message.to_string(),
            milestone_id: (!project.current_milestone_id.is_empty())
                .then(|| project.current_milestone_id.clone()),
            mid_stage_id: (!project.current_mid_stage_id.is_empty())
                .then(|| project.current_mid_stage_id.clone()),
            subtask_id: (!request.task_id.is_empty()).then(|| request.task_id.clone()),
            criterion_index: (request.criterion_indexes.len() == 1)
                .then(|| request.criterion_indexes[0]),
            decision_id: (!request.decision_id.is_empty()).then(|| request.decision_id.clone()),
            action_id: Some(request.action_id.clone()),
            validator_id,
            model_call_id,
            control_lock_owner_process_start_id: None,
            control_lock_heartbeat_at: None,
            control_lock_clear_reason: None,
            control_lock_post_task_state: None,
        });
    if project.execution_history.len() > project::MAX_EXECUTION_HISTORY {
        let excess = project.execution_history.len() - project::MAX_EXECUTION_HISTORY;
        project.execution_history.drain(0..excess);
    }
}

fn previous_result(
    project: &project::Project,
    request: &ControlActionRequest,
) -> ControlActionExecutionResult {
    ControlActionExecutionResult {
        action_id: request.action_id.clone(),
        action: request.action,
        task_id: request.task_id.clone(),
        lifecycle: ControlActionLifecycle::Completed,
        idempotent: true,
        queued: false,
        made_progress: project.task_control.last_action_made_progress,
        before_fingerprint: project.task_control.last_action_before_fingerprint.clone(),
        after_fingerprint: project.task_control.last_action_after_fingerprint.clone(),
        project_revision: project.workflow_state.data_revision,
        tree_revision: project.task_control.tree_revision,
        message: project.task_control.last_action_result.clone(),
    }
}

fn control_fingerprint(project: &project::Project, task_id: &str) -> Result<String, String> {
    let task = if task_id.is_empty() {
        None
    } else {
        crate::task_tree::find_task(project, task_id)?
    };
    let bytes = serde_json::to_vec(&(
        project.task_control.tree_revision,
        project.workflow_state.current_step.clone(),
        task.map(|task| {
            (
                task.status.clone(),
                task.contract_snapshot.clone(),
                task.acceptance_ledger.clone(),
                task.fact_snapshot.clone(),
                task.child_tasks.len(),
            )
        }),
    ))
    .map_err(|error| format!("控制状态指纹生成失败：{}", error))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Milestone, MilestoneStatus, StageMode, Subtask};
    use std::path::PathBuf;

    struct ControlProjectGuard {
        data_path: PathBuf,
        lock_path: PathBuf,
    }

    impl ControlProjectGuard {
        fn new(project_name: &str) -> Result<Self, String> {
            let data_path = crate::project_data_path(project_name)?;
            let lock_path = data_path.with_extension("control-action.lock");
            Ok(Self {
                data_path,
                lock_path,
            })
        }
    }

    impl Drop for ControlProjectGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.data_path);
            let _ = std::fs::remove_file(&self.lock_path);
        }
    }

    fn project_with_task(status: project::SubtaskStatus) -> project::Project {
        let mut project = project::Project::new("executor");
        project.workload_profile = Some(crate::workload_policy::test_profile(
            project::WorkloadScale::Small,
        ));
        project.milestones.push(Milestone {
            id: "m".to_string(),
            version: "v0.1".to_string(),
            title: "M".to_string(),
            description: String::new(),
            tech_stack: String::new(),
            status: MilestoneStatus::InProgress,
            mode: StageMode::Quick,
            mid_stages: Vec::new(),
            subtasks: vec![Subtask {
                id: "task".to_string(),
                status,
                acceptance_criteria: vec!["criterion".to_string()],
                ..Default::default()
            }],
            qa_result: None,
            git_commit_hash: String::new(),
            decomposition_check: None,
            review_status: None,
            review_conclusion: None,
            approved_at: None,
            goal: String::new(),
            scope: String::new(),
            dependencies: Vec::new(),
            expected_output: String::new(),
            acceptance_criteria: Vec::new(),
            ..Default::default()
        });
        project.current_milestone_id = "m".to_string();
        project
    }

    fn prepare_deviation_task(task: &mut Subtask) {
        task.status = project::SubtaskStatus::AwaitingConfirmation;
        task.execution_result = Some(project::ExecutionResult {
            success: true,
            output: "execution completed".to_string(),
            ..Default::default()
        });
        task.acceptance_ledger = vec![project::AcceptanceLedgerItem {
            criterion_index: 1,
            criterion: "criterion".to_string(),
            status: project::AcceptanceStatus::Unknown,
            ..Default::default()
        }];
    }

    fn mid_stage(
        id: &str,
        status: project::MidStageStatus,
        subtasks: Vec<Subtask>,
    ) -> project::MidStage {
        project::MidStage {
            id: id.to_string(),
            title: id.to_string(),
            version: "v0.1.1".to_string(),
            order: None,
            status,
            subtasks,
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

    async fn execute_deviation(project_name: &str) -> Result<ControlActionExecutionResult, String> {
        let persisted = crate::load_project(project_name)?;
        execute(
            Arc::new(Mutex::new(None)),
            project_name.to_string(),
            ControlActionRequest {
                action_id: format!("accept-deviation-{}", uuid::Uuid::new_v4()),
                action: ControlActionKind::AcceptDeviation,
                task_id: "task".to_string(),
                expected_project_revision: Some(persisted.workflow_state.data_revision),
                expected_tree_revision: Some(persisted.task_control.tree_revision),
                criterion_indexes: vec![1],
                reason: "已核实成功执行，接受当前范围化偏差".to_string(),
                source: project::OperationSource::User,
                ..Default::default()
            },
        )
        .await
    }

    #[tokio::test]
    async fn adaptive_execution_contract_accept_deviation_quick_reconciles_once_to_review(
    ) -> Result<(), String> {
        let project_name = format!("accept-quick-{}", uuid::Uuid::new_v4());
        let _guard = ControlProjectGuard::new(&project_name)?;
        let mut project = project_with_task(project::SubtaskStatus::AwaitingConfirmation);
        project.name = project_name.clone();
        project.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        project.workflow_state.current_step = project::WorkflowStep::Execution;
        project.workflow_state.autopilot_active = true;
        project.workflow_state.autopilot_state = Some(project::AutopilotState::default());
        prepare_deviation_task(&mut project.milestones[0].subtasks[0]);
        crate::save_project(&project)?;
        let before = crate::load_project(&project_name)?;

        let result = execute_deviation(&project_name).await?;
        let completed = crate::load_project(&project_name)?;
        assert_eq!(
            completed.milestones[0].subtasks[0].status,
            project::SubtaskStatus::AcceptedDeviation
        );
        assert_eq!(
            completed.milestones[0].status,
            project::MilestoneStatus::Completed
        );
        assert_eq!(
            completed.milestones[0].review_status.as_deref(),
            Some("pending_review")
        );
        assert_eq!(
            completed.workflow_state.current_step,
            project::WorkflowStep::MilestoneReview
        );
        assert_eq!(completed.workflow_state.review_node_id, "m");
        assert_eq!(
            completed
                .workflow_state
                .autopilot_state
                .as_ref()
                .unwrap()
                .run_status,
            project::AutopilotRunStatus::WaitingMilestoneReview
        );
        assert_eq!(
            completed.workflow_state.data_revision,
            before.workflow_state.data_revision + 3
        );
        assert_eq!(
            completed.task_control.tree_revision,
            before.task_control.tree_revision
        );
        assert!(completed.task_control.active_action_lease.is_none());
        assert_eq!(
            result.project_revision,
            completed.workflow_state.data_revision
        );
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_accept_deviation_professional_advances_to_next_stage(
    ) -> Result<(), String> {
        let project_name = format!("accept-professional-{}", uuid::Uuid::new_v4());
        let _guard = ControlProjectGuard::new(&project_name)?;
        let mut project = project_with_task(project::SubtaskStatus::AwaitingConfirmation);
        project.name = project_name.clone();
        project.workload_profile = Some(crate::workload_policy::test_profile(
            project::WorkloadScale::System,
        ));
        project.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        project.workflow_state.current_step = project::WorkflowStep::Execution;
        project.current_mid_stage_id = "mid-1".to_string();
        let mut current = mid_stage(
            "mid-1",
            project::MidStageStatus::InProgress,
            std::mem::take(&mut project.milestones[0].subtasks),
        );
        prepare_deviation_task(&mut current.subtasks[0]);
        let next = mid_stage(
            "mid-2",
            project::MidStageStatus::Ready,
            vec![Subtask {
                id: "next-task".to_string(),
                acceptance_criteria: vec!["next criterion".to_string()],
                ..Default::default()
            }],
        );
        project.milestones[0].mode = project::StageMode::Professional;
        project.milestones[0].mid_stages = vec![current, next];
        crate::save_project(&project)?;
        let before = crate::load_project(&project_name)?;

        execute_deviation(&project_name).await?;
        let completed = crate::load_project(&project_name)?;
        assert_eq!(
            completed.milestones[0].mid_stages[0].status,
            project::MidStageStatus::Completed
        );
        assert_eq!(
            completed.workflow_state.current_step,
            project::WorkflowStep::MidStageSelection
        );
        assert!(completed.current_mid_stage_id.is_empty());
        assert_eq!(
            completed.milestones[0].status,
            project::MilestoneStatus::InProgress
        );
        assert_eq!(
            completed.workflow_state.data_revision,
            before.workflow_state.data_revision + 3
        );
        assert_eq!(
            completed.task_control.tree_revision,
            before.task_control.tree_revision
        );
        Ok(())
    }

    #[tokio::test]
    async fn adaptive_execution_contract_accept_deviation_professional_last_stage_reaches_review(
    ) -> Result<(), String> {
        let project_name = format!("accept-professional-last-{}", uuid::Uuid::new_v4());
        let _guard = ControlProjectGuard::new(&project_name)?;
        let mut project = project_with_task(project::SubtaskStatus::AwaitingConfirmation);
        project.name = project_name.clone();
        project.workload_profile = Some(crate::workload_policy::test_profile(
            project::WorkloadScale::System,
        ));
        project.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        project.workflow_state.current_step = project::WorkflowStep::Execution;
        project.current_mid_stage_id = "mid-1".to_string();
        let mut current = mid_stage(
            "mid-1",
            project::MidStageStatus::InProgress,
            std::mem::take(&mut project.milestones[0].subtasks),
        );
        prepare_deviation_task(&mut current.subtasks[0]);
        project.milestones[0].mode = project::StageMode::Professional;
        project.milestones[0].mid_stages = vec![current];
        crate::save_project(&project)?;
        let before = crate::load_project(&project_name)?;

        execute_deviation(&project_name).await?;
        let completed = crate::load_project(&project_name)?;
        assert_eq!(
            completed.milestones[0].mid_stages[0].status,
            project::MidStageStatus::Completed
        );
        assert_eq!(
            completed.milestones[0].status,
            project::MilestoneStatus::Completed
        );
        assert_eq!(
            completed.workflow_state.current_step,
            project::WorkflowStep::MilestoneReview
        );
        assert_eq!(completed.workflow_state.review_node_id, "m");
        assert_eq!(
            completed.workflow_state.data_revision,
            before.workflow_state.data_revision + 3
        );
        assert_eq!(
            completed.task_control.tree_revision,
            before.task_control.tree_revision
        );
        Ok(())
    }

    #[test]
    fn execute_rejects_parent_and_non_pending_task() {
        let mut project = project_with_task(project::SubtaskStatus::Pending);
        project.milestones[0].subtasks[0].child_tasks = vec![Subtask {
            id: "child".to_string(),
            ..Default::default()
        }];
        let request = ControlActionRequest {
            action: ControlActionKind::Execute,
            task_id: "task".to_string(),
            ..Default::default()
        };
        assert!(validate_request(&project, &request)
            .unwrap_err()
            .contains("叶子"));
    }

    #[test]
    fn git_confirm_requires_awaiting_leaf() {
        let project = project_with_task(project::SubtaskStatus::Pending);
        let request = ControlActionRequest {
            action: ControlActionKind::GitConfirm,
            task_id: "task".to_string(),
            ..Default::default()
        };
        assert!(validate_request(&project, &request)
            .unwrap_err()
            .contains("待确认"));
    }

    #[test]
    fn phase1_human_action_safety_accepting_deviation_requires_success_and_scope() {
        let mut project = project_with_task(project::SubtaskStatus::AwaitingConfirmation);
        project.milestones[0].subtasks[0].execution_result = Some(project::ExecutionResult {
            success: true,
            ..Default::default()
        });
        project.milestones[0].subtasks[0].acceptance_ledger = vec![project::AcceptanceLedgerItem {
            criterion_index: 1,
            criterion: "criterion".to_string(),
            ..Default::default()
        }];
        let missing_scope = ControlActionRequest {
            action: ControlActionKind::AcceptDeviation,
            task_id: "task".to_string(),
            reason: "已由用户确认".to_string(),
            ..Default::default()
        };
        assert!(validate_request(&project, &missing_scope)
            .unwrap_err()
            .contains("选择至少一个"));

        let missing_reason = ControlActionRequest {
            action: ControlActionKind::AcceptDeviation,
            task_id: "task".to_string(),
            criterion_indexes: vec![1],
            ..Default::default()
        };
        assert!(validate_request(&project, &missing_reason)
            .unwrap_err()
            .contains("填写依据"));

        project.milestones[0].subtasks[0].execution_result = None;
        let unexecuted = ControlActionRequest {
            action: ControlActionKind::AcceptDeviation,
            task_id: "task".to_string(),
            criterion_indexes: vec![1],
            reason: "已由用户确认".to_string(),
            ..Default::default()
        };
        assert!(validate_request(&project, &unexecuted)
            .unwrap_err()
            .contains("没有成功完成"));
    }

    #[test]
    fn phase1_human_action_safety_claimed_dispatch_rejects_newer_revision() {
        let mut project = project_with_task(project::SubtaskStatus::AwaitingConfirmation);
        project.milestones[0].subtasks[0].execution_result = Some(project::ExecutionResult {
            success: true,
            ..Default::default()
        });
        project.milestones[0].subtasks[0].acceptance_ledger = vec![project::AcceptanceLedgerItem {
            criterion_index: 1,
            criterion: "criterion".to_string(),
            ..Default::default()
        }];
        let now = chrono::Utc::now().to_rfc3339();
        install_action_lease(
            &mut project.task_control,
            &ControlActionRequest {
                action_id: "action".to_string(),
                action: ControlActionKind::AcceptDeviation,
                task_id: "task".to_string(),
                ..Default::default()
            },
            crate::project_state_bus::process_start_id(),
            &now,
        );
        project.workflow_state.data_revision = 8;
        project.task_control.tree_revision = 3;
        let request = ControlActionRequest {
            action_id: "action".to_string(),
            action: ControlActionKind::AcceptDeviation,
            task_id: "task".to_string(),
            criterion_indexes: vec![1],
            reason: "known deviation".to_string(),
            ..Default::default()
        };
        validate_claimed_dispatch(&project, &request, 8, 3).unwrap();
        project.workflow_state.data_revision = 9;
        assert!(validate_claimed_dispatch(&project, &request, 8, 3).is_err());
    }

    fn test_lease(owner: &str, started_at: &str, heartbeat_at: &str) -> ControlActionLease {
        ControlActionLease {
            action_id: "action-1".to_string(),
            owner_process_start_id: owner.to_string(),
            action_kind: "execute".to_string(),
            task_id: "task-1".to_string(),
            started_at: started_at.to_string(),
            heartbeat_at: heartbeat_at.to_string(),
            expected_max_duration_secs: CONTROL_ACTION_EXECUTION_EXPECTED_SECS,
        }
    }

    fn state_with_lease(lease: ControlActionLease) -> TaskControlState {
        TaskControlState {
            active_action_id: lease.action_id.clone(),
            active_action_kind: lease.action_kind.clone(),
            active_action_task_id: lease.task_id.clone(),
            active_action_lease: Some(lease),
            ..TaskControlState::default()
        }
    }

    #[test]
    fn runtime_fault_lock_lease_fresh_local_and_foreign_owners_remain_active() {
        let now = chrono::Utc::now();
        let timestamp = now.to_rfc3339();
        let local = state_with_lease(test_lease("process-a", &timestamp, &timestamp));
        assert!(matches!(
            classify_control_action_occupancy(&local, "process-a", now),
            ControlActionOccupancy::ActiveLocal(_)
        ));
        assert!(matches!(
            classify_control_action_occupancy(&local, "process-b", now),
            ControlActionOccupancy::ActiveForeign(_)
        ));
    }

    #[test]
    fn runtime_fault_lock_lease_expired_heartbeat_is_stale() {
        let now = chrono::Utc::now();
        let started = (now - chrono::Duration::seconds(30)).to_rfc3339();
        let heartbeat = (now - chrono::Duration::seconds(16)).to_rfc3339();
        let state = state_with_lease(test_lease("process-a", &started, &heartbeat));
        assert!(matches!(
            classify_control_action_occupancy(&state, "process-a", now),
            ControlActionOccupancy::Stale { reason, .. } if reason.contains("心跳")
        ));
    }

    #[test]
    fn runtime_fault_lock_lease_expected_duration_is_bounded_by_hard_limit() {
        let now = chrono::Utc::now();
        let started = (now
            - chrono::Duration::seconds(CONTROL_ACTION_MAX_EXECUTION_SECS as i64 + 1))
        .to_rfc3339();
        let heartbeat = now.to_rfc3339();
        let mut lease = test_lease("process-a", &started, &heartbeat);
        lease.expected_max_duration_secs = CONTROL_ACTION_MAX_EXECUTION_SECS * 2;
        let state = state_with_lease(lease);
        assert!(matches!(
            classify_control_action_occupancy(&state, "process-a", now),
            ControlActionOccupancy::Stale { reason, .. } if reason.contains("最长执行时长")
        ));
    }

    #[test]
    fn provability_closeout_lock_windows_are_short_and_keep_normal_actions_safe() {
        assert!(CONTROL_ACTION_EXECUTION_EXPECTED_SECS < 27 * 60);
        assert!(CONTROL_ACTION_EXECUTION_EXPECTED_SECS > 105);
        assert!(CONTROL_ACTION_VALIDATION_EXPECTED_SECS < CONTROL_ACTION_EXECUTION_EXPECTED_SECS);
        assert_eq!(
            expected_action_duration_secs(ControlActionKind::Execute),
            10 * 60
        );
        assert_eq!(
            expected_action_duration_secs(ControlActionKind::TargetedValidate),
            5 * 60
        );
    }

    #[test]
    fn provability_closeout_fresh_overdue_foreign_lease_is_not_force_cleared() {
        let now = chrono::Utc::now();
        let started = (now - chrono::Duration::seconds(11 * 60)).to_rfc3339();
        let heartbeat = now.to_rfc3339();
        let mut project = project_with_task(project::SubtaskStatus::Pending);
        let mut lease = test_lease("live-other-process", &started, &heartbeat);
        lease.task_id = "task".to_string();
        project.task_control = state_with_lease(lease);

        assert!(matches!(
            classify_control_action_occupancy(&project.task_control, "this-process", now),
            ControlActionOccupancy::Stale { reason, .. } if reason.contains("最长执行时长")
        ));
        assert_eq!(
            reconcile_stale_control_action_lock(&mut project, "this-process", now),
            ControlActionLockReconciliation::Unchanged
        );
        assert!(project.task_control.active_action_lease.is_some());
    }

    #[test]
    fn runtime_fault_lock_lease_legacy_string_lock_requires_reconciliation() {
        let state = TaskControlState {
            active_action_id: "legacy-action".to_string(),
            ..TaskControlState::default()
        };
        assert!(matches!(
            classify_control_action_occupancy(&state, "process-a", chrono::Utc::now()),
            ControlActionOccupancy::Stale { lease: None, reason } if reason.contains("旧格式")
        ));
    }

    #[test]
    fn runtime_fault_stale_lock_reconciliation_clears_and_audits_expired_lease() {
        let now = chrono::Utc::now();
        let started = (now - chrono::Duration::seconds(40)).to_rfc3339();
        let heartbeat = (now - chrono::Duration::seconds(20)).to_rfc3339();
        let mut project = project_with_task(project::SubtaskStatus::Pending);
        let mut lease = test_lease("old-process", &started, &heartbeat);
        lease.task_id = "task".to_string();
        project.task_control = state_with_lease(lease);

        let result = reconcile_stale_control_action_lock(&mut project, "new-process", now);
        assert!(matches!(
            result,
            ControlActionLockReconciliation::Cleared {
                completed: false,
                needs_human_confirmation: false,
                ..
            }
        ));
        assert!(project.task_control.active_action_lease.is_none());
        assert!(project.task_control.active_action_id.is_empty());
        assert!(project
            .task_control
            .last_action_clear_reason
            .contains("心跳"));
        let event = project.execution_history.last().unwrap();
        assert_eq!(
            event.event_type,
            project::ExecutionEventType::StaleControlLockCleared
        );
        assert_eq!(
            event.control_lock_owner_process_start_id.as_deref(),
            Some("old-process")
        );
        assert_eq!(
            event.control_lock_heartbeat_at.as_deref(),
            Some(heartbeat.as_str())
        );
        assert_eq!(
            event.control_lock_post_task_state.as_deref(),
            Some("Pending")
        );
    }

    #[test]
    fn runtime_fault_stale_lock_reconciliation_preserves_fresh_foreign_owner() {
        let now = chrono::Utc::now();
        let timestamp = now.to_rfc3339();
        let mut project = project_with_task(project::SubtaskStatus::Pending);
        let mut lease = test_lease("live-other-process", &timestamp, &timestamp);
        lease.task_id = "task".to_string();
        project.task_control = state_with_lease(lease);

        assert_eq!(
            reconcile_stale_control_action_lock(&mut project, "this-process", now),
            ControlActionLockReconciliation::Unchanged
        );
        assert!(project.task_control.active_action_lease.is_some());
        assert!(project.execution_history.is_empty());
    }

    #[test]
    fn runtime_fault_stale_lock_reconciliation_legacy_lock_requires_human_audit() {
        let mut project = project_with_task(project::SubtaskStatus::Pending);
        project.task_control.active_action_id = "legacy-action".to_string();

        let result =
            reconcile_stale_control_action_lock(&mut project, "this-process", chrono::Utc::now());
        assert!(matches!(
            result,
            ControlActionLockReconciliation::Cleared {
                needs_human_confirmation: true,
                ..
            }
        ));
        assert!(project.task_control.active_action_id.is_empty());
        assert_eq!(
            project.execution_history.last().unwrap().event_type,
            project::ExecutionEventType::StaleControlActionNeedsHumanConfirmation
        );
    }

    #[test]
    fn runtime_fault_stale_lock_reconciliation_publishes_project_change() -> Result<(), String> {
        let now = chrono::Utc::now();
        let started = (now - chrono::Duration::seconds(40)).to_rfc3339();
        let heartbeat = (now - chrono::Duration::seconds(20)).to_rfc3339();
        let project_name = format!("stale-lock-notify-{}", uuid::Uuid::new_v4());
        let mut project = project_with_task(project::SubtaskStatus::Pending);
        project.name = project_name.clone();
        let mut lease = test_lease("old-process", &started, &heartbeat);
        lease.task_id = "task".to_string();
        project.task_control = state_with_lease(lease);
        crate::save_project(&project)?;
        let before = crate::project_state_bus::project_state_cursor(&project_name)?;

        crate::mutate_project_for_control(&project_name, |persisted| {
            let result =
                reconcile_stale_control_action_lock(persisted, "new-process", chrono::Utc::now());
            Ok(((), result.changed()))
        })?;
        let after = crate::project_state_bus::project_state_cursor(&project_name)?;
        assert!(after.event_sequence > before.event_sequence);
        assert!(crate::load_project(&project_name)?
            .task_control
            .active_action_id
            .is_empty());

        let data_path = crate::project_data_path(&project_name)?;
        let lock_path = data_path.with_extension("control-action.lock");
        let _ = std::fs::remove_file(data_path);
        let _ = std::fs::remove_file(lock_path);
        Ok(())
    }

    #[tokio::test]
    async fn runtime_fault_regression_fresh_occupancy_rejects_new_action() -> Result<(), String> {
        let project_name = format!("active-lock-reject-{}", uuid::Uuid::new_v4());
        let mut project = project_with_task(project::SubtaskStatus::Pending);
        project.name = project_name.clone();
        let now = chrono::Utc::now().to_rfc3339();
        let mut lease = test_lease("live-other-process", &now, &now);
        lease.task_id = "task".to_string();
        project.task_control = state_with_lease(lease);
        crate::save_project(&project)?;

        let error = execute(
            Arc::new(Mutex::new(None)),
            project_name.clone(),
            ControlActionRequest {
                action_id: "new-action".to_string(),
                action: ControlActionKind::Execute,
                task_id: "task".to_string(),
                ..Default::default()
            },
        )
        .await
        .expect_err("fresh occupancy must reject a different action");
        assert!(error.contains("另一 Metheus 进程正在执行控制动作"));

        let data_path = crate::project_data_path(&project_name)?;
        let lock_path = data_path.with_extension("control-action.lock");
        let _ = std::fs::remove_file(data_path);
        let _ = std::fs::remove_file(lock_path);
        Ok(())
    }

    #[test]
    fn automated_status_updates_only_automated_criteria() {
        let task = Subtask {
            acceptance_criteria: vec!["cargo test 测试通过".to_string()],
            ..Default::default()
        };
        let evidence = crate::automated_validation::AutomatedTestEvidence {
            rendered: None,
            command: "cargo test".to_string(),
            exit_code: Some(0),
            output_summary: "3 passed".to_string(),
            status: project::AutomatedTestStatus::Passed,
        };
        let updates = automated_ledger_updates(&task, &[1], &evidence).unwrap();
        assert_eq!(updates[0].status, project::AcceptanceStatus::Satisfied);
        assert_eq!(updates[0].confidence, 1.0);

        let semantic = Subtask {
            acceptance_criteria: vec!["页面显示测试按钮".to_string()],
            ..Default::default()
        };
        assert!(automated_ledger_updates(&semantic, &[1], &evidence).is_err());
    }

    #[test]
    fn empty_request_filters_targets_to_the_selected_validation_channel() {
        let task = Subtask {
            acceptance_criteria: vec![
                "file exists: `index.html`".to_string(),
                "cargo test 测试通过".to_string(),
                "用户可以完成结账".to_string(),
            ],
            ..Default::default()
        };
        assert_eq!(
            validation_targets_for_mode(
                &task,
                &[],
                crate::validator_contract::VerificationMode::AutomatedTest,
            )
            .unwrap(),
            vec![2]
        );
        assert!(validation_targets_for_mode(
            &task,
            &[3],
            crate::validator_contract::VerificationMode::AutomatedTest,
        )
        .is_err());

        let mut fallback = task;
        fallback.acceptance_ledger = vec![project::AcceptanceLedgerItem {
            criterion_index: 1,
            status: project::AcceptanceStatus::Unknown,
            evidence: "local_unprovable:需要语义审查".to_string(),
            ..Default::default()
        }];
        assert_eq!(
            validation_targets_for_mode(
                &fallback,
                &[],
                crate::validator_contract::VerificationMode::SemanticReview,
            )
            .unwrap(),
            vec![1, 3]
        );
    }

    #[test]
    fn human_review_action_accepts_only_human_review_criteria() {
        let mut project = project_with_task(project::SubtaskStatus::AwaitingConfirmation);
        let workload = project.workload_profile.as_ref().unwrap().clone();
        let task = &mut project.milestones[0].subtasks[0];
        task.acceptance_criteria = vec![
            "用户可以完成结账".to_string(),
            "操作员确认真实桌面行为".to_string(),
        ];
        task.acceptance_ledger = task
            .acceptance_criteria
            .iter()
            .enumerate()
            .map(|(index, criterion)| project::AcceptanceLedgerItem {
                criterion_index: index as u32 + 1,
                criterion: criterion.clone(),
                ..Default::default()
            })
            .collect();
        let mut contract = crate::task_contract::compile_subtask(task, None, 0, &workload);
        contract.verification_modes = vec![
            crate::validator_contract::VerificationMode::SemanticReview,
            crate::validator_contract::VerificationMode::HumanReview,
        ];
        crate::task_contract::refresh_fingerprint(&mut contract);
        task.contract_snapshot = Some(contract);

        let mut request = ControlActionRequest {
            action: ControlActionKind::Human,
            task_id: "task".to_string(),
            criterion_indexes: vec![2],
            ..Default::default()
        };
        validate_request(&project, &request).unwrap();

        request.criterion_indexes = vec![1];
        assert!(validate_request(&project, &request)
            .unwrap_err()
            .contains("不属于 HumanReview 通道"));
        let task = &project.milestones[0].subtasks[0];
        assert!(validation_targets_for_mode(
            task,
            &[2],
            crate::validator_contract::VerificationMode::SemanticReview,
        )
        .unwrap_err()
        .contains("不属于 SemanticReview 通道"));
    }

    #[test]
    fn provability_closeout_human_review_boundary_preserves_ledger_and_uses_human_validator_audit()
    {
        let mut project = project_with_task(project::SubtaskStatus::AwaitingConfirmation);
        project.workflow_state.autopilot_state = Some(project::AutopilotState::default());
        let task = &mut project.milestones[0].subtasks[0];
        task.acceptance_criteria = vec!["操作员确认真实桌面行为".to_string()];
        task.acceptance_criteria_meta =
            crate::provability::normalize_metadata(&task.acceptance_criteria, &[]);
        task.acceptance_ledger = vec![project::AcceptanceLedgerItem {
            criterion_index: 1,
            criterion: "操作员确认真实桌面行为".to_string(),
            status: project::AcceptanceStatus::Unknown,
            ..Default::default()
        }];
        project.execution_session = Some(project::ExecutionSession {
            subtask_id: "task".to_string(),
            execution_id: "execution-human".to_string(),
            active: true,
            ..Default::default()
        });
        let before = project.milestones[0].subtasks[0].acceptance_ledger.clone();
        let request = ControlActionRequest {
            action: ControlActionKind::Human,
            action_id: "human-review-1".to_string(),
            task_id: "task".to_string(),
            criterion_indexes: vec![1],
            ..Default::default()
        };

        let message = enter_human_boundary(&mut project, &request).unwrap();
        assert!(message.contains("等待显式人工结论"));
        assert_eq!(project.milestones[0].subtasks[0].acceptance_ledger, before);
        let state = project.workflow_state.autopilot_state.as_ref().unwrap();
        assert_eq!(state.run_status, project::AutopilotRunStatus::ErrorStopped);
        assert_eq!(
            state.recovery_action,
            project::AutopilotRecoveryAction::WaitHumanDecision
        );

        append_control_event(&mut project, &request, &message, true);
        let event = project.execution_history.last().unwrap();
        assert_eq!(event.validator_id.as_deref(), Some("human_boundary_review"));
        assert!(event.model_call_id.is_none());
        assert!(project.cost_ledger.calls.is_empty());
    }

    #[test]
    fn unconfigured_and_unavailable_tests_remain_unknown() {
        let task = Subtask {
            acceptance_criteria: vec!["自动化测试通过".to_string()],
            ..Default::default()
        };
        for status in [
            project::AutomatedTestStatus::NotConfigured,
            project::AutomatedTestStatus::Unavailable,
        ] {
            let evidence = crate::automated_validation::AutomatedTestEvidence {
                rendered: None,
                command: String::new(),
                exit_code: None,
                output_summary: String::new(),
                status,
            };
            let updates = automated_ledger_updates(&task, &[1], &evidence).unwrap();
            assert_eq!(updates[0].status, project::AcceptanceStatus::Unknown);
            assert_eq!(updates[0].confidence, 0.0);
        }
    }

    #[test]
    fn automated_action_audit_uses_test_runner_without_model_call() {
        let mut project = project_with_task(project::SubtaskStatus::AwaitingConfirmation);
        let request = ControlActionRequest {
            action: ControlActionKind::AutomatedValidate,
            action_id: "automated-1".to_string(),
            task_id: "task".to_string(),
            criterion_indexes: vec![1],
            ..Default::default()
        };
        append_control_event(&mut project, &request, "done", true);
        let event = project.execution_history.last().unwrap();
        assert_eq!(event.validator_id.as_deref(), Some("automated_test_runner"));
        assert!(event.model_call_id.is_none());
        assert!(project.cost_ledger.calls.is_empty());
    }
}
