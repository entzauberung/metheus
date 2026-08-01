use crate::pipeline::PipelineState;
use crate::project::Project;
use crate::recovery_presentation::{present_recovery, RecoveryPresentation};
use crate::task_control::TaskControlMode;
use crate::AppState;
use serde::{Deserialize, Serialize};

pub const RUNTIME_MUTATION_RESULT_VERSION: &str = "runtime-mutation-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSnapshot {
    pub project: Project,
    pub pipeline_state: Option<PipelineState>,
    pub process_start_id: String,
    pub event_sequence: u64,
    pub recovery_presentation: RecoveryPresentation,
    pub task_control_snapshot_version: String,
    pub task_control_tree_revision: u64,
    pub task_control_event_sequence: u64,
    pub task_control_action_id: Option<String>,
    pub task_control_mode: TaskControlMode,
    /// Best-effort detail generated from the same project/event cursor. A failure must not
    /// prevent the primary runtime and recovery state from being returned.
    pub task_control_snapshot: Option<crate::control_snapshot::TaskControlSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskControlSnapshotSummary {
    pub available: bool,
    pub snapshot_version: String,
    pub tree_revision: u64,
    pub event_sequence: u64,
    pub control_action_id: Option<String>,
    pub control_mode: TaskControlMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryResultSummary {
    pub title: String,
    pub message: String,
    pub baseline: Option<String>,
    pub baseline_summary: String,
    pub discarded_files: Vec<String>,
    pub discarded_files_summary: String,
    pub background_job_started: bool,
    pub background_job_summary: String,
    pub next_step: String,
    pub next_step_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeActionSummary {
    pub action: String,
    pub message: String,
    pub notify_user: bool,
    pub recovery_result: Option<RecoveryResultSummary>,
}

impl RuntimeActionSummary {
    pub fn silent(action: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            message: String::new(),
            notify_user: false,
            recovery_result: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeMutationResult {
    pub result_version: String,
    pub runtime_snapshot: RuntimeSnapshot,
    pub task_control: TaskControlSnapshotSummary,
    pub action: RuntimeActionSummary,
    pub task_control_snapshot: Option<crate::control_snapshot::TaskControlSnapshot>,
}

pub(crate) fn load_consistent_project(
    project_name: &str,
) -> Result<(Project, crate::project_state_bus::ProjectStateSubscription), String> {
    for _ in 0..5 {
        let before = crate::project_state_bus::project_state_cursor(project_name)?;
        let project = crate::load_project(project_name)?;
        let after = crate::project_state_bus::project_state_cursor(project_name)?;
        if before.process_start_id == after.process_start_id
            && before.event_sequence == after.event_sequence
        {
            return Ok((project, after));
        }
    }
    Err("项目状态持续变化，暂时无法取得一致运行时快照，请重试".to_string())
}

pub(crate) fn compose_runtime_snapshot(
    project: Project,
    pipeline_state: Option<PipelineState>,
    cursor: crate::project_state_bus::ProjectStateSubscription,
) -> RuntimeSnapshot {
    let pipeline_state = pipeline_state.filter(|pipeline| pipeline.project_name == project.name);
    let recovery_presentation = present_recovery(&project);
    let task_control_action_id = if !project.task_control.active_action_id.is_empty() {
        Some(project.task_control.active_action_id.clone())
    } else if !project.task_control.last_completed_action_id.is_empty() {
        Some(project.task_control.last_completed_action_id.clone())
    } else {
        None
    };
    RuntimeSnapshot {
        task_control_snapshot_version: project.task_control.snapshot_version.clone(),
        task_control_tree_revision: project.task_control.tree_revision,
        task_control_event_sequence: cursor.event_sequence,
        task_control_action_id,
        task_control_mode: project.task_control.mode,
        project,
        pipeline_state,
        process_start_id: cursor.process_start_id,
        event_sequence: cursor.event_sequence,
        recovery_presentation,
        task_control_snapshot: None,
    }
}

pub(crate) fn mutation_result(
    project_name: &str,
    pipeline_state: Option<PipelineState>,
    action: RuntimeActionSummary,
    include_task_control_snapshot: bool,
) -> Result<RuntimeMutationResult, String> {
    let (project, cursor) = load_consistent_project(project_name)?;
    let same_cursor_task_control_snapshot = crate::control_snapshot::build_at_event(
        &project,
        &cursor.process_start_id,
        cursor.event_sequence,
    )
    .ok();
    let task_control_available =
        !include_task_control_snapshot || same_cursor_task_control_snapshot.is_some();
    let task_control_snapshot = include_task_control_snapshot
        .then(|| same_cursor_task_control_snapshot.clone())
        .flatten();
    let task_control = TaskControlSnapshotSummary {
        available: task_control_available,
        snapshot_version: project.task_control.snapshot_version.clone(),
        tree_revision: project.task_control.tree_revision,
        event_sequence: cursor.event_sequence,
        control_action_id: if !project.task_control.active_action_id.is_empty() {
            Some(project.task_control.active_action_id.clone())
        } else if !project.task_control.last_completed_action_id.is_empty() {
            Some(project.task_control.last_completed_action_id.clone())
        } else {
            None
        },
        control_mode: project.task_control.mode,
    };
    let mut runtime_snapshot = compose_runtime_snapshot(project, pipeline_state, cursor);
    runtime_snapshot.task_control_snapshot = same_cursor_task_control_snapshot;
    Ok(RuntimeMutationResult {
        result_version: RUNTIME_MUTATION_RESULT_VERSION.to_string(),
        runtime_snapshot,
        task_control,
        action,
        task_control_snapshot,
    })
}

#[tauri::command]
pub(crate) async fn get_runtime_snapshot(
    state: tauri::State<'_, AppState>,
    project_name: String,
    include_task_control_snapshot: Option<bool>,
) -> Result<RuntimeSnapshot, String> {
    if project_name.trim().is_empty() {
        return Err("读取运行时快照时项目名不能为空".to_string());
    }
    // Execution writers use this lock while committing their durable Project transition.
    // Taking it before the disk read prevents an old Project from being paired with a newer
    // PipelineState when a fallback snapshot races a terminal transition.
    let pipeline_guard = state.pipeline_state.lock().await;
    let (project, cursor) = load_consistent_project(&project_name)?;
    let pipeline_state = pipeline_guard.clone();
    let task_control_snapshot = include_task_control_snapshot
        .unwrap_or(false)
        .then(|| {
            crate::control_snapshot::build_at_event(
                &project,
                &cursor.process_start_id,
                cursor.event_sequence,
            )
        })
        .and_then(Result::ok);
    let mut snapshot = compose_runtime_snapshot(project, pipeline_state, cursor);
    snapshot.task_control_snapshot = task_control_snapshot;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::PipelineStatus;
    use crate::project::{ExecutionSession, Project};
    use crate::recovery_presentation::RecoveryPresentationKind;

    fn pipeline(project_name: &str) -> PipelineState {
        PipelineState {
            execution_id: "execution-1".to_string(),
            mid_stage_id: "mid-stage-1".to_string(),
            status: PipelineStatus::Failed,
            current_subtask_index: 0,
            total_subtasks: 1,
            subtask_statuses: Vec::new(),
            current_log: "stopped".to_string(),
            last_error: Some("session lost".to_string()),
            child_pid: None,
            project_name: project_name.to_string(),
            milestone_id: "milestone-1".to_string(),
            plan_revision: 1,
            current_subtask_id: "task-1".to_string(),
            awaiting_confirmation: false,
            log_history: Vec::new(),
        }
    }

    fn cursor(sequence: u64) -> crate::project_state_bus::ProjectStateSubscription {
        crate::project_state_bus::ProjectStateSubscription {
            subscription_id: String::new(),
            process_start_id: "process-1".to_string(),
            event_sequence: sequence,
        }
    }

    #[test]
    fn runtime_snapshot_combines_matching_pipeline_and_disk_recovery() {
        let mut project = Project::new("runtime-snapshot");
        project.execution_session = Some(ExecutionSession {
            execution_id: "execution-1".to_string(),
            status: "execution_failed".to_string(),
            ..ExecutionSession::default()
        });

        let snapshot =
            compose_runtime_snapshot(project, Some(pipeline("runtime-snapshot")), cursor(7));

        assert!(snapshot.pipeline_state.is_some());
        assert_eq!(snapshot.event_sequence, 7);
        assert!(snapshot.task_control_snapshot.is_none());
        assert_eq!(
            snapshot.recovery_presentation.kind,
            RecoveryPresentationKind::BaselineRecovery
        );
    }

    #[test]
    fn runtime_snapshot_excludes_pipeline_from_another_project() {
        let project = Project::new("runtime-snapshot-b");
        let snapshot =
            compose_runtime_snapshot(project, Some(pipeline("runtime-snapshot-a")), cursor(3));

        assert!(snapshot.pipeline_state.is_none());
        assert_eq!(snapshot.project.name, "runtime-snapshot-b");
    }

    #[test]
    fn runtime_snapshot_accepts_same_cursor_task_control_detail_without_blocking_primary() {
        let project = Project::new("runtime-snapshot-detail");
        let cursor = cursor(11);
        let detail = crate::control_snapshot::build_at_event(
            &project,
            &cursor.process_start_id,
            cursor.event_sequence,
        )
        .expect("详细快照");
        let mut snapshot = compose_runtime_snapshot(project, None, cursor);
        snapshot.task_control_snapshot = Some(detail);

        let detail = snapshot.task_control_snapshot.as_ref().expect("同源详情");
        assert_eq!(detail.source_process_start_id, snapshot.process_start_id);
        assert_eq!(
            detail.source_event_sequence,
            snapshot.task_control_event_sequence
        );
        assert_eq!(
            detail.project_revision,
            snapshot.project.workflow_state.data_revision
        );
        assert_eq!(
            detail.task_tree_revision,
            snapshot.task_control_tree_revision
        );
        assert_eq!(
            detail.source_control_action_id,
            snapshot.task_control_action_id
        );
    }
}
