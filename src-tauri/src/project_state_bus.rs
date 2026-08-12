use crate::project::{AutopilotRecoveryAction, AutopilotRunStatus, Project, WorkflowStep};
use crate::task_control::TaskControlMode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tauri::ipc::Channel;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectStateChangedEvent {
    pub project_name: String,
    pub process_start_id: String,
    pub event_sequence: u64,
    pub data_revision: u64,
    pub current_step: WorkflowStep,
    pub execution_session_status: Option<String>,
    pub autopilot_status: Option<AutopilotRunStatus>,
    pub recovery_action: AutopilotRecoveryAction,
    pub task_control_tree_revision: u64,
    pub task_control_snapshot_version: String,
    pub control_action_id: Option<String>,
    pub control_mode: TaskControlMode,
    pub task_control_dirty: bool,
    /// 仅运行期快照（如流式日志）已变化；项目数据修订号可以保持不变。
    #[serde(default)]
    pub runtime_dirty: bool,
    pub occurred_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectStateSubscription {
    pub subscription_id: String,
    pub process_start_id: String,
    pub event_sequence: u64,
}

struct ProjectSubscriber {
    id: String,
    channel: Channel<ProjectStateChangedEvent>,
}

#[derive(Default)]
struct ProjectStream {
    event_sequence: u64,
    task_control_state: Option<TaskControlEventState>,
    recovery_state: Option<RecoveryEventState>,
    subscribers: Vec<ProjectSubscriber>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskControlEventState {
    tree_revision: u64,
    control_action_id: Option<String>,
    control_mode: TaskControlMode,
    snapshot_version: String,
}

impl TaskControlEventState {
    fn from_project(project: &Project) -> Self {
        Self {
            tree_revision: project.task_control.tree_revision,
            control_action_id: current_control_action_id(project),
            control_mode: project.task_control.mode,
            snapshot_version: project.task_control.snapshot_version.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RecoveryEventState {
    phase: Option<crate::project::RecoveryPhase>,
    error_kind: Option<crate::project::RecoveryErrorKind>,
    progress_at: Option<String>,
    action_id: String,
    action_kind: String,
    action_started_at: String,
    autopilot_status: Option<AutopilotRunStatus>,
    recovery_action: AutopilotRecoveryAction,
    next_retry_at: Option<String>,
    next_validation_retry_at: Option<String>,
    heartbeat_bucket: Option<i64>,
}

impl RecoveryEventState {
    fn from_project(project: &Project) -> Option<Self> {
        let recovery = project.workflow_state.recovery_state.as_ref();
        let autopilot = project.workflow_state.autopilot_state.as_ref();
        let recovery_action = autopilot
            .map(|state| state.recovery_action.clone())
            .unwrap_or_default();
        let claimed = autopilot
            .map(|state| {
                !state.current_action_id.is_empty()
                    && state.current_action_kind == "run_error_recovery"
            })
            .unwrap_or(false);
        let relevant_action = matches!(
            recovery_action,
            AutopilotRecoveryAction::RestoreExecutionBaseline
                | AutopilotRecoveryAction::RunAutomaticRecovery
                | AutopilotRecoveryAction::WaitHumanDecision
        );
        if recovery.is_none() && !claimed && !relevant_action {
            return None;
        }
        let heartbeat_bucket = autopilot
            .filter(|state| claimed && state.run_status == AutopilotRunStatus::Running)
            .and_then(|state| chrono::DateTime::parse_from_rfc3339(&state.heartbeat_at).ok())
            .map(|heartbeat| heartbeat.timestamp().div_euclid(5));
        Some(Self {
            phase: recovery.map(|state| state.phase.clone()),
            error_kind: recovery.map(|state| state.error_kind.clone()),
            progress_at: recovery
                .map(|state| state.updated_at.clone())
                .filter(|value| !value.is_empty()),
            action_id: autopilot
                .map(|state| state.current_action_id.clone())
                .unwrap_or_default(),
            action_kind: autopilot
                .map(|state| state.current_action_kind.clone())
                .unwrap_or_default(),
            action_started_at: autopilot
                .map(|state| state.action_started_at.clone())
                .unwrap_or_default(),
            autopilot_status: autopilot.map(|state| state.run_status.clone()),
            recovery_action,
            next_retry_at: autopilot.and_then(|state| state.next_retry_at.clone()),
            next_validation_retry_at: recovery
                .and_then(|state| state.next_validation_retry_at.clone()),
            heartbeat_bucket,
        })
    }
}

pub struct ProjectStateBus {
    process_start_id: String,
    streams: Mutex<HashMap<String, ProjectStream>>,
}

impl Default for ProjectStateBus {
    fn default() -> Self {
        Self {
            process_start_id: uuid::Uuid::new_v4().to_string(),
            streams: Mutex::new(HashMap::new()),
        }
    }
}

impl ProjectStateBus {
    fn subscribe(
        &self,
        project_name: &str,
        channel: Channel<ProjectStateChangedEvent>,
    ) -> Result<ProjectStateSubscription, String> {
        let mut streams = self
            .streams
            .lock()
            .map_err(|_| "项目状态通知总线锁已损坏".to_string())?;
        let stream = streams.entry(project_name.to_string()).or_default();
        let subscription_id = uuid::Uuid::new_v4().to_string();
        stream.subscribers.push(ProjectSubscriber {
            id: subscription_id.clone(),
            channel,
        });
        Ok(ProjectStateSubscription {
            subscription_id,
            process_start_id: self.process_start_id.clone(),
            event_sequence: stream.event_sequence,
        })
    }

    fn unsubscribe(&self, subscription_id: &str) -> Result<(), String> {
        let mut streams = self
            .streams
            .lock()
            .map_err(|_| "项目状态通知总线锁已损坏".to_string())?;
        for stream in streams.values_mut() {
            stream
                .subscribers
                .retain(|subscriber| subscriber.id != subscription_id);
        }
        streams.retain(|_, stream| stream.event_sequence > 0 || !stream.subscribers.is_empty());
        Ok(())
    }

    fn publish(&self, project: &Project) -> Result<ProjectStateChangedEvent, String> {
        self.publish_with_runtime_state(project, false)
    }

    fn publish_with_runtime_state(
        &self,
        project: &Project,
        runtime_dirty: bool,
    ) -> Result<ProjectStateChangedEvent, String> {
        let mut streams = self
            .streams
            .lock()
            .map_err(|_| "项目状态通知总线锁已损坏".to_string())?;
        let stream = streams.entry(project.name.clone()).or_default();
        stream.event_sequence = stream.event_sequence.saturating_add(1);
        let task_control_state = TaskControlEventState::from_project(project);
        let task_control_dirty = stream
            .task_control_state
            .as_ref()
            .map(|previous| previous != &task_control_state)
            .unwrap_or(true);
        stream.task_control_state = Some(task_control_state);
        let recovery_state = RecoveryEventState::from_project(project);
        let recovery_dirty = stream.recovery_state.as_ref() != recovery_state.as_ref();
        stream.recovery_state = recovery_state;
        let event = event_from_project(
            project,
            &self.process_start_id,
            stream.event_sequence,
            task_control_dirty,
            runtime_dirty || recovery_dirty,
        );

        // Keep publication and delivery under the same lock so concurrent saves cannot
        // reorder events for a project. A disconnected Channel is removed immediately.
        stream
            .subscribers
            .retain(|subscriber| subscriber.channel.send(event.clone()).is_ok());
        Ok(event)
    }

    fn cursor(&self, project_name: &str) -> Result<ProjectStateSubscription, String> {
        let streams = self
            .streams
            .lock()
            .map_err(|_| "项目状态通知总线锁已损坏".to_string())?;
        Ok(ProjectStateSubscription {
            subscription_id: String::new(),
            process_start_id: self.process_start_id.clone(),
            event_sequence: streams
                .get(project_name)
                .map(|stream| stream.event_sequence)
                .unwrap_or(0),
        })
    }
}

fn event_from_project(
    project: &Project,
    process_start_id: &str,
    event_sequence: u64,
    task_control_dirty: bool,
    runtime_dirty: bool,
) -> ProjectStateChangedEvent {
    let autopilot = project.workflow_state.autopilot_state.as_ref();
    ProjectStateChangedEvent {
        project_name: project.name.clone(),
        process_start_id: process_start_id.to_string(),
        event_sequence,
        data_revision: project.workflow_state.data_revision,
        current_step: project.workflow_state.current_step.clone(),
        execution_session_status: project
            .execution_session
            .as_ref()
            .map(|session| session.status.clone()),
        autopilot_status: autopilot.map(|state| state.run_status.clone()),
        recovery_action: autopilot
            .map(|state| state.recovery_action.clone())
            .unwrap_or_default(),
        task_control_tree_revision: project.task_control.tree_revision,
        task_control_snapshot_version: project.task_control.snapshot_version.clone(),
        control_action_id: current_control_action_id(project),
        control_mode: project.task_control.mode,
        task_control_dirty,
        runtime_dirty,
        occurred_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn current_control_action_id(project: &Project) -> Option<String> {
    if !project.task_control.active_action_id.is_empty() {
        Some(project.task_control.active_action_id.clone())
    } else if !project.task_control.last_completed_action_id.is_empty() {
        Some(project.task_control.last_completed_action_id.clone())
    } else {
        None
    }
}

fn bus() -> &'static ProjectStateBus {
    static BUS: OnceLock<ProjectStateBus> = OnceLock::new();
    BUS.get_or_init(ProjectStateBus::default)
}

pub(crate) fn process_start_id() -> &'static str {
    bus().process_start_id.as_str()
}

pub(crate) fn subscribe_project_state_channel(
    project_name: &str,
    channel: Channel<ProjectStateChangedEvent>,
) -> Result<ProjectStateSubscription, String> {
    bus().subscribe(project_name, channel)
}

pub(crate) fn unsubscribe_project_state_channel(subscription_id: &str) -> Result<(), String> {
    bus().unsubscribe(subscription_id)
}

pub(crate) fn publish_project_state(project: &Project) -> Result<ProjectStateChangedEvent, String> {
    bus().publish(project)
}

pub(crate) fn publish_project_runtime_state(
    project_name: &str,
) -> Result<ProjectStateChangedEvent, String> {
    let project = crate::load_project(project_name)?;
    bus().publish_with_runtime_state(&project, true)
}

pub(crate) fn project_state_cursor(project_name: &str) -> Result<ProjectStateSubscription, String> {
    bus().cursor(project_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_revision_changes_still_receive_distinct_sequences() -> Result<(), String> {
        let bus = ProjectStateBus::default();
        let project = Project::new("state-sequence");

        let first = bus.publish(&project)?;
        let second = bus.publish(&project)?;

        assert_eq!(first.data_revision, second.data_revision);
        assert_eq!(second.event_sequence, first.event_sequence + 1);
        assert_eq!(first.process_start_id, second.process_start_id);
        assert!(first.task_control_dirty);
        assert!(!second.task_control_dirty);
        assert!(!first.runtime_dirty);
        assert!(!second.runtime_dirty);
        Ok(())
    }

    #[test]
    fn runtime_fix_runtime_change_is_explicit_without_data_revision() -> Result<(), String> {
        let bus = ProjectStateBus::default();
        let project = Project::new("runtime-state-sequence");

        let persisted = bus.publish(&project)?;
        let runtime = bus.publish_with_runtime_state(&project, true)?;

        assert_eq!(runtime.data_revision, persisted.data_revision);
        assert_eq!(runtime.event_sequence, persisted.event_sequence + 1);
        assert!(runtime.runtime_dirty);
        Ok(())
    }

    #[test]
    fn task_control_changes_are_explicitly_invalidated() -> Result<(), String> {
        let bus = ProjectStateBus::default();
        let mut project = Project::new("task-control-dirty");

        let first = bus.publish(&project)?;
        let project_only = bus.publish(&project)?;
        project.task_control.tree_revision += 1;
        let task_control = bus.publish(&project)?;

        assert!(first.task_control_dirty);
        assert!(!project_only.task_control_dirty);
        assert!(task_control.task_control_dirty);
        assert_eq!(task_control.task_control_tree_revision, 1);
        Ok(())
    }

    #[test]
    fn active_recovery_sync_is_bounded() -> Result<(), String> {
        let bus = ProjectStateBus::default();
        let mut project = Project::new("active-recovery-sync");
        project.workflow_state.recovery_state = Some(crate::project::RecoveryState {
            phase: crate::project::RecoveryPhase::Repairing,
            error_kind: crate::project::RecoveryErrorKind::TestFailure,
            updated_at: "2026-08-11T12:00:01Z".to_string(),
            ..Default::default()
        });
        project.workflow_state.autopilot_state = Some(crate::project::AutopilotState {
            active: true,
            run_status: AutopilotRunStatus::Running,
            recovery_action: AutopilotRecoveryAction::RunAutomaticRecovery,
            current_action_id: "claim-1".to_string(),
            current_action_kind: "run_error_recovery".to_string(),
            action_started_at: "2026-08-11T12:00:00Z".to_string(),
            heartbeat_at: "2026-08-11T12:00:01Z".to_string(),
            ..Default::default()
        });

        assert!(bus.publish(&project)?.runtime_dirty);
        assert!(!bus.publish(&project)?.runtime_dirty);

        project
            .workflow_state
            .autopilot_state
            .as_mut()
            .expect("autopilot")
            .heartbeat_at = "2026-08-11T12:00:04Z".to_string();
        assert!(!bus.publish(&project)?.runtime_dirty);

        project
            .workflow_state
            .autopilot_state
            .as_mut()
            .expect("autopilot")
            .heartbeat_at = "2026-08-11T12:00:06Z".to_string();
        assert!(bus.publish(&project)?.runtime_dirty);
        assert!(!bus.publish(&project)?.runtime_dirty);

        project
            .workflow_state
            .recovery_state
            .as_mut()
            .expect("recovery")
            .updated_at = "2026-08-11T12:00:07Z".to_string();
        assert!(bus.publish(&project)?.runtime_dirty);

        project
            .workflow_state
            .recovery_state
            .as_mut()
            .expect("recovery")
            .phase = crate::project::RecoveryPhase::WaitingHuman;
        let autopilot = project
            .workflow_state
            .autopilot_state
            .as_mut()
            .expect("autopilot");
        autopilot.run_status = AutopilotRunStatus::ErrorStopped;
        autopilot.recovery_action = AutopilotRecoveryAction::WaitHumanDecision;
        autopilot.current_action_id.clear();
        autopilot.current_action_kind.clear();
        autopilot.action_started_at.clear();
        assert!(bus.publish(&project)?.runtime_dirty);
        assert!(!bus.publish(&project)?.runtime_dirty);

        let normal = Project::new("normal-sync-frequency");
        assert!(!bus.publish(&normal)?.runtime_dirty);
        assert!(!bus.publish(&normal)?.runtime_dirty);
        Ok(())
    }

    #[test]
    fn project_sequences_are_isolated() -> Result<(), String> {
        let bus = ProjectStateBus::default();
        let first = Project::new("state-project-a");
        let second = Project::new("state-project-b");

        assert_eq!(bus.publish(&first)?.event_sequence, 1);
        assert_eq!(bus.publish(&first)?.event_sequence, 2);
        assert_eq!(bus.publish(&second)?.event_sequence, 1);
        assert_eq!(bus.cursor(&first.name)?.event_sequence, 2);
        assert_eq!(bus.cursor(&second.name)?.event_sequence, 1);
        Ok(())
    }

    #[test]
    fn alternate_path_writes_do_not_publish() -> Result<(), String> {
        let project_name = format!("state-alternate-{}", uuid::Uuid::new_v4());
        let project = Project::new(&project_name);
        let temp_root =
            std::env::temp_dir().join(format!("metheus-state-bus-{}", uuid::Uuid::new_v4()));
        let path = temp_root.join("project.json");

        crate::save_project_to_path(&project, &path)?;
        assert_eq!(project_state_cursor(&project_name)?.event_sequence, 0);

        std::fs::remove_dir_all(&temp_root)
            .map_err(|error| format!("清理状态总线测试目录失败：{error}"))?;
        Ok(())
    }

    #[test]
    fn official_save_publishes_once_after_success() -> Result<(), String> {
        let project_name = format!("state-official-{}", uuid::Uuid::new_v4());
        let mut project = Project::new(&project_name);
        project.workflow_state.data_revision = 7;

        crate::save_project(&project)?;
        let cursor = project_state_cursor(&project_name)?;
        assert_eq!(cursor.event_sequence, 1);

        let path = crate::project_data_path(&project_name)?;
        std::fs::remove_file(path).map_err(|error| format!("清理状态总线测试项目失败：{error}"))?;
        Ok(())
    }

    #[test]
    fn failed_official_save_does_not_publish() -> Result<(), String> {
        let project_name = format!("state-failure-{}\0", uuid::Uuid::new_v4());
        let project = Project::new(&project_name);

        assert!(crate::save_project(&project).is_err());
        assert_eq!(project_state_cursor(&project_name)?.event_sequence, 0);
        Ok(())
    }
}
