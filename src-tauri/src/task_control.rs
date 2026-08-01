use crate::project::Project;
use serde::{Deserialize, Serialize};

pub const TASK_CONTROL_ALGORITHM_VERSION: &str = "v0.0.4-phase1-final";
pub const TASK_CONTROL_SNAPSHOT_VERSION: &str = "task-control-snapshot-v2";
pub const SERIAL_TAKEOVER_CONTRACT_VERSION: &str = "serial-takeover-v1";
/// v0.0.4 第一阶段正式收口：仅新项目默认进入串行接管，磁盘旧项目保持原模式。
pub const PHASE1_DEFAULT_TASK_CONTROL_MODE: TaskControlMode = TaskControlMode::SerialTakeover;

/// Controls migration from the existing fixed workflow to the task controller.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum TaskControlMode {
    #[default]
    Legacy,
    Shadow,
    SerialTakeover,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum TakeoverCapabilityStatus {
    #[default]
    Unknown,
    Ready,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskActionFamily {
    Execute,
    Confirm,
    Repair,
    Wait,
    Human,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ShadowComparisonOutcome {
    Match,
    Difference,
    Uncomparable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShadowDecisionComparison {
    pub compared_at: String,
    pub shadow_decision_id: String,
    pub shadow_action: String,
    pub legacy_command: String,
    pub shadow_family: Option<TaskActionFamily>,
    pub legacy_family: Option<TaskActionFamily>,
    pub outcome: ShadowComparisonOutcome,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ShadowComparisonMetrics {
    pub evaluated: u64,
    pub comparable_matches: u64,
    pub comparable_differences: u64,
    pub uncomparable: u64,
    pub latest: Option<ShadowDecisionComparison>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskControlModeChangeRecord {
    pub from: TaskControlMode,
    pub to: TaskControlMode,
    pub source: String,
    pub reason: String,
    pub changed_at: String,
    pub project_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskControlState {
    #[serde(default)]
    pub mode: TaskControlMode,
    #[serde(default = "default_algorithm_version")]
    pub algorithm_version: String,
    #[serde(default = "default_snapshot_version")]
    pub snapshot_version: String,
    #[serde(default = "default_takeover_version")]
    pub takeover_version: String,
    #[serde(default)]
    pub takeover_capability_status: TakeoverCapabilityStatus,
    #[serde(default)]
    pub last_takeover_check_result: String,
    #[serde(default)]
    pub takeover_unavailable_reason: String,
    #[serde(default)]
    pub takeover_checked_at: Option<String>,
    #[serde(default)]
    pub mode_change_history: Vec<TaskControlModeChangeRecord>,
    #[serde(default)]
    pub last_shadow_decision_at: Option<String>,
    #[serde(default)]
    pub last_shadow_decision_summary: String,
    #[serde(default)]
    pub shadow_comparison: ShadowComparisonMetrics,
    #[serde(default)]
    pub last_decision_id: String,
    #[serde(default)]
    pub last_decision_fingerprint: String,
    #[serde(default)]
    pub last_decision: Option<crate::control_scheduler::TaskControlDecision>,
    #[serde(default)]
    pub control_source: String,
    #[serde(default)]
    pub tree_revision: u64,
    #[serde(default)]
    pub active_action_id: String,
    #[serde(default)]
    pub active_action_kind: String,
    #[serde(default)]
    pub active_action_task_id: String,
    #[serde(default)]
    pub last_completed_action_id: String,
    #[serde(default)]
    pub last_completed_action_kind: String,
    #[serde(default)]
    pub last_completed_action_task_id: String,
    #[serde(default)]
    pub last_action_result: String,
    #[serde(default)]
    pub last_action_made_progress: bool,
    #[serde(default)]
    pub last_action_before_fingerprint: String,
    #[serde(default)]
    pub last_action_after_fingerprint: String,
    #[serde(default)]
    pub last_action_at: Option<String>,
}

fn default_algorithm_version() -> String {
    TASK_CONTROL_ALGORITHM_VERSION.to_string()
}

fn default_snapshot_version() -> String {
    TASK_CONTROL_SNAPSHOT_VERSION.to_string()
}

fn default_takeover_version() -> String {
    SERIAL_TAKEOVER_CONTRACT_VERSION.to_string()
}

impl Default for TaskControlState {
    fn default() -> Self {
        Self {
            mode: TaskControlMode::Legacy,
            algorithm_version: default_algorithm_version(),
            snapshot_version: default_snapshot_version(),
            takeover_version: default_takeover_version(),
            takeover_capability_status: TakeoverCapabilityStatus::Unknown,
            last_takeover_check_result: String::new(),
            takeover_unavailable_reason: String::new(),
            takeover_checked_at: None,
            mode_change_history: Vec::new(),
            last_shadow_decision_at: None,
            last_shadow_decision_summary: String::new(),
            shadow_comparison: ShadowComparisonMetrics::default(),
            last_decision_id: String::new(),
            last_decision_fingerprint: String::new(),
            last_decision: None,
            control_source: "legacy_workflow".to_string(),
            tree_revision: 0,
            active_action_id: String::new(),
            active_action_kind: String::new(),
            active_action_task_id: String::new(),
            last_completed_action_id: String::new(),
            last_completed_action_kind: String::new(),
            last_completed_action_task_id: String::new(),
            last_action_result: String::new(),
            last_action_made_progress: false,
            last_action_before_fingerprint: String::new(),
            last_action_after_fingerprint: String::new(),
            last_action_at: None,
        }
    }
}

fn inspect_task_capabilities(
    tasks: &[crate::project::Subtask],
    parent_id: Option<&str>,
    depth: u32,
) -> Result<(), String> {
    for task in tasks {
        let compiled = crate::task_compiler::compile(task, parent_id, depth);
        if compiled.contract.fingerprint.trim().is_empty() {
            return Err(format!("任务 {} 无法生成稳定合同指纹", task.id));
        }
        for criterion in &task.acceptance_criteria {
            if crate::validator_registry::validators_for(criterion).is_empty() {
                return Err(format!(
                    "任务 {} 的验收项缺少验证器：{}",
                    task.id, criterion
                ));
            }
        }
        inspect_task_capabilities(&task.child_tasks, Some(&task.id), depth.saturating_add(1))?;
    }
    Ok(())
}

pub fn inspect_serial_takeover_capability(project: &Project) -> Result<(), String> {
    crate::task_tree::validate_project_tree(project)
        .map_err(|reason| format!("任务树不可用：{}", reason))?;
    for milestone in &project.milestones {
        inspect_task_capabilities(&milestone.subtasks, None, 0)?;
        for stage in &milestone.mid_stages {
            inspect_task_capabilities(&stage.subtasks, None, 0)?;
        }
    }
    crate::control_action_executor::ensure_serial_takeover_actions_available()?;
    crate::validator_registry::ensure_serial_takeover_validators_available()?;
    crate::control_snapshot::build(project)
        .map_err(|reason| format!("任务控制快照不可用：{}", reason))?;
    Ok(())
}

/// Refreshes the cached takeover contract only while there is no active execution session.
/// An in-flight task must keep the boundary that was checked before dispatch.
pub fn refresh_serial_takeover_capability(project: &mut Project) -> bool {
    if project
        .execution_session
        .as_ref()
        .is_some_and(|session| session.active)
    {
        return project.task_control.takeover_capability_status == TakeoverCapabilityStatus::Ready;
    }
    let checked_at = chrono::Utc::now().to_rfc3339();
    let result = inspect_serial_takeover_capability(project);
    project.task_control.takeover_version = SERIAL_TAKEOVER_CONTRACT_VERSION.to_string();
    project.task_control.takeover_checked_at = Some(checked_at);
    match result {
        Ok(()) => {
            project.task_control.takeover_capability_status = TakeoverCapabilityStatus::Ready;
            project.task_control.last_takeover_check_result =
                "任务合同、任务树、动作执行器、验证器和控制快照均可用".to_string();
            project.task_control.takeover_unavailable_reason.clear();
            true
        }
        Err(reason) => {
            project.task_control.takeover_capability_status = TakeoverCapabilityStatus::Unavailable;
            project.task_control.last_takeover_check_result = reason.clone();
            project.task_control.takeover_unavailable_reason = reason;
            false
        }
    }
}

pub fn ensure_serial_takeover_capability(project: &Project) -> Result<(), String> {
    if project.task_control.takeover_version != SERIAL_TAKEOVER_CONTRACT_VERSION {
        return Err(format!(
            "正式接管契约版本不匹配：项目={}，当前={}",
            project.task_control.takeover_version, SERIAL_TAKEOVER_CONTRACT_VERSION
        ));
    }
    if project.task_control.takeover_capability_status != TakeoverCapabilityStatus::Ready {
        let reason = if project.task_control.takeover_unavailable_reason.is_empty() {
            "正式接管能力尚未通过检查".to_string()
        } else {
            project.task_control.takeover_unavailable_reason.clone()
        };
        return Err(reason);
    }
    Ok(())
}

impl TaskControlState {
    pub fn for_new_project() -> Self {
        Self {
            mode: PHASE1_DEFAULT_TASK_CONTROL_MODE,
            control_source: "task_controller".to_string(),
            ..Self::default()
        }
    }

    pub fn record_shadow_comparison(
        &mut self,
        shadow: &crate::control_scheduler::TaskControlDecision,
        legacy: &crate::autopilot_policy::AutopilotNextStep,
    ) {
        let shadow_family = shadow_action_family(shadow.action.kind);
        let legacy_family = legacy_action_family(legacy);
        let (outcome, reason) = match (shadow_family, legacy_family) {
            (Some(left), Some(right)) if left == right => (
                ShadowComparisonOutcome::Match,
                "新旧控制器选择了相同的任务动作族".to_string(),
            ),
            (Some(_), Some(_)) => (
                ShadowComparisonOutcome::Difference,
                "新旧控制器选择了不同的任务动作族".to_string(),
            ),
            _ => (
                ShadowComparisonOutcome::Uncomparable,
                "动作粒度不同，保留为不可比较样本".to_string(),
            ),
        };
        self.shadow_comparison.evaluated = self.shadow_comparison.evaluated.saturating_add(1);
        match outcome {
            ShadowComparisonOutcome::Match => {
                self.shadow_comparison.comparable_matches =
                    self.shadow_comparison.comparable_matches.saturating_add(1);
            }
            ShadowComparisonOutcome::Difference => {
                self.shadow_comparison.comparable_differences = self
                    .shadow_comparison
                    .comparable_differences
                    .saturating_add(1);
            }
            ShadowComparisonOutcome::Uncomparable => {
                self.shadow_comparison.uncomparable =
                    self.shadow_comparison.uncomparable.saturating_add(1);
            }
        }
        self.shadow_comparison.latest = Some(ShadowDecisionComparison {
            compared_at: chrono::Utc::now().to_rfc3339(),
            shadow_decision_id: shadow.decision_id.clone(),
            shadow_action: shadow.action.kind.as_str().to_string(),
            legacy_command: legacy.command.clone(),
            shadow_family,
            legacy_family,
            outcome,
            reason,
        });
    }
}

fn shadow_action_family(
    action: crate::control_action::ControlActionKind,
) -> Option<TaskActionFamily> {
    use crate::control_action::ControlActionKind;
    match action {
        ControlActionKind::Execute => Some(TaskActionFamily::Execute),
        ControlActionKind::GitConfirm => Some(TaskActionFamily::Confirm),
        ControlActionKind::Repair => Some(TaskActionFamily::Repair),
        ControlActionKind::Wait => Some(TaskActionFamily::Wait),
        ControlActionKind::Human | ControlActionKind::AcceptDeviation => {
            Some(TaskActionFamily::Human)
        }
        ControlActionKind::Split
        | ControlActionKind::LocalValidate
        | ControlActionKind::AutomatedValidate
        | ControlActionKind::TargetedValidate
        | ControlActionKind::Recompile => None,
    }
}

fn legacy_action_family(
    decision: &crate::autopilot_policy::AutopilotNextStep,
) -> Option<TaskActionFamily> {
    match decision.command.as_str() {
        "execute_current_subtask" => Some(TaskActionFamily::Execute),
        "confirm_subtask_result" => Some(TaskActionFamily::Confirm),
        "run_error_recovery" | "retry_current_subtask" => Some(TaskActionFamily::Repair),
        "calibrate_next_subtask_command" => None,
        "" if decision.waiting_for_execution => Some(TaskActionFamily::Wait),
        "" if decision.is_error => Some(TaskActionFamily::Human),
        _ => None,
    }
}

pub fn hydrate_project(project: &mut Project) -> Result<(), String> {
    project.cost_ledger.rebuild_summaries();
    if project.task_control.algorithm_version.is_empty() {
        project.task_control.algorithm_version = TASK_CONTROL_ALGORITHM_VERSION.to_string();
    }
    if project.task_control.snapshot_version.is_empty() {
        project.task_control.snapshot_version = TASK_CONTROL_SNAPSHOT_VERSION.to_string();
    }
    if project.task_control.takeover_version.is_empty() {
        project.task_control.takeover_version = SERIAL_TAKEOVER_CONTRACT_VERSION.to_string();
    }
    if project.task_control.control_source.is_empty() {
        project.task_control.control_source = match project.task_control.mode {
            TaskControlMode::Legacy => "legacy_workflow",
            TaskControlMode::Shadow => "shadow_controller",
            TaskControlMode::SerialTakeover => "task_controller",
        }
        .to_string();
    }
    let mut has_dynamic_tasks = false;
    for milestone in &mut project.milestones {
        hydrate_task_contracts(&mut milestone.subtasks, None, 0, &mut has_dynamic_tasks)?;
        for stage in &mut milestone.mid_stages {
            hydrate_task_contracts(&mut stage.subtasks, None, 0, &mut has_dynamic_tasks)?;
        }
    }
    if has_dynamic_tasks && project.task_control.tree_revision == 0 {
        project.task_control.tree_revision = 1;
    }
    migrate_legacy_parent_session(project)?;
    if !project
        .execution_session
        .as_ref()
        .is_some_and(|session| session.active)
    {
        refresh_serial_takeover_capability(project);
    }
    Ok(())
}

fn hydrate_task_contracts(
    tasks: &mut [crate::project::Subtask],
    parent_id: Option<&str>,
    depth: u32,
    has_dynamic_tasks: &mut bool,
) -> Result<(), String> {
    if depth > crate::task_tree::MAX_TASK_TREE_DEPTH {
        return Err(format!(
            "任务树迁移深度超过安全上限 {}",
            crate::task_tree::MAX_TASK_TREE_DEPTH
        ));
    }
    for task in tasks {
        if task.contract_snapshot.is_none() {
            task.contract_snapshot = Some(crate::task_contract::compile_subtask(
                task, parent_id, depth,
            ));
        }
        if !task.child_tasks.is_empty() {
            *has_dynamic_tasks = true;
        }
        let task_id = task.id.clone();
        hydrate_task_contracts(
            &mut task.child_tasks,
            Some(&task_id),
            depth.saturating_add(1),
            has_dynamic_tasks,
        )?;
    }
    Ok(())
}

fn migrate_legacy_parent_session(project: &mut Project) -> Result<(), String> {
    let Some(session) = project
        .execution_session
        .as_ref()
        .filter(|session| session.active && !session.subtask_id.is_empty())
        .cloned()
    else {
        return Ok(());
    };
    let task = crate::task_tree::find_task(project, &session.subtask_id)?
        .ok_or_else(|| format!("旧执行会话指向不存在的任务：{}", session.subtask_id))?
        .clone();
    if task.child_tasks.is_empty() {
        return Ok(());
    }
    if crate::task_tree::is_terminal(&task.status) {
        if let Some(current) = project.execution_session.as_mut() {
            current.active = false;
        }
        append_migration_event(
            project,
            &task.id,
            "已关闭指向已完成父任务的陈旧执行会话，历史任务结果保持不变",
        );
        return Ok(());
    }

    let execution_started = task.status == crate::project::SubtaskStatus::Executing
        || session.status.eq_ignore_ascii_case("executing");
    if execution_started {
        if let Some(current) = project.execution_session.as_mut() {
            current.active = false;
            current.status = "session_lost".to_string();
            current.failure_message =
                "旧执行会话指向已有子任务的父节点，需要人工确认执行边界".to_string();
        }
        if let Some(state) = project.workflow_state.autopilot_state.as_mut() {
            state.run_status = crate::project::AutopilotRunStatus::ErrorStopped;
            state.recovery_action = crate::project::AutopilotRecoveryAction::WaitHumanDecision;
            state.error_message = "活动父任务会话无法安全自动迁移".to_string();
            state.last_action = "等待人工确认动态任务树执行边界".to_string();
            state.last_action_at = chrono::Utc::now().to_rfc3339();
        }
        append_migration_event(
            project,
            &task.id,
            "活动父任务会话已停止，等待人工确认动态任务树执行边界",
        );
        return Ok(());
    }

    if let Some(current) = project.execution_session.as_mut() {
        current.active = false;
    }
    let address = crate::task_tree::first_available_descendant_leaf(project, &task.id)?
        .ok_or_else(|| format!("父任务 {} 没有可安全迁移的叶子任务", task.id))?;
    let leaf = crate::task_tree::find_task(project, &address.task_id)?
        .ok_or_else(|| format!("迁移目标叶子不存在：{}", address.task_id))?;
    let leaf_title = leaf.title.clone();
    let contract_fingerprint = leaf
        .contract_snapshot
        .as_ref()
        .map(|contract| contract.fingerprint.clone())
        .unwrap_or_default();
    if let Some(current) = project.execution_session.as_mut() {
        current.active = false;
        current.status = "not_started".to_string();
        current.failure_message.clear();
        current.milestone_id = address.milestone_id.clone();
        current.mid_stage_id = address.mid_stage_id.clone();
        current.subtask_id = address.task_id.clone();
        current.subtask_title = leaf_title;
        current.task_path = address.task_path();
        current.parent_task_id = address
            .ancestor_task_ids
            .last()
            .cloned()
            .unwrap_or_default();
        current.top_level_task_id = address.top_level_task_id.clone();
        current.task_tree_revision = project.task_control.tree_revision;
        current.contract_fingerprint = contract_fingerprint;
        current.node_depth = address.depth;
    }
    append_migration_event(
        project,
        &address.task_id,
        "未开始的旧父任务会话已迁移到首个可执行叶子",
    );
    Ok(())
}

fn append_migration_event(project: &mut Project, task_id: &str, text: &str) {
    project
        .execution_history
        .push(crate::project::ExecutionHistoryEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            level: "info".to_string(),
            event_type: crate::project::ExecutionEventType::SystemAdvance,
            source: crate::project::OperationSource::System,
            text: text.to_string(),
            milestone_id: (!project.current_milestone_id.is_empty())
                .then(|| project.current_milestone_id.clone()),
            mid_stage_id: (!project.current_mid_stage_id.is_empty())
                .then(|| project.current_mid_stage_id.clone()),
            subtask_id: Some(task_id.to_string()),
            criterion_index: None,
            decision_id: None,
            action_id: None,
            validator_id: None,
            model_call_id: None,
        });
}

pub fn mode_label(mode: TaskControlMode) -> &'static str {
    match mode {
        TaskControlMode::Legacy => "旧流水线",
        TaskControlMode::Shadow => "影子控制器",
        TaskControlMode::SerialTakeover => "串行接管",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_parent_session(
        parent_status: crate::project::SubtaskStatus,
        session_status: &str,
    ) -> Project {
        let mut project = Project::new("legacy-dynamic-tree");
        project.current_milestone_id = "m".to_string();
        let parent = crate::project::Subtask {
            id: "parent".to_string(),
            title: "Parent".to_string(),
            status: parent_status,
            child_tasks: vec![
                crate::project::Subtask {
                    id: "leaf-one".to_string(),
                    title: "Leaf one".to_string(),
                    ..Default::default()
                },
                crate::project::Subtask {
                    id: "leaf-two".to_string(),
                    title: "Leaf two".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        project.milestones.push(crate::project::Milestone {
            id: "m".to_string(),
            version: "v0.1".to_string(),
            title: "M".to_string(),
            description: String::new(),
            tech_stack: String::new(),
            status: crate::project::MilestoneStatus::InProgress,
            mode: crate::project::StageMode::Quick,
            mid_stages: Vec::new(),
            subtasks: vec![parent],
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
        });
        project.execution_session = Some(crate::project::ExecutionSession {
            active: true,
            milestone_id: "m".to_string(),
            subtask_id: "parent".to_string(),
            status: session_status.to_string(),
            ..Default::default()
        });
        project
    }

    #[test]
    fn legacy_defaults_are_safe() {
        let state = TaskControlState::default();
        assert_eq!(state.mode, TaskControlMode::Legacy);
        assert_eq!(state.control_source, "legacy_workflow");
    }

    #[test]
    fn old_project_state_is_hydrated_without_changing_mode() {
        let mut project = Project::new("legacy");
        project.task_control = TaskControlState::default();
        project.task_control.algorithm_version.clear();
        project.task_control.snapshot_version.clear();
        project.task_control.control_source.clear();
        hydrate_project(&mut project).unwrap();
        assert_eq!(project.task_control.mode, TaskControlMode::Legacy);
        assert!(!project.task_control.algorithm_version.is_empty());
    }

    #[test]
    fn phase1_runtime_contract_new_projects_default_to_serial_without_migrating_legacy() {
        assert_eq!(
            Project::new("new").task_control.mode,
            TaskControlMode::SerialTakeover
        );
        assert_eq!(
            Project::new_half("half", "/tmp/half").task_control.mode,
            TaskControlMode::SerialTakeover
        );
        assert_eq!(
            Project::new("capable")
                .task_control
                .takeover_capability_status,
            TakeoverCapabilityStatus::Ready
        );
        assert_eq!(
            Project::new("source").task_control.control_source,
            "task_controller"
        );

        let mut value = serde_json::to_value(Project::new("old-json")).unwrap();
        value.as_object_mut().unwrap().remove("task_control");
        let mut restored: Project = serde_json::from_value(value).unwrap();
        hydrate_project(&mut restored).unwrap();
        assert_eq!(restored.task_control.mode, TaskControlMode::Legacy);
    }

    #[test]
    fn hydration_preserves_an_existing_control_mode() {
        let mut project = Project::new("existing-shadow");
        project.task_control.mode = TaskControlMode::Shadow;
        project.task_control.control_source = "shadow_controller".to_string();
        hydrate_project(&mut project).unwrap();
        assert_eq!(project.task_control.mode, TaskControlMode::Shadow);
        project.task_control.mode = TaskControlMode::SerialTakeover;
        hydrate_project(&mut project).unwrap();
        assert_eq!(project.task_control.mode, TaskControlMode::SerialTakeover);
    }

    #[test]
    fn pending_parent_session_migrates_to_first_leaf_and_hydrates_contracts() {
        let mut project =
            project_with_parent_session(crate::project::SubtaskStatus::Pending, "not_started");
        hydrate_project(&mut project).unwrap();

        let parent = &project.milestones[0].subtasks[0];
        assert!(parent.contract_snapshot.is_some());
        assert!(parent
            .child_tasks
            .iter()
            .all(|task| task.contract_snapshot.is_some()));
        assert_eq!(project.task_control.tree_revision, 1);
        let session = project.execution_session.as_ref().unwrap();
        assert!(!session.active);
        assert_eq!(session.subtask_id, "leaf-one");
        assert_eq!(session.task_path, vec!["parent", "leaf-one"]);
        assert_eq!(session.parent_task_id, "parent");
        assert_eq!(session.task_tree_revision, 1);
    }

    #[test]
    fn executing_parent_session_stops_at_human_boundary_without_resetting_recovery() {
        let mut project =
            project_with_parent_session(crate::project::SubtaskStatus::Executing, "executing");
        project.workflow_state.autopilot_active = true;
        project.workflow_state.autopilot_state = Some(crate::project::AutopilotState {
            active: true,
            run_status: crate::project::AutopilotRunStatus::Running,
            ..Default::default()
        });
        project.workflow_state.recovery_state = Some(crate::project::RecoveryState {
            attempt: 2,
            ..Default::default()
        });

        hydrate_project(&mut project).unwrap();

        let session = project.execution_session.as_ref().unwrap();
        assert!(!session.active);
        assert_eq!(session.status, "session_lost");
        let autopilot = project.workflow_state.autopilot_state.as_ref().unwrap();
        assert_eq!(
            autopilot.run_status,
            crate::project::AutopilotRunStatus::ErrorStopped
        );
        assert_eq!(
            autopilot.recovery_action,
            crate::project::AutopilotRecoveryAction::WaitHumanDecision
        );
        assert_eq!(
            project
                .workflow_state
                .recovery_state
                .as_ref()
                .unwrap()
                .attempt,
            2
        );
    }

    #[test]
    fn completed_parent_history_is_not_reopened_during_migration() {
        let mut project =
            project_with_parent_session(crate::project::SubtaskStatus::Passed, "executing");
        hydrate_project(&mut project).unwrap();

        assert_eq!(
            project.milestones[0].subtasks[0].status,
            crate::project::SubtaskStatus::Passed
        );
        assert!(!project.execution_session.as_ref().unwrap().active);
    }
}
