use crate::project;
use crate::AppState;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PipelineStatus {
    Idle,
    Running,
    Paused,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskStatusItem {
    pub subtask_id: String,
    pub title: String,
    pub status: String,
    pub test_result: Option<project::TestResult>,
    pub retry_count: u32,
}

/// 执行日志条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// ISO 8601 时间戳
    pub timestamp: String,
    /// 日志级别：info / success / error / pause
    pub level: String,
    /// 日志文本
    pub text: String,
    /// 结构化来源，例如 pipeline/stdout/stderr。
    #[serde(default)]
    pub source: String,
    /// Provider 事件关联 ID；旧日志缺失时保持 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

/// 日志历史上限
const MAX_LOG_HISTORY: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineState {
    /// 单次后台执行的唯一标识；用于拒绝旧任务回写
    #[serde(default)]
    pub execution_id: String,
    pub mid_stage_id: String,
    pub status: PipelineStatus,
    pub current_subtask_index: usize,
    pub total_subtasks: usize,
    pub subtask_statuses: Vec<SubtaskStatusItem>,
    pub current_log: String,
    pub last_error: Option<String>,
    /// 当前正在运行的子进程 PID，用于 stop_execution 快速终止
    #[serde(default)]
    pub child_pid: Option<u32>,
    // === V1 人工执行字段 ===
    /// 项目名称
    #[serde(default)]
    pub project_name: String,
    /// 大阶段 ID
    #[serde(default)]
    pub milestone_id: String,
    /// 计划修订号（验证计划未被修改）
    #[serde(default)]
    pub plan_revision: u64,
    /// 当前执行的小阶段 ID
    #[serde(default)]
    pub current_subtask_id: String,
    /// 等待用户确认执行结果
    #[serde(default)]
    pub awaiting_confirmation: bool,
    /// 累积日志历史（最新条目在末尾）
    #[serde(default)]
    pub log_history: Vec<LogEntry>,
}

/// A running pipeline is owned only when it carries a non-empty execution identity.
/// The identity is the backend claim shared with the persisted execution session.
pub(crate) fn pipeline_owner_matches(pipeline: Option<&PipelineState>, execution_id: &str) -> bool {
    !execution_id.is_empty()
        && pipeline.is_some_and(|state| {
            state.status == PipelineStatus::Running && state.execution_id == execution_id
        })
}

/// 追加日志条目到 PipelineState，同时更新 current_log 并限制历史上限
pub(crate) fn append_log(state: &mut PipelineState, level: &str, text: String) {
    append_log_with_context(state, level, "pipeline", None, text);
}

fn append_log_with_context(
    state: &mut PipelineState,
    level: &str,
    source: &str,
    correlation_id: Option<String>,
    text: String,
) {
    let entry = LogEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        level: level.to_string(),
        text: text.clone(),
        source: source.to_string(),
        correlation_id,
    };
    state.log_history.push(entry);
    // 保持最近 MAX_LOG_HISTORY 条
    if state.log_history.len() > MAX_LOG_HISTORY {
        let excess = state.log_history.len() - MAX_LOG_HISTORY;
        state.log_history.drain(0..excess);
    }
    state.current_log = text;
}

/// 运行期实时日志：与 append_log 相同容量上限，供执行器流式写入
pub(crate) fn append_runtime_log(state: &mut PipelineState, level: &str, text: String) {
    append_log_with_context(state, level, "runtime", None, text);
}

pub(crate) fn append_runtime_log_with_context(
    state: &mut PipelineState,
    level: &str,
    source: &str,
    correlation_id: Option<String>,
    text: String,
) {
    append_log_with_context(state, level, source, correlation_id, text);
}

/// Thought/debug 只占一个有界 live slot，不进入普通 200 条日志历史。
pub(crate) fn set_runtime_debug_log(
    state: &mut PipelineState,
    source: &str,
    correlation_id: Option<String>,
    text: String,
) {
    state.current_log = serde_json::json!({
        "kind": "runtime_log",
        "level": "debug",
        "source": source,
        "correlation_id": correlation_id,
        "text": text,
    })
    .to_string();
}

/// 向调用方持有的项目事实追加执行历史；持久化由调用方在事务边界统一完成。
pub(crate) fn write_execution_history(
    proj: &mut project::Project,
    level: &str,
    event_type: project::ExecutionEventType,
    text: String,
    milestone_id: Option<&str>,
    mid_stage_id: Option<&str>,
    subtask_id: Option<&str>,
) {
    let source = match &event_type {
        project::ExecutionEventType::UserExecute
        | project::ExecutionEventType::UserConfirm
        | project::ExecutionEventType::UserReject
        | project::ExecutionEventType::UserInStop
        | project::ExecutionEventType::UserEdStop
        | project::ExecutionEventType::UserContinue
        | project::ExecutionEventType::UserAdjust
        | project::ExecutionEventType::UserRollback
        | project::ExecutionEventType::HumanVerificationAccepted
        | project::ExecutionEventType::TaskSkipped => project::OperationSource::User,
        project::ExecutionEventType::RecoveryStarted
        | project::ExecutionEventType::ErrorDiagnosed
        | project::ExecutionEventType::RepairAttemptStarted
        | project::ExecutionEventType::RepairAttemptCompleted
        | project::ExecutionEventType::RetestCompleted
        | project::ExecutionEventType::EvidenceRebuildStarted
        | project::ExecutionEventType::EvidenceRebuildCompleted
        | project::ExecutionEventType::EvidenceStillInsufficient
        | project::ExecutionEventType::ReplanStarted
        | project::ExecutionEventType::ReplanCompleted
        | project::ExecutionEventType::ReplanExecutionStarted
        | project::ExecutionEventType::RecoveryWarning
        | project::ExecutionEventType::RecoveryStalled
        | project::ExecutionEventType::RecoverySucceeded
        | project::ExecutionEventType::RecoveryExhausted
        | project::ExecutionEventType::ReviewRequested
        | project::ExecutionEventType::ProtocolNormalized
        | project::ExecutionEventType::ProtocolRepairAttempted
        | project::ExecutionEventType::ValidationRetryScheduled
        | project::ExecutionEventType::ValidationRecoverySucceeded => {
            project::OperationSource::Recovery
        }
        _ => project::OperationSource::System,
    };
    write_execution_history_with_source(
        proj,
        level,
        event_type,
        source,
        text,
        milestone_id,
        mid_stage_id,
        subtask_id,
    );
}

/// 显式记录动作来源。后台动作和跨异步边界的生命周期事件必须使用此入口。
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_execution_history_with_source(
    proj: &mut project::Project,
    level: &str,
    event_type: project::ExecutionEventType,
    source: project::OperationSource,
    text: String,
    milestone_id: Option<&str>,
    mid_stage_id: Option<&str>,
    subtask_id: Option<&str>,
) {
    let entry = project::ExecutionHistoryEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        level: level.to_string(),
        event_type,
        source,
        text,
        milestone_id: milestone_id.map(|s| s.to_string()),
        mid_stage_id: mid_stage_id
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        subtask_id: subtask_id.map(|s| s.to_string()),
        criterion_index: None,
        decision_id: None,
        action_id: None,
        validator_id: None,
        model_call_id: None,
        control_lock_owner_process_start_id: None,
        control_lock_heartbeat_at: None,
        control_lock_clear_reason: None,
        control_lock_post_task_state: None,
    };
    proj.execution_history.push(entry);
    // 限制历史上限
    if proj.execution_history.len() > project::MAX_EXECUTION_HISTORY {
        let excess = proj.execution_history.len() - project::MAX_EXECUTION_HISTORY;
        proj.execution_history.drain(0..excess);
    }
}

fn session_has_in_process_owner(status: &str) -> bool {
    matches!(
        status,
        "executing" | "recovering" | "replanning" | "replan_ready" | "confirming" | "rejecting"
    )
}

/// Persist the fact that application shutdown intentionally ended in-process
/// execution. The session remains active so startup can reconcile it; child
/// PID evidence is handled separately by the snapshot layer.
pub(crate) fn record_intentional_exit(
    proj: &mut project::Project,
    pipeline: Option<&PipelineState>,
) -> bool {
    let Some(session) = proj.execution_session.as_ref() else {
        return false;
    };
    if !session.active || !session_has_in_process_owner(session.status.as_str()) {
        return false;
    }
    let pipeline_matches = pipeline.is_some_and(|state| {
        state.project_name == proj.name
            && state.execution_id == session.execution_id
            && state.status == PipelineStatus::Running
    });
    if pipeline.is_some() && !pipeline_matches {
        return false;
    }
    let already_recorded = proj.execution_history.iter().rev().take(4).any(|entry| {
        entry.subtask_id.as_deref() == Some(session.subtask_id.as_str())
            && entry.text.starts_with("应用正常退出：执行会话保留")
    });
    if already_recorded {
        return false;
    }
    let execution_id = session.execution_id.clone();
    let milestone_id = session.milestone_id.clone();
    let mid_stage_id = session.mid_stage_id.clone();
    let subtask_id = session.subtask_id.clone();
    let child_pid = pipeline.and_then(|state| state.child_pid);
    write_execution_history(
        proj,
        "info",
        project::ExecutionEventType::SystemAdvance,
        format!(
            "应用正常退出：执行会话保留，内置任务随应用结束；重开时对账 execution_id={} child_pid={}",
            execution_id,
            child_pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "未知".to_string())
        ),
        Some(&milestone_id),
        Some(&mid_stage_id),
        Some(&subtask_id),
    );
    true
}

fn execution_request_audit(
    source: project::OperationSource,
    position: usize,
    total: usize,
    title: &str,
    provider: &str,
) -> (project::ExecutionEventType, String) {
    match source {
        project::OperationSource::User => (
            project::ExecutionEventType::UserExecute,
            format!(
                "用户点击执行 ({}/{}): {}（{}）",
                position, total, title, provider
            ),
        ),
        project::OperationSource::Autopilot => (
            project::ExecutionEventType::AutopilotExecute,
            format!(
                "自动驾驶触发执行 ({}/{}): {}（{}）",
                position, total, title, provider
            ),
        ),
        _ => (
            project::ExecutionEventType::SystemAdvance,
            format!(
                "系统触发执行 ({}/{}): {}（{}）",
                position, total, title, provider
            ),
        ),
    }
}

fn confirmation_audit(
    source: project::OperationSource,
    title: &str,
) -> (project::ExecutionEventType, String) {
    match source {
        project::OperationSource::User => (
            project::ExecutionEventType::UserConfirm,
            format!("用户确认通过：{}", title),
        ),
        project::OperationSource::Autopilot => (
            project::ExecutionEventType::AutopilotConfirm,
            format!("自动驾驶确认通过：{}", title),
        ),
        _ => (
            project::ExecutionEventType::SystemAdvance,
            format!("系统确认通过：{}", title),
        ),
    }
}

fn verification_stage_description(stage: &project::VerificationStage) -> &'static str {
    match stage {
        project::VerificationStage::NotStarted => "等待验证",
        project::VerificationStage::AutomatedTests => "正在运行自动化测试",
        project::VerificationStage::PreparingEvidence => "正在准备验收证据",
        project::VerificationStage::RequestingReview => "正在请求 AI 审查",
        project::VerificationStage::ParsingReview => "正在解析审查结果",
        project::VerificationStage::DeterministicNormalization => "正在确定性归一化审查协议",
        project::VerificationStage::ProtocolRepair => "正在按 Schema 修复审查协议",
        project::VerificationStage::ReviewRetry => "正在重新请求 AI 审查",
        project::VerificationStage::TargetedEvidence => "正在定向补充验收证据",
        project::VerificationStage::Completed => "验证已完成",
    }
}

/// 按 execution_id 写入验证进度；旧执行或已结束会话不能覆盖当前项目状态。
pub(crate) fn persist_verification_progress(
    project_name: &str,
    execution_id: &str,
    stage: project::VerificationStage,
) -> Result<bool, String> {
    let mut proj = crate::load_project(project_name)?;
    let Some(session) = proj.execution_session.as_mut().filter(|session| {
        session.active
            && session.execution_id == execution_id
            && matches!(session.status.as_str(), "executing" | "recovering")
    }) else {
        return Ok(false);
    };
    let now = chrono::Utc::now().to_rfc3339();
    if session.verification_stage != stage {
        session.verification_stage = stage.clone();
        session.state_entered_at = now.clone();
    }
    if proj.workflow_state.autopilot_active {
        if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
            let retry = proj
                .workflow_state
                .recovery_state
                .as_ref()
                .filter(|recovery| crate::recovery::is_review_validation_recovery(recovery))
                .map(|recovery| {
                    format!(
                        "（验证重试 {}/{}）",
                        recovery.validation_retry_count, recovery.max_validation_retries
                    )
                })
                .unwrap_or_default();
            autopilot.last_action = format!("{}{}", verification_stage_description(&stage), retry);
            autopilot.last_action_at = now.clone();
            autopilot.heartbeat_at = now;
            autopilot.job_owner = project::AutopilotJobOwner::BackendRuntime;
        }
    }
    crate::save_project(&proj)?;
    Ok(true)
}

/// Acquire the pipeline lock for a new execution while rejecting an existing
/// running session. Keeping this check and the subsequent state reservation
/// under one guard prevents two callers from launching the same subtask.
async fn acquire_pipeline_start<'a>(
    pipeline_state: &'a std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
) -> Result<tokio::sync::MutexGuard<'a, Option<PipelineState>>, String> {
    let mut guard = pipeline_state.lock().await;
    if let Some(pipeline) = guard.as_mut() {
        if pipeline.status != PipelineStatus::Running {
            return Ok(guard);
        }
        let execution_id = pipeline.execution_id.clone();
        if pipeline_owner_matches(Some(pipeline), &execution_id) {
            return Err("已有小阶段正在执行，请等待当前任务结束。".to_string());
        }
        pipeline.status = PipelineStatus::Failed;
        pipeline.last_error = Some(
            "流水线处于 Running 但缺少有效 execution owner，已收敛为失败并允许重建。".to_string(),
        );
        pipeline.awaiting_confirmation = false;
    }
    Ok(guard)
}

#[tauri::command]
pub(crate) async fn get_execution_status(
    state: tauri::State<'_, AppState>,
) -> Result<Option<PipelineState>, String> {
    let guard = state.pipeline_state.lock().await;
    Ok(guard.clone())
}

// ===================================================================
// V1 人工执行命令：单小阶段执行 → 人工确认
// ===================================================================

/// V1 执行当前小阶段（从磁盘读取已批准计划，一次只执行一个）。
///
/// # 返回值说明
///
/// 本命令保留为返回 `PipelineState` 的兼容入口；前端主路径使用
/// `execute_current_subtask_runtime`，一次取得 Project、Pipeline 与恢复展示。
/// 保留该入口的原因：
///
/// 1. **两阶段保存模式**：执行过程分为两个持久化点：
///    - 阶段一（执行前）：保存 `SubtaskStatus::Executing` + `execution_session(status="executing")`
///    - 阶段二（执行后）：保存 `SubtaskStatus::AwaitingConfirmation` + `execution_session(status="awaiting_confirmation")`
///    两次保存之间执行器在运行，不适合每次都做 save+reload 往返。
///
/// 2. **前端需要实时状态流**：前端执行面板依赖 `PipelineState` 中的
///    `subtask_statuses`、`current_log`、`awaiting_confirmation` 等实时字段
///    来渲染进度条和日志流。`Project` 不包含这些运行时字段。
///
/// 3. **Project 同步由前端轮询完成**：前端执行轮询（`get_execution_status`）
///    在检测到 `Completed`/`AwaitingConfirmation` 时调用 `get_project` 从磁盘
///    刷新完整 `Project`，保持业务状态同步。
///
/// # 前端契约
///
/// - 兼容调用方应立即使用返回的 `PipelineState` 更新 `executionStatus`
/// - 新调用方必须消费统一运行时变更结果，并在终态读取统一运行时快照
#[tauri::command]
pub(crate) async fn execute_current_subtask(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<PipelineState, String> {
    execute_current_subtask_with_pipeline(state.pipeline_state.clone(), project_name).await
}

pub(crate) async fn execute_current_subtask_with_pipeline(
    pipeline_state: std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
    project_name: String,
) -> Result<PipelineState, String> {
    execute_current_subtask_with_source(
        pipeline_state,
        project_name,
        project::OperationSource::User,
    )
    .await
}

pub(crate) async fn execute_current_subtask_with_source(
    pipeline_state: std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
    project_name: String,
    operation_source: project::OperationSource,
) -> Result<PipelineState, String> {
    execute_task_with_source(pipeline_state, project_name, None, operation_source).await
}

pub(crate) async fn execute_task_with_source(
    pipeline_state: std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
    project_name: String,
    requested_task_id: Option<String>,
    operation_source: project::OperationSource,
) -> Result<PipelineState, String> {
    // 以全局流水线锁串行化“校验 + Running 落盘 + 内存状态建立”，阻止重复启动。
    let mut pipeline_guard = acquire_pipeline_start(&pipeline_state).await?;

    let mut proj = crate::load_project(&project_name)?;
    let project_path = proj.project_path.clone();

    if let Some(session) = proj
        .execution_session
        .as_ref()
        .filter(|session| session.active)
    {
        let message = match session.parsed_status() {
            project::ExecutionSessionStatus::AwaitingConfirmation => {
                "当前任务已有待确认变更，请先确认、驳回或恢复基线。"
            }
            project::ExecutionSessionStatus::QualityBlocked => {
                "当前任务处于质量阻断状态，请先完成恢复或人工核验。"
            }
            _ => "项目已有活跃执行会话，请先同步或处理恢复状态。",
        };
        return Err(message.to_string());
    }

    let milestone_id = proj.current_milestone_id.clone();
    let mid_stage_id = proj.current_mid_stage_id.clone();
    let scope = crate::plan_scope::PlanScope::resolve(&proj)?;

    // Verify plan is approved
    if scope.plan_approved_at(&proj).is_none() || scope.plan_revision(&proj) == 0 {
        return Err("执行计划尚未批准，请先在 Console 中批准执行计划。".to_string());
    }
    crate::plan_contract::validate_subtasks(scope.subtasks(&proj))
        .map_err(|error| format!("执行计划契约无效：{}", error))?;

    // Verify Git workspace is ready
    let ws = get_execution_workspace_status_inner(&project_path)?;
    if !ws.ready {
        return Err(ws.status_message);
    }
    crate::plan_contract::validate_subtasks_in_project(scope.subtasks(&proj), &project_path)
        .map_err(|error| format!("执行计划契约无效：{}", error))?;

    let prepared_engine = crate::engine::prepare_engine(&proj.execution_profile).await?;
    let engine_health = &prepared_engine.health;
    if engine_health.status.blocks_execution() {
        return Err(format!("执行引擎不可用：{}", engine_health.message));
    }
    let execution_profile = proj.execution_profile.clone();
    let app_settings = prepared_engine.settings();
    let engine_settings_revision = app_settings.revision;
    let engine_source_revision = if execution_profile.runtime == project::ExecutionRuntime::BuiltIn
    {
        engine_health.source_revision.clone().unwrap_or_default()
    } else {
        String::new()
    };
    let engine_api_backend = if execution_profile.runtime == project::ExecutionRuntime::BuiltIn {
        app_settings
            .built_in_grok_build
            .api_backend
            .as_str()
            .to_string()
    } else {
        String::new()
    };
    let engine_model = if execution_profile.runtime == project::ExecutionRuntime::BuiltIn {
        app_settings.built_in_grok_build.model.clone()
    } else {
        String::new()
    };
    let endpoint_fingerprint = if execution_profile.runtime == project::ExecutionRuntime::BuiltIn {
        crate::settings::endpoint_fingerprint(&app_settings.built_in_grok_build.api_base_url)
    } else {
        String::new()
    };
    let engine_executable_path = engine_health.executable_path.clone().unwrap_or_default();

    let selected_address = match requested_task_id.as_deref() {
        Some(task_id) => crate::task_tree::locate_task(&proj, task_id)?
            .ok_or_else(|| format!("任务节点不存在：{}", task_id))?,
        None => crate::task_tree::select_current_leaf(&proj)?.ok_or_else(|| {
            "没有可执行的叶子任务。所有任务可能已完成或依赖尚未满足。".to_string()
        })?,
    };
    if selected_address.milestone_id != milestone_id
        || selected_address.mid_stage_id != mid_stage_id
    {
        return Err("目标叶子任务不属于当前计划目标".to_string());
    }
    if !selected_address.dependencies_satisfied {
        return Err(format!(
            "任务 {} 的依赖尚未满足，不能执行",
            selected_address.task_id
        ));
    }
    let leaves = crate::task_tree::leaf_addresses_in_scope(&proj, &milestone_id, &mid_stage_id)?;
    let next_idx = leaves
        .iter()
        .position(|address| address.task_id == selected_address.task_id)
        .ok_or_else(|| "当前任务不是可执行叶子节点".to_string())?;
    let subtask = crate::task_tree::find_task(&proj, &selected_address.task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", selected_address.task_id))?;
    if !subtask.child_tasks.is_empty() {
        return Err("父任务不能直接执行，必须选择最深层叶子节点".to_string());
    }
    if subtask.status != project::SubtaskStatus::Pending {
        return Err(format!(
            "任务 {} 当前状态为 {:?}，不能开始执行",
            subtask.id, subtask.status
        ));
    }
    let authorized_paths =
        crate::plan_contract::validate_subtask(subtask, &format!("第 {} 个小阶段", next_idx + 1))?;
    let subtask_id = subtask.id.clone();
    let subtask_title = subtask.title.clone();
    let subtask_goal = if subtask.goal.is_empty() {
        subtask.title.clone()
    } else {
        subtask.goal.clone()
    };
    let acceptance_criteria = subtask.acceptance_criteria.clone();
    let learning = crate::recovery_learning::render_matching(&proj, subtask, None, None);
    let approved_prompt =
        crate::plan_compiler::compile_execution_prompt_with_learning(subtask, &learning);

    let total = leaves.len();
    let plan_revision = scope.plan_revision(&proj);
    let workload = crate::workload_policy::current_profile(&proj)?;
    let compiled_contract = crate::task_compiler::compile(
        subtask,
        selected_address
            .ancestor_task_ids
            .last()
            .map(String::as_str),
        selected_address.depth,
        workload,
    )
    .contract;
    let task_budget = compiled_contract.budget.clone();
    let execution_id = format!(
        "execution-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let subtask_statuses = leaves
        .iter()
        .filter_map(|address| {
            crate::task_tree::find_task(&proj, &address.task_id)
                .ok()
                .flatten()
                .map(|task| (address, task))
        })
        .map(|(address, task)| SubtaskStatusItem {
            subtask_id: task.id.clone(),
            title: task.title.clone(),
            status: if address.task_id == subtask_id {
                "executing".to_string()
            } else if crate::task_tree::is_terminal(&task.status) {
                "completed".to_string()
            } else {
                "waiting".to_string()
            },
            test_result: None,
            retry_count: 0,
        })
        .collect();
    let now = chrono::Utc::now().to_rfc3339();

    // 执行事实和启动历史使用同一个项目对象并在同一事务边界保存。
    let (request_event, request_text) = execution_request_audit(
        operation_source,
        next_idx + 1,
        total,
        &subtask_title,
        execution_profile.provider.display_name(),
    );
    write_execution_history_with_source(
        &mut proj,
        "info",
        request_event,
        operation_source,
        request_text,
        Some(&milestone_id),
        Some(&mid_stage_id),
        Some(&subtask_id),
    );

    // === 阶段一关键修复：执行前先持久化 "Executing" 到磁盘 ===
    // 这样刷新后前端能从磁盘 Project 中知道当前正在执行，
    // 而不是错误地显示"点击执行"。
    {
        let st = crate::task_tree::find_task_mut(&mut proj, &subtask_id)?
            .ok_or_else(|| format!("任务节点不存在：{}", subtask_id))?;
        if !st.child_tasks.is_empty() || st.status != project::SubtaskStatus::Pending {
            return Err("叶子任务状态在执行认领前发生变化".to_string());
        }
        st.status = project::SubtaskStatus::Executing;
        // 读取当前 Git HEAD 作为执行基线；失败时不得启动后台执行。
        let base_commit_output = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&project_path)
            .output()
            .map_err(|error| format!("读取执行基线失败：{}", error))?;
        if !base_commit_output.status.success() {
            return Err(format!(
                "读取执行基线失败：{}",
                String::from_utf8_lossy(&base_commit_output.stderr).trim()
            ));
        }
        let base_commit = String::from_utf8(base_commit_output.stdout)
            .map_err(|error| format!("执行基线不是有效 UTF-8：{}", error))?
            .trim()
            .to_string();
        proj.execution_session = Some(project::ExecutionSession {
            execution_id: execution_id.clone(),
            active: true,
            milestone_id: milestone_id.clone(),
            mid_stage_id: mid_stage_id.clone(),
            subtask_id: subtask_id.clone(),
            subtask_title: subtask_title.clone(),
            status: "executing".to_string(),
            base_commit,
            failure_message: String::new(),
            verification_stage: project::VerificationStage::NotStarted,
            confirmation_transaction_id: String::new(),
            confirmation_phase: project::ConfirmationPhase::NotStarted,
            confirmation_candidate_tag: String::new(),
            confirmation_commit: String::new(),
            confirmation_failure_kind: None,
            started_at: now.clone(),
            state_entered_at: now.clone(),
            plan_revision,
            subtask_index: next_idx,
            total_subtasks: total,
            task_path: selected_address.task_path(),
            parent_task_id: selected_address
                .ancestor_task_ids
                .last()
                .cloned()
                .unwrap_or_default(),
            top_level_task_id: selected_address.top_level_task_id.clone(),
            task_tree_revision: proj.task_control.tree_revision,
            contract_fingerprint: compiled_contract.fingerprint.clone(),
            node_depth: selected_address.depth,
            engine_snapshot: execution_profile.clone(),
            engine_settings_revision,
            engine_source_revision,
            engine_api_backend,
            engine_model,
            endpoint_fingerprint,
            engine_executable_path,
            human_review_cadence: proj.human_review_cadence,
        });
    }

    write_execution_history_with_source(
        &mut proj,
        "info",
        project::ExecutionEventType::SubtaskExecuting,
        operation_source,
        format!("▶ 开始执行 ({}/{})：{}", next_idx + 1, total, subtask_title),
        Some(&milestone_id),
        Some(&mid_stage_id),
        Some(&subtask_id),
    );
    crate::snapshot::clear_startup_process_observation(&project_name)?;
    crate::save_project(&proj)?;

    // Initialize pipeline state, then return immediately after scheduling the background task.
    let initial_state = PipelineState {
        execution_id: execution_id.clone(),
        mid_stage_id: mid_stage_id.clone(),
        status: PipelineStatus::Running,
        current_subtask_index: next_idx,
        total_subtasks: total,
        subtask_statuses,
        current_log: format!("▶ 执行中 ({}/{})：{}", next_idx + 1, total, subtask_title),
        last_error: None,
        child_pid: None,
        project_name: project_name.clone(),
        milestone_id: milestone_id.clone(),
        plan_revision,
        current_subtask_id: subtask_id.clone(),
        awaiting_confirmation: false,
        log_history: vec![LogEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            level: "info".to_string(),
            text: format!("▶ 执行中 ({}/{})：{}", next_idx + 1, total, subtask_title),
            source: "pipeline".to_string(),
            correlation_id: None,
        }],
    };
    *pipeline_guard = Some(initial_state.clone());
    drop(pipeline_guard);

    let background_pipeline_state = pipeline_state.clone();
    let failure_project_name = project_name.clone();
    let failure_milestone_id = milestone_id.clone();
    let failure_mid_stage_id = mid_stage_id.clone();
    let failure_subtask_id = subtask_id.clone();
    let failure_subtask_title = subtask_title.clone();
    let failure_execution_id = execution_id.clone();
    // Heap-allocate the background execution future: builtin-grok debug futures are large
    // enough to overflow the default Tokio worker stack when polled on the stack.
    tauri::async_runtime::spawn(async move {
        let result = Box::pin(execute_current_subtask_background(
            project_name,
            project_path,
            milestone_id,
            mid_stage_id,
            subtask_id,
            subtask_title,
            subtask_goal,
            acceptance_criteria,
            approved_prompt,
            authorized_paths,
            next_idx,
            total,
            execution_id,
            execution_profile,
            task_budget,
            prepared_engine,
            background_pipeline_state.clone(),
            operation_source,
        ))
        .await;
        if let Err(error) = result {
            if let Err(persist_error) = finalize_background_execution_failure(
                &failure_project_name,
                &failure_milestone_id,
                &failure_mid_stage_id,
                &failure_subtask_id,
                &failure_subtask_title,
                next_idx,
                total,
                &failure_execution_id,
                &error,
                background_pipeline_state.clone(),
                operation_source,
            )
            .await
            {
                let mut guard = background_pipeline_state.lock().await;
                if let Some(pipeline) = guard.as_mut() {
                    if pipeline.execution_id == failure_execution_id {
                        pipeline.status = PipelineStatus::Failed;
                        pipeline.last_error = Some(format!(
                            "{}；失败状态持久化失败：{}",
                            error.message, persist_error
                        ));
                    }
                }
            }
        }
    });

    Ok(initial_state)
}

fn interrupted_execution_cost_facts(
    error: &crate::engine::EngineError,
) -> Option<(
    Option<&crate::cost_ledger::ProviderUsage>,
    bool,
    &'static str,
)> {
    let (execution_result, failure_kind) = match error {
        crate::engine::EngineError::Cancelled { execution_result } => {
            (execution_result.as_deref(), "Cancelled")
        }
        crate::engine::EngineError::Timeout { execution_result } => {
            (execution_result.as_deref(), "Timeout")
        }
        crate::engine::EngineError::ResourceHardStop {
            execution_result, ..
        } => (execution_result.as_deref(), "ResourceHardStop"),
        _ => return None,
    };
    Some((
        execution_result.and_then(|result| result.token_usage.as_ref()),
        execution_result.is_some_and(|result| !result.file_changes.is_empty()),
        failure_kind,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn execute_current_subtask_background(
    project_name: String,
    project_path: String,
    milestone_id: String,
    mid_stage_id: String,
    subtask_id: String,
    subtask_title: String,
    subtask_goal: String,
    acceptance_criteria: Vec<String>,
    approved_prompt: String,
    authorized_paths: Vec<String>,
    subtask_idx: usize,
    total: usize,
    execution_id: String,
    execution_profile: project::ExecutionProfile,
    task_budget: crate::task_contract::TaskBudgetSummary,
    prepared_engine: crate::engine::PreparedEngine,
    pipeline_state: std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
    operation_source: project::OperationSource,
) -> Result<(), BackgroundExecutionFailure> {
    let execution_started_at = chrono::Utc::now().to_rfc3339();
    let execution_timer = std::time::Instant::now();
    let execution_model = if execution_profile.runtime == project::ExecutionRuntime::BuiltIn {
        prepared_engine.settings().built_in_grok_build.model.clone()
    } else {
        execution_profile.provider.display_name().to_string()
    };
    let engine_result = crate::engine::execute(
        prepared_engine,
        crate::engine::ExecutionRequest {
            project_path: project_path.clone(),
            prompt: approved_prompt.clone(),
            authorized_paths: authorized_paths.clone(),
            subtask_id: subtask_id.clone(),
            execution_id: execution_id.clone(),
            task_budget,
        },
        pipeline_state.clone(),
    )
    .await;
    let execution_elapsed_ms = execution_timer.elapsed().as_millis() as u64;
    let cost_stage_id = if mid_stage_id.is_empty() {
        milestone_id.clone()
    } else {
        mid_stage_id.clone()
    };
    let execution_context = crate::cost_ledger::ModelCallContext {
        project_name: project_name.clone(),
        milestone_id: milestone_id.clone(),
        stage_id: cost_stage_id.clone(),
        task_id: subtask_id.clone(),
        purpose: Some(crate::cost_ledger::ModelCallPurpose::Execution),
        ..Default::default()
    };
    let record_execution_cost =
        |usage: Option<&crate::cost_ledger::ProviderUsage>, produced_change, failure_kind: &str| {
            crate::cost_ledger::record_execution_call_best_effort(
                &project_name,
                &execution_id,
                &execution_context,
                execution_profile.provider.display_name(),
                &execution_model,
                execution_started_at.clone(),
                execution_elapsed_ms,
                usage,
                produced_change,
                failure_kind,
            );
        };
    if let Err(error) = &engine_result {
        if let Some((usage, produced_change, failure_kind)) =
            interrupted_execution_cost_facts(error)
        {
            record_execution_cost(usage, produced_change, failure_kind);
        }
    }
    let exec_result = match engine_result {
        Ok(result) => {
            let failure_kind = result
                .engine_failure_kind
                .as_ref()
                .map(|kind| format!("{kind:?}"))
                .unwrap_or_default();
            record_execution_cost(
                result.token_usage.as_ref(),
                !result.file_changes.is_empty(),
                &failure_kind,
            );
            result
        }
        Err(crate::engine::EngineError::Cancelled { .. }) => return Ok(()),
        Err(crate::engine::EngineError::Timeout { execution_result }) => {
            return Err(BackgroundExecutionFailure::engine(
                project::RecoveryErrorKind::ExecutionError,
                project::EngineFailureKind::Timeout,
                "执行超时".to_string(),
                execution_result.map(|result| *result),
            ));
        }
        Err(crate::engine::EngineError::ResourceHardStop {
            execution_result,
            resource_observation,
        }) => {
            return Err(BackgroundExecutionFailure::resource(
                project::RecoveryErrorKind::ExecutionError,
                "执行因资源压力达到硬停止阈值而终止".to_string(),
                execution_result.map(|result| *result),
                resource_observation,
            ));
        }
        Err(error) => {
            let message = error.to_string();
            let kind = crate::engine::classify_process_failure(None, &message, "");
            record_execution_cost(None, false, &format!("{kind:?}"));
            return Err(BackgroundExecutionFailure::engine(
                project::RecoveryErrorKind::ExecutionError,
                kind,
                message,
                None,
            ));
        }
    };

    if !exec_result.success {
        let message = if exec_result.error_log.is_empty() {
            format!("{} 非零退出", execution_profile.provider.display_name())
        } else {
            exec_result.error_log.clone()
        };
        let kind = exec_result
            .engine_failure_kind
            .clone()
            .unwrap_or(project::EngineFailureKind::TaskExecutionError);
        return Err(BackgroundExecutionFailure::engine(
            project::RecoveryErrorKind::ExecutionError,
            kind,
            message,
            Some(exec_result),
        ));
    }

    let out_of_scope =
        crate::plan_contract::out_of_scope_changes(&exec_result.file_changes, &authorized_paths);
    if !out_of_scope.is_empty() {
        return Err(BackgroundExecutionFailure::new(
            project::RecoveryErrorKind::ScopeViolation,
            format!(
                "执行修改了计划范围外文件：{}。必须恢复执行基线后重新规划或重试。",
                out_of_scope.join("、")
            ),
        ));
    }

    // 执行器结束后立即进入测试阶段，便于前端区分执行/测试
    {
        let mut guard = pipeline_state.lock().await;
        if let Some(pipeline) = guard.as_mut() {
            if pipeline.execution_id == execution_id && pipeline.status == PipelineStatus::Running {
                append_log(
                    pipeline,
                    "info",
                    format!(
                        "🧪 执行完成，正在测试 ({}/{})：{}",
                        subtask_idx + 1,
                        total,
                        subtask_title
                    ),
                );
                if let Some(status) = pipeline.subtask_statuses.get_mut(subtask_idx) {
                    status.status = "testing".to_string();
                }
            }
        }
    }

    let progress_project_name = project_name.clone();
    let progress_execution_id = execution_id.clone();
    let progress: crate::test_runner::VerificationProgressReporter =
        std::sync::Arc::new(move |stage| {
            let _ = persist_verification_progress(
                &progress_project_name,
                &progress_execution_id,
                stage,
            );
        });
    let mut test = crate::test_runner::check_subtask_with_context_and_progress_and_model(
        &project_path,
        &subtask_goal,
        &subtask_id,
        &milestone_id,
        &mid_stage_id,
        Some(acceptance_criteria.clone()),
        Some(authorized_paths.clone()),
        Some(approved_prompt),
        None,
        progress,
        Some(crate::cost_ledger::ModelCallContext {
            project_name: project_name.clone(),
            milestone_id: milestone_id.clone(),
            stage_id: cost_stage_id,
            task_id: subtask_id.clone(),
            purpose: Some(crate::cost_ledger::ModelCallPurpose::Review),
            ..Default::default()
        }),
    )
    .await
    .unwrap_or(project::TestResult {
        passed: false,
        issues: vec!["测试服务不可用".to_string()],
        suggestion: "请手动检查".to_string(),
        warnings: vec![],
        automated_test_status: project::AutomatedTestStatus::Unavailable,
        ..Default::default()
    });
    test.acceptance_results =
        crate::acceptance::build_ledger(&acceptance_criteria, &test, &authorized_paths);
    let quality = crate::quality_gate::evaluate(
        Some(&test),
        &test.acceptance_results,
        acceptance_criteria.len(),
        false,
    );
    test.passed = quality.passed();

    // 与暂停命令共用流水线锁，保证 execution_id 校验到项目保存之间不被旧任务穿透。
    let mut pipeline_guard = pipeline_state.lock().await;
    let pipeline_matches = pipeline_owner_matches(pipeline_guard.as_ref(), &execution_id);
    if !pipeline_matches {
        return Ok(());
    }

    let mut proj = crate::load_project(&project_name).map_err(|error| {
        BackgroundExecutionFailure::new(project::RecoveryErrorKind::StateConflict, error)
    })?;
    let session = match proj.execution_session.as_ref() {
        Some(session)
            if session.active
                && session.status == "executing"
                && session.execution_id == execution_id =>
        {
            session.clone()
        }
        _ => return Ok(()),
    };
    if proj.workflow_state.current_step == project::WorkflowStep::PauseDecision {
        return Ok(());
    }
    if proj.task_control.tree_revision != session.task_tree_revision {
        return Err(BackgroundExecutionFailure::state_conflict(
            "任务执行期间任务树修订发生变化，拒绝旧结果写回",
        ));
    }
    let current_address = crate::task_tree::locate_task(&proj, &subtask_id)
        .map_err(|error| {
            BackgroundExecutionFailure::new(project::RecoveryErrorKind::StateConflict, error)
        })?
        .ok_or_else(|| BackgroundExecutionFailure::state_conflict("目标叶子任务不存在"))?;
    if current_address.task_path() != session.task_path {
        return Err(BackgroundExecutionFailure::state_conflict(
            "执行会话祖先路径与磁盘任务树不一致",
        ));
    }
    let current_task = crate::task_tree::find_task(&proj, &subtask_id)
        .map_err(|error| {
            BackgroundExecutionFailure::new(project::RecoveryErrorKind::StateConflict, error)
        })?
        .ok_or_else(|| BackgroundExecutionFailure::state_conflict("目标叶子任务不存在"))?;
    if !current_task.child_tasks.is_empty() {
        return Err(BackgroundExecutionFailure::state_conflict(
            "执行目标已变成父任务，拒绝旧结果写回",
        ));
    }
    let workload = crate::workload_policy::current_profile(&proj).map_err(|error| {
        BackgroundExecutionFailure::new(project::RecoveryErrorKind::StateConflict, error)
    })?;
    let current_contract = crate::task_compiler::compile(
        current_task,
        current_address.ancestor_task_ids.last().map(String::as_str),
        current_address.depth,
        workload,
    )
    .contract;
    if !session.contract_fingerprint.is_empty()
        && current_contract.fingerprint != session.contract_fingerprint
    {
        return Err(BackgroundExecutionFailure::state_conflict(
            "任务合同在执行期间发生变化，拒绝旧结果写回",
        ));
    }

    {
        let subtask = crate::task_tree::find_task_mut(&mut proj, &subtask_id)
            .map_err(|error| {
                BackgroundExecutionFailure::new(project::RecoveryErrorKind::StateConflict, error)
            })?
            .ok_or_else(|| BackgroundExecutionFailure::state_conflict("目标叶子任务不存在"))?;
        if subtask.id != subtask_id || subtask.status != project::SubtaskStatus::Executing {
            return Ok(());
        }
        subtask.execution_result = Some(exec_result);
        subtask.test_result = Some(test.clone());
        subtask.acceptance_ledger = test.acceptance_results.clone();
        subtask.status = project::SubtaskStatus::AwaitingConfirmation;
    }

    let now_await = chrono::Utc::now().to_rfc3339();
    proj.execution_session = Some(project::ExecutionSession {
        execution_id: execution_id.clone(),
        active: true,
        milestone_id: milestone_id.clone(),
        mid_stage_id: mid_stage_id.clone(),
        subtask_id: subtask_id.clone(),
        subtask_title: subtask_title.clone(),
        status: "awaiting_confirmation".to_string(),
        base_commit: session.base_commit,
        failure_message: String::new(),
        verification_stage: test.verification_stage.clone(),
        confirmation_transaction_id: String::new(),
        confirmation_phase: project::ConfirmationPhase::NotStarted,
        confirmation_candidate_tag: String::new(),
        confirmation_commit: String::new(),
        confirmation_failure_kind: None,
        started_at: session.started_at,
        state_entered_at: now_await,
        plan_revision: session.plan_revision,
        subtask_index: subtask_idx,
        total_subtasks: total,
        task_path: session.task_path,
        parent_task_id: session.parent_task_id,
        top_level_task_id: session.top_level_task_id,
        task_tree_revision: session.task_tree_revision,
        contract_fingerprint: session.contract_fingerprint,
        node_depth: session.node_depth,
        engine_snapshot: session.engine_snapshot,
        engine_settings_revision: session.engine_settings_revision,
        engine_source_revision: session.engine_source_revision,
        engine_api_backend: session.engine_api_backend,
        engine_model: session.engine_model,
        endpoint_fingerprint: session.endpoint_fingerprint,
        engine_executable_path: session.engine_executable_path,
        human_review_cadence: session.human_review_cadence,
    });
    write_execution_history_with_source(
        &mut proj,
        "info",
        project::ExecutionEventType::ExecutorComplete,
        operation_source,
        format!(
            "✅ 执行完成 ({}/{})：{}",
            subtask_idx + 1,
            total,
            subtask_title
        ),
        Some(&milestone_id),
        Some(&mid_stage_id),
        Some(&subtask_id),
    );
    write_execution_history_with_source(
        &mut proj,
        if quality.passed() { "success" } else { "error" },
        project::ExecutionEventType::TestComplete,
        operation_source,
        if quality.passed() {
            format!(
                "🔍 质量门禁通过 ({}/{})：{}",
                subtask_idx + 1,
                total,
                subtask_title
            )
        } else {
            format!(
                "🔍 质量门禁阻断 ({}/{})：{} — {}",
                subtask_idx + 1,
                total,
                subtask_title,
                quality.message
            )
        },
        Some(&milestone_id),
        Some(&mid_stage_id),
        Some(&subtask_id),
    );
    write_execution_history_with_source(
        &mut proj,
        "info",
        project::ExecutionEventType::AwaitingConfirmation,
        operation_source,
        format!(
            "⏳ 待确认 ({}/{})：{}",
            subtask_idx + 1,
            total,
            subtask_title
        ),
        Some(&milestone_id),
        Some(&mid_stage_id),
        Some(&subtask_id),
    );
    crate::save_project(&proj).map_err(|error| {
        BackgroundExecutionFailure::new(project::RecoveryErrorKind::StateConflict, error)
    })?;

    if let Some(pipeline) = pipeline_guard.as_mut() {
        if pipeline.execution_id == execution_id {
            pipeline.status = PipelineStatus::Paused;
            append_log(
                pipeline,
                "info",
                format!(
                    "⏳ 待确认 ({}/{})：{}",
                    subtask_idx + 1,
                    total,
                    subtask_title
                ),
            );
            pipeline.awaiting_confirmation = true;
            if let Some(status) = pipeline.subtask_statuses.get_mut(subtask_idx) {
                status.status = "testing".to_string();
                status.test_result = Some(test);
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct BackgroundExecutionFailure {
    kind: project::RecoveryErrorKind,
    message: String,
    engine_failure_kind: Option<project::EngineFailureKind>,
    execution_result: Option<project::ExecutionResult>,
    resource_observation: project::ResourceObservationSummary,
    resource_failure_kind: Option<project::ResourceFailureKind>,
}

impl BackgroundExecutionFailure {
    fn new(kind: project::RecoveryErrorKind, message: String) -> Self {
        Self {
            kind,
            message,
            engine_failure_kind: None,
            execution_result: None,
            resource_observation: project::ResourceObservationSummary::default(),
            resource_failure_kind: None,
        }
    }

    fn engine(
        kind: project::RecoveryErrorKind,
        engine_failure_kind: project::EngineFailureKind,
        message: String,
        execution_result: Option<project::ExecutionResult>,
    ) -> Self {
        Self {
            kind,
            message,
            engine_failure_kind: Some(engine_failure_kind),
            execution_result,
            resource_observation: project::ResourceObservationSummary::default(),
            resource_failure_kind: None,
        }
    }

    fn resource(
        kind: project::RecoveryErrorKind,
        message: String,
        execution_result: Option<project::ExecutionResult>,
        resource_observation: Option<project::ResourceObservationSummary>,
    ) -> Self {
        Self {
            kind,
            message,
            engine_failure_kind: None,
            execution_result,
            resource_observation: resource_observation.unwrap_or_else(|| {
                project::ResourceObservationSummary {
                    state: project::ResourceObservationState::HardStop,
                    ..Default::default()
                }
            }),
            resource_failure_kind: Some(project::ResourceFailureKind::ResourcePressure),
        }
    }

    fn state_conflict(message: &str) -> Self {
        Self::new(
            project::RecoveryErrorKind::StateConflict,
            message.to_string(),
        )
    }
}

#[allow(clippy::too_many_arguments)]
async fn finalize_background_execution_failure(
    project_name: &str,
    milestone_id: &str,
    mid_stage_id: &str,
    subtask_id: &str,
    subtask_title: &str,
    subtask_idx: usize,
    total: usize,
    execution_id: &str,
    failure: &BackgroundExecutionFailure,
    pipeline_state: std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
    operation_source: project::OperationSource,
) -> Result<(), String> {
    let mut pipeline_guard = pipeline_state.lock().await;
    let pipeline_matches = pipeline_owner_matches(pipeline_guard.as_ref(), execution_id);
    if !pipeline_matches {
        return Ok(());
    }

    let mut proj = crate::load_project(project_name)?;
    let session_matches = proj
        .execution_session
        .as_ref()
        .map(|session| session.active && session.execution_id == execution_id)
        .unwrap_or(false);
    if !session_matches || proj.workflow_state.current_step == project::WorkflowStep::PauseDecision
    {
        return Ok(());
    }

    let resource_blocked = failure.resource_failure_kind.is_some();
    let engine_blocked = failure
        .engine_failure_kind
        .as_ref()
        .is_some_and(crate::engine::blocks_code_recovery)
        || resource_blocked;
    let engine_boundary = failure
        .engine_failure_kind
        .as_ref()
        .map(crate::recovery::engine_block_boundary);
    let baseline = proj
        .execution_session
        .as_ref()
        .map(|session| session.base_commit.clone())
        .unwrap_or_default();

    finalize_execution_failure(
        &mut proj,
        &mut *pipeline_guard,
        subtask_id,
        &failure.message,
        failure.execution_result.clone(),
        failure.kind != project::RecoveryErrorKind::StateConflict,
    );
    let baseline_outcome = if engine_blocked {
        let target = if baseline.is_empty() {
            "HEAD"
        } else {
            &baseline
        };
        Some(restore_git_execution_baseline(&proj.project_path, target))
    } else {
        None
    };
    let effective_message = match baseline_outcome.as_ref() {
        Some(Ok(outcome)) => format!(
            "{}；执行引擎阻断后已恢复任务基线（{}）",
            failure.message, outcome.target_summary
        ),
        Some(Err(outcome)) => format!(
            "{}；执行引擎阻断后恢复任务基线失败：{}",
            failure.message,
            outcome.error_message()
        ),
        None => failure.message.clone(),
    };
    crate::recovery::begin_execution_recovery(
        &mut proj,
        if engine_blocked && !resource_blocked {
            engine_boundary
                .as_ref()
                .map(|(error_kind, _)| error_kind.clone())
                .unwrap_or(project::RecoveryErrorKind::EngineBlocked)
        } else {
            failure.kind.clone()
        },
        execution_id,
        &effective_message,
    );
    if let Some(recovery) = proj.workflow_state.recovery_state.as_mut() {
        recovery.engine_failure_kind = failure.engine_failure_kind.clone();
        if let Some((_, phase)) = engine_boundary.as_ref() {
            recovery.phase = phase.clone();
        }
        match baseline_outcome.as_ref() {
            Some(Ok(outcome)) | Some(Err(outcome)) => {
                apply_baseline_restore_outcome(recovery, outcome);
            }
            None => {
                recovery.baseline_status = project::RecoveryBaselineStatus::NotRequired;
            }
        }
    }
    if let Some(resource_failure_kind) = failure.resource_failure_kind {
        crate::recovery::record_resource_facts(
            &mut proj,
            failure.resource_observation.clone(),
            Some(resource_failure_kind),
            &failure.message,
        );
    }
    if engine_blocked {
        if let Some(session) = proj.execution_session.as_mut() {
            session.failure_message = effective_message.clone();
        }
        if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
            let failure_kind = failure
                .engine_failure_kind
                .as_ref()
                .map(crate::autopilot_failure::from_engine_failure)
                .unwrap_or(project::AutopilotFailureKind::Permanent);
            let next_attempt = autopilot.transient_retry_count.saturating_add(1);
            let retry_delay = crate::autopilot_failure::is_transient(&failure_kind)
                .then(|| crate::autopilot_failure::retry_delay_secs(next_attempt))
                .flatten();
            autopilot.run_status = project::AutopilotRunStatus::ErrorStopped;
            autopilot.error_message = effective_message.clone();
            autopilot.last_failure_kind = failure_kind;
            autopilot.last_failure_fingerprint =
                crate::autopilot_policy::text_fingerprint(&effective_message);
            if let Some(delay_secs) = retry_delay {
                autopilot.transient_retry_count = next_attempt;
                autopilot.next_retry_at = Some(
                    (chrono::Utc::now() + chrono::Duration::seconds(delay_secs as i64))
                        .to_rfc3339(),
                );
                autopilot.last_action = format!(
                    "执行引擎暂时不可用；将在 {} 秒后自动重试（{}/{})",
                    delay_secs,
                    next_attempt,
                    crate::autopilot_failure::MAX_TRANSIENT_RETRIES
                );
                autopilot.recovery_action =
                    project::AutopilotRecoveryAction::RestoreExecutionBaseline;
            } else {
                autopilot.next_retry_at = None;
                if failure.engine_failure_kind == Some(project::EngineFailureKind::OutputTruncated)
                {
                    // Single entry only: current-subtask replan via run_error_recovery.
                    // Never set RegenerateExecutionPlan (whole-stage plan regen is illegal here).
                    // Permanent failure facts stay; transport/transient retry stays 0/3.
                    autopilot.run_status = project::AutopilotRunStatus::Running;
                    autopilot.last_action =
                        "内置执行续执行后仍被截断，正在进入受限重规划".to_string();
                    autopilot.recovery_action =
                        project::AutopilotRecoveryAction::RunAutomaticRecovery;
                } else {
                    autopilot.last_action =
                        if crate::autopilot_failure::is_transient(&autopilot.last_failure_kind) {
                            "执行引擎自动重试已耗尽，等待人工处理".to_string()
                        } else {
                            "执行引擎错误不可自动重试，等待人工处理".to_string()
                        };
                    autopilot.recovery_action = project::AutopilotRecoveryAction::WaitHumanDecision;
                }
            }
            autopilot.last_action_at = chrono::Utc::now().to_rfc3339();
        }
    }
    write_execution_history_with_source(
        &mut proj,
        "error",
        project::ExecutionEventType::ExecutionFailed,
        operation_source,
        format!(
            "❌ 执行失败 ({}/{}): {} - {}",
            subtask_idx + 1,
            total,
            subtask_title,
            effective_message
        ),
        Some(milestone_id),
        Some(mid_stage_id),
        Some(subtask_id),
    );
    crate::save_project(&proj)
}

/// 质量门禁：校验执行结果、测试结果和证据完整性。
/// 任一条件不满足都返回具体阻断原因。
pub(crate) fn validate_subtask_quality_gate(proj: &project::Project) -> Result<(), String> {
    validate_subtask_quality_gate_with_session_statuses(
        proj,
        &["awaiting_confirmation", "AwaitingConfirmation"],
    )
}

/// 确认路径在 CAS 认领后 session 为 `confirming`，仍按子任务证据做质量门禁。
fn validate_subtask_quality_gate_allowing_claim(proj: &project::Project) -> Result<(), String> {
    validate_subtask_quality_gate_with_session_statuses(
        proj,
        &[
            "awaiting_confirmation",
            "AwaitingConfirmation",
            "confirming",
        ],
    )
}

fn validate_subtask_quality_gate_with_session_statuses(
    proj: &project::Project,
    allowed_session_statuses: &[&str],
) -> Result<(), String> {
    let session = proj
        .execution_session
        .as_ref()
        .ok_or("没有活跃的执行会话。".to_string())?;

    if !allowed_session_statuses
        .iter()
        .any(|status| session.status.eq_ignore_ascii_case(status))
    {
        return Err(format!(
            "任务未处于待确认状态（当前：{}），无法确认。",
            session.status
        ));
    }

    let address = crate::task_tree::locate_task(proj, &session.subtask_id)?
        .ok_or_else(|| "执行会话中的任务不存在。".to_string())?;
    if proj.task_control.tree_revision != session.task_tree_revision {
        return Err("任务树修订已变化，旧执行会话不能确认。".to_string());
    }
    if !session.task_path.is_empty() && address.task_path() != session.task_path {
        return Err("执行会话中的任务路径与磁盘任务树不一致。".to_string());
    }
    let subtask = crate::task_tree::find_task(proj, &session.subtask_id)?
        .ok_or_else(|| "执行会话中的任务不存在。".to_string())?;
    if !subtask.child_tasks.is_empty() {
        return Err("执行会话指向父任务，不能进入质量确认。".to_string());
    }
    let workload = crate::workload_policy::current_profile(proj)?;
    let contract = crate::task_compiler::compile(
        subtask,
        address.ancestor_task_ids.last().map(String::as_str),
        address.depth,
        workload,
    )
    .contract;
    if !session.contract_fingerprint.is_empty()
        && contract.fingerprint != session.contract_fingerprint
    {
        return Err("任务合同已变化，旧执行会话不能确认。".to_string());
    }

    let quality = quality_evaluation_for_completion(proj, subtask)?;

    match crate::quality_gate::decide_completion(subtask, Some(&quality), false) {
        crate::quality_gate::CompletionDecision::AwaitingConfirmation
        | crate::quality_gate::CompletionDecision::Completed => Ok(()),
        crate::quality_gate::CompletionDecision::Blocked(reason) => Err(reason),
    }
}

fn quality_evaluation_for_completion(
    proj: &project::Project,
    subtask: &project::Subtask,
) -> Result<crate::quality_gate::QualityGateEvaluation, String> {
    let human_override = subtask
        .human_verification
        .as_ref()
        .is_some_and(|verification| {
            matches!(
                verification.resolution,
                project::HumanResolution::ConfirmActualPass
                    | project::HumanResolution::AcceptDeviation
            )
        });
    if human_override {
        // 人工核验是独立的通过通道；真实测试结果保持原值。
        crate::human_action_policy::validate_recorded_human_acceptance(proj, subtask)?;
        return Ok(crate::quality_gate::QualityGateEvaluation {
            outcome: crate::quality_gate::QualityGateOutcome::Passed,
            message: "人工核验边界已记录".to_string(),
        });
    }
    let test_result = subtask
        .test_result
        .as_ref()
        .ok_or("缺少测试结果，无法确认。测试服务可能不可用。".to_string())?;
    Ok(crate::quality_gate::evaluate_with_deferred(
        Some(test_result),
        &subtask.acceptance_ledger,
        subtask.acceptance_criteria.len(),
        false,
        batch_deferred_review_is_complete(proj, subtask),
    ))
}

fn batch_deferred_review_is_complete(proj: &project::Project, subtask: &project::Subtask) -> bool {
    if proj.human_review_cadence != project::HumanReviewCadence::MilestoneBatch {
        return false;
    }
    let deferred = subtask
        .acceptance_ledger
        .iter()
        .filter(|item| {
            matches!(
                item.status,
                project::AcceptanceStatus::AiProvisionallySatisfied
                    | project::AcceptanceStatus::DeferredHumanReview
            )
        })
        .collect::<Vec<_>>();
    if deferred.is_empty() {
        return false;
    }
    let Ok(Some(address)) = crate::task_tree::locate_task(proj, &subtask.id) else {
        return false;
    };
    let Some(milestone) = proj
        .milestones
        .iter()
        .find(|milestone| milestone.id == address.milestone_id)
    else {
        return false;
    };
    let contract_fingerprint = subtask
        .contract_snapshot
        .as_ref()
        .map(|contract| contract.fingerprint.as_str())
        .unwrap_or_default();
    let Ok(execution_facts_fingerprint) =
        crate::human_action_policy::execution_result_fingerprint(subtask)
    else {
        return false;
    };
    deferred.iter().all(|ledger| {
        let id = project::milestone_human_review_item_id(
            &address.milestone_id,
            &subtask.id,
            ledger.criterion_index,
            contract_fingerprint,
        );
        milestone.human_review_items.iter().any(|item| {
            item.id == id
                && item.review_cycle == milestone.human_review_cycle
                && item.contract_fingerprint == contract_fingerprint
                && item.execution_facts_fingerprint == execution_facts_fingerprint
                && item.ai_status == ledger.status
        })
    })
}

/// Reconcile stage state after any terminal task outcome. Passing, accepting a
/// deviation, and dependency-approved skipping must advance through the same
/// state transition instead of leaving a completed stage marked InProgress.
pub(crate) fn reconcile_terminal_stage(
    proj: &mut project::Project,
    milestone_id: &str,
    mid_stage_id: &str,
) -> Result<(bool, bool), String> {
    let now = chrono::Utc::now().to_rfc3339();
    if mid_stage_id.is_empty() {
        let milestone_completed = proj
            .milestones
            .iter()
            .find(|milestone| milestone.id == milestone_id)
            .is_some_and(|milestone| {
                milestone.mode == project::StageMode::Quick
                    && milestone.mid_stages.is_empty()
                    && !milestone.subtasks.is_empty()
                    && milestone.subtasks.iter().all(|subtask| {
                        crate::workflow_resolution::is_terminal_subtask(&subtask.status)
                    })
            });
        if !milestone_completed {
            return Ok((false, false));
        }
        crate::workflow_resolution::apply_milestone_review_boundary(proj, milestone_id, &now)?;
        proj.workflow_state.data_revision = proj.workflow_state.data_revision.saturating_add(1);
        proj.workflow_state.last_transition_at = now;
        return Ok((true, true));
    }
    let mid_completed =
        proj.milestones
            .iter()
            .find(|milestone| milestone.id == milestone_id)
            .and_then(|milestone| {
                milestone
                    .mid_stages
                    .iter()
                    .find(|mid_stage| mid_stage.id == mid_stage_id)
            })
            .is_some_and(|mid_stage| {
                !mid_stage.subtasks.is_empty()
                    && mid_stage.subtasks.iter().all(|subtask| {
                        crate::workflow_resolution::is_terminal_subtask(&subtask.status)
                    })
            });
    if !mid_completed {
        return Ok((false, false));
    }

    let milestone_completed = proj
        .milestones
        .iter()
        .find(|milestone| milestone.id == milestone_id)
        .is_some_and(|milestone| {
            !milestone.mid_stages.is_empty()
                && milestone.mid_stages.iter().all(|mid_stage| {
                    mid_stage.id == mid_stage_id
                        || mid_stage.status == project::MidStageStatus::Completed
                })
        });
    if milestone_completed {
        let mut candidate = proj.clone();
        let mid_stage = candidate
            .milestones
            .iter_mut()
            .find(|milestone| milestone.id == milestone_id)
            .and_then(|milestone| {
                milestone
                    .mid_stages
                    .iter_mut()
                    .find(|mid_stage| mid_stage.id == mid_stage_id)
            })
            .ok_or_else(|| format!("中阶段不存在：{}", mid_stage_id))?;
        mid_stage.status = project::MidStageStatus::Completed;
        mid_stage.completed_at = Some(now.clone());
        crate::workflow_resolution::apply_milestone_review_boundary(
            &mut candidate,
            milestone_id,
            &now,
        )?;
        candidate.workflow_state.data_revision =
            candidate.workflow_state.data_revision.saturating_add(1);
        candidate.workflow_state.last_transition_at = now;
        *proj = candidate;
        return Ok((true, true));
    }

    let mid_stage = proj
        .milestones
        .iter_mut()
        .find(|milestone| milestone.id == milestone_id)
        .and_then(|milestone| {
            milestone
                .mid_stages
                .iter_mut()
                .find(|mid_stage| mid_stage.id == mid_stage_id)
        })
        .ok_or_else(|| format!("中阶段不存在：{}", mid_stage_id))?;
    mid_stage.status = project::MidStageStatus::Completed;
    mid_stage.completed_at = Some(now.clone());
    proj.workflow_state.current_step = project::WorkflowStep::MidStageSelection;
    proj.current_mid_stage_id.clear();
    proj.workflow_state.data_revision = proj.workflow_state.data_revision.saturating_add(1);
    proj.workflow_state.last_transition_at = now;
    Ok((true, false))
}

/// 执行器失败时修正磁盘任务、会话、流水线和自动驾驶状态
fn finalize_execution_failure(
    proj: &mut project::Project,
    pipeline_state: &mut Option<PipelineState>,
    subtask_id: &str,
    error_message: &str,
    execution_result: Option<project::ExecutionResult>,
    reset_task: bool,
) {
    let now = chrono::Utc::now().to_rfc3339();
    let mut error_chars = error_message.chars();
    let truncated_body: String = error_chars.by_ref().take(2048).collect();
    let truncated = if error_chars.next().is_some() {
        format!("{}...", truncated_body)
    } else {
        truncated_body
    };

    // 修正执行会话：保留 execution_id / subtask_id / base_commit 与失败原因
    if let Some(ref mut session) = proj.execution_session {
        session.active = false;
        session.status = "execution_failed".to_string();
        session.failure_message = truncated.clone();
        session.state_entered_at = now.clone();
    }

    // 修正小阶段状态：回到 Pending，但可由会话状态定位为可恢复（不依赖 retry_count）
    if reset_task {
        if let Ok(Some(st)) = crate::task_tree::find_task_mut(proj, subtask_id) {
            if st.child_tasks.is_empty() {
                st.status = project::SubtaskStatus::Pending;
                st.execution_result = execution_result;
                st.test_result = None;
            }
        }
    }

    // 修正流水线状态
    if let Some(ref mut ps) = pipeline_state {
        ps.status = PipelineStatus::Failed;
        ps.last_error = Some(error_message.to_string());
        ps.awaiting_confirmation = false;
        append_log(ps, "error", format!("❌ 执行失败：{}", truncated));
    }

    // 自动驾驶活跃时标记错误，并显式写入恢复动作（不靠错误文本猜测）
    if proj.workflow_state.autopilot_active {
        if let Some(ref mut ap) = proj.workflow_state.autopilot_state {
            ap.run_status = project::AutopilotRunStatus::ErrorStopped;
            ap.last_action = format!("执行器失败：{}", error_message);
            ap.last_action_at = now.clone();
            ap.error_message = truncated;
            ap.recovery_action = project::AutopilotRecoveryAction::RestoreExecutionBaseline;
        }
    }
}

/// 在流水线锁内认领待确认会话，防止自动确认与人工确认并发双提交。
///
/// 成功时把 session 标为 `claim_status`（`confirming` / `rejecting`）并落盘。
/// 调用方在失败路径必须调用 [`release_confirmation_claim`] 恢复 `awaiting_confirmation`。
fn claim_awaiting_confirmation_under_lock(
    proj: &mut project::Project,
    claim_status: &str,
) -> Result<(), String> {
    let has_awaiting = proj
        .execution_session
        .as_ref()
        .and_then(|session| {
            crate::task_tree::find_task(proj, &session.subtask_id)
                .ok()
                .flatten()
        })
        .is_some_and(|task| task.status == project::SubtaskStatus::AwaitingConfirmation);
    if !has_awaiting {
        return Err("没有待确认的小阶段。".to_string());
    }
    let session = proj
        .execution_session
        .as_mut()
        .ok_or_else(|| "没有活跃的执行会话。".to_string())?;
    let status = session.status.as_str();
    if status == "confirming" || status == "rejecting" {
        return Err("确认或驳回操作正在进行中，请勿重复提交。".to_string());
    }
    // 仅允许从待确认、质量阻断或结构化 Git 确认阻断进入认领
    let allowed = status.eq_ignore_ascii_case("awaiting_confirmation")
        || status.eq_ignore_ascii_case("quality_blocked")
        || status.eq_ignore_ascii_case("confirmation_blocked");
    if !allowed {
        return Err(format!(
            "任务未处于可确认状态（当前：{}），无法提交。",
            status
        ));
    }
    if claim_status == "confirming"
        && status.eq_ignore_ascii_case("confirmation_blocked")
        && !confirmation_failure_is_retryable(session.confirmation_failure_kind.as_ref())
    {
        return Err(
            "当前 Git 确认阻断需要人工核对，禁止机械重试；代码与质量结果仍被保留。".to_string(),
        );
    }
    if claim_status == "confirming" {
        if session.confirmation_transaction_id.is_empty() {
            session.confirmation_transaction_id = uuid::Uuid::new_v4().to_string();
        }
        if session.confirmation_phase == project::ConfirmationPhase::NotStarted {
            session.confirmation_phase = project::ConfirmationPhase::Preparing;
        }
        if session.confirmation_candidate_tag.is_empty() {
            session.confirmation_candidate_tag = crate::git_ops::subtask_v2_tag(
                &session.milestone_id,
                &session.mid_stage_id,
                &session.subtask_id,
                &session.confirmation_transaction_id,
            );
        }
        session.confirmation_failure_kind = None;
        session.failure_message.clear();
    }
    session.status = claim_status.to_string();
    session.state_entered_at = chrono::Utc::now().to_rfc3339();
    session.active = true;
    crate::save_project(proj)?;
    Ok(())
}

fn release_confirmation_claim(proj: &mut project::Project, restore_status: &str) {
    if let Some(ref mut session) = proj.execution_session {
        if session.status == "confirming" || session.status == "rejecting" {
            session.status = restore_status.to_string();
            session.state_entered_at = chrono::Utc::now().to_rfc3339();
        }
    }
}

fn confirmation_failure_is_retryable(
    failure_kind: Option<&project::GitConfirmationFailureKind>,
) -> bool {
    matches!(
        failure_kind,
        Some(
            project::GitConfirmationFailureKind::LegacyV1TagConflict
                | project::GitConfirmationFailureKind::CommitFailed
                | project::GitConfirmationFailureKind::TagFailed
                | project::GitConfirmationFailureKind::ProjectFinalizationFailed
                | project::GitConfirmationFailureKind::GitMetadataUnavailable
        )
    )
}

fn confirmation_recovery_action(
    failure_kind: &project::GitConfirmationFailureKind,
) -> project::AutopilotRecoveryAction {
    match failure_kind {
        project::GitConfirmationFailureKind::TagIdentityConflict
        | project::GitConfirmationFailureKind::V2TagIntegrityConflict
        | project::GitConfirmationFailureKind::ScopeViolation => {
            project::AutopilotRecoveryAction::WaitHumanDecision
        }
        project::GitConfirmationFailureKind::LegacyV1TagConflict
        | project::GitConfirmationFailureKind::CommitFailed
        | project::GitConfirmationFailureKind::TagFailed
        | project::GitConfirmationFailureKind::ProjectFinalizationFailed
        | project::GitConfirmationFailureKind::GitMetadataUnavailable => {
            project::AutopilotRecoveryAction::RetryGitConfirmation
        }
    }
}

fn mark_confirmation_blocked(
    proj: &mut project::Project,
    failure_kind: project::GitConfirmationFailureKind,
    message: String,
) {
    mark_confirmation_blocked_with_source(
        proj,
        failure_kind,
        message,
        project::OperationSource::System,
    );
}

fn mark_confirmation_blocked_with_source(
    proj: &mut project::Project,
    failure_kind: project::GitConfirmationFailureKind,
    message: String,
    operation_source: project::OperationSource,
) {
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(session) = proj.execution_session.as_mut() {
        session.active = false;
        session.status = "confirmation_blocked".to_string();
        session.failure_message = message.clone();
        session.confirmation_failure_kind = Some(failure_kind.clone());
        session.state_entered_at = now.clone();
    }
    if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
        autopilot.run_status = project::AutopilotRunStatus::ErrorStopped;
        autopilot.error_message = message.clone();
        autopilot.last_action = "Git 确认受阻".to_string();
        autopilot.last_action_at = now;
        autopilot.recovery_action = confirmation_recovery_action(&failure_kind);
    }
    let ids = proj.execution_session.as_ref().map(|session| {
        (
            session.milestone_id.clone(),
            session.mid_stage_id.clone(),
            session.subtask_id.clone(),
        )
    });
    if let Some((milestone_id, mid_stage_id, subtask_id)) = ids {
        write_execution_history_with_source(
            proj,
            "error",
            project::ExecutionEventType::GitConfirmationBlocked,
            operation_source,
            format!("Git 确认受阻：{}", message),
            Some(&milestone_id),
            Some(&mid_stage_id),
            Some(&subtask_id),
        );
    }
}

/// V1 确认小阶段执行结果（用户点击"确认通过"）
#[tauri::command]
pub(crate) async fn confirm_subtask_result(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    confirm_subtask_result_with_pipeline(&state.pipeline_state, project_name).await
}

pub(crate) async fn confirm_subtask_result_with_pipeline(
    pipeline_state: &std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
    project_name: String,
) -> Result<project::Project, String> {
    confirm_subtask_result_with_source(pipeline_state, project_name, project::OperationSource::User)
        .await
}

pub(crate) async fn confirm_subtask_result_with_source(
    pipeline_state: &std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
    project_name: String,
    operation_source: project::OperationSource,
) -> Result<project::Project, String> {
    // 与后台完成/启动对账共用流水线锁做 CAS 认领，关闭自动确认与人工确认的并发窗口。
    {
        let _guard = pipeline_state.lock().await;
        let mut claim_proj = crate::load_project(&project_name)?;
        claim_awaiting_confirmation_under_lock(&mut claim_proj, "confirming")?;
    }

    let mut proj = crate::load_project(&project_name)?;
    let project_path = proj.project_path.clone();

    let (milestone_id, mid_stage_id) = proj
        .execution_session
        .as_ref()
        .map(|session| (session.milestone_id.clone(), session.mid_stage_id.clone()))
        .ok_or_else(|| "没有活跃的执行会话。".to_string())?;
    if milestone_id.is_empty() {
        release_confirmation_claim(&mut proj, "awaiting_confirmation");
        let _ = crate::save_project(&proj);
        return Err("执行会话缺少大阶段身份。".to_string());
    }

    // 质量门禁：在创建 Git 标签之前校验执行/测试/证据完整性
    // 认领后 session 为 confirming，质量门禁仍按子任务状态判定
    if let Err(gate_reason) = validate_subtask_quality_gate_allowing_claim(&proj) {
        write_execution_history_with_source(
            &mut proj,
            "error",
            project::ExecutionEventType::QualityGateBlocked,
            operation_source,
            format!("🚫 质量门禁阻断：{}", gate_reason),
            Some(&milestone_id),
            Some(&mid_stage_id),
            None,
        );
        // 质量门禁需人工处理（确认面板提供驳回/重试）；不得伪装成“重新推进”或强制恢复基线
        if proj.workflow_state.autopilot_active {
            if let Some(ref mut ap) = proj.workflow_state.autopilot_state {
                let now = chrono::Utc::now().to_rfc3339();
                ap.run_status = project::AutopilotRunStatus::ErrorStopped;
                ap.last_action = format!("质量门禁阻断：{}", gate_reason);
                ap.last_action_at = now;
                ap.error_message = gate_reason.clone();
                ap.recovery_action = project::AutopilotRecoveryAction::WaitHumanDecision;
            }
        }
        if let Some(ref mut session) = proj.execution_session {
            session.status = "quality_blocked".to_string();
            session.failure_message = gate_reason.clone();
            session.state_entered_at = chrono::Utc::now().to_rfc3339();
        }
        crate::save_project(&proj)?;
        return Err(gate_reason);
    }

    let precheck = (|| {
        let scope = crate::plan_scope::PlanScope::resolve(&proj)?;
        if proj.current_milestone_id != milestone_id || proj.current_mid_stage_id != mid_stage_id {
            return Err("执行会话与当前计划目标不一致。".to_string());
        }
        let milestone = scope.milestone(&proj);
        let milestone_title = milestone.title.clone();
        let plan_target_version = scope
            .mid_stage(&proj)
            .map(|stage| stage.version.clone())
            .unwrap_or_else(|| milestone.version.clone());
        let session = proj
            .execution_session
            .as_ref()
            .ok_or_else(|| "没有活跃的执行会话。".to_string())?;
        let subtask_idx = session.subtask_index;
        let task = crate::task_tree::find_task(&proj, &session.subtask_id)?
            .ok_or_else(|| "执行会话中的任务不存在。".to_string())?;
        if task.status != project::SubtaskStatus::AwaitingConfirmation {
            return Err("执行会话中的叶子任务未处于待确认状态。".to_string());
        }
        let subtask_id = task.id.clone();
        let subtask_title = task.title.clone();
        let authorized_paths = crate::plan_contract::validate_subtask(
            task,
            &format!("第 {} 个小阶段", subtask_idx + 1),
        )?;
        Ok::<_, String>((
            milestone_title,
            plan_target_version,
            subtask_idx,
            subtask_id,
            subtask_title,
            authorized_paths,
        ))
    })();

    let (
        milestone_title,
        plan_target_version,
        subtask_idx,
        subtask_id,
        subtask_title,
        authorized_paths,
    ) = match precheck {
        Ok(v) => v,
        Err(msg) => {
            release_confirmation_claim(&mut proj, "awaiting_confirmation");
            let _ = crate::save_project(&proj);
            return Err(msg);
        }
    };

    let (transaction_id, mut confirmation_phase, mut confirmation_commit, candidate_tag) = proj
        .execution_session
        .as_ref()
        .map(|session| {
            (
                session.confirmation_transaction_id.clone(),
                session.confirmation_phase.clone(),
                session.confirmation_commit.clone(),
                session.confirmation_candidate_tag.clone(),
            )
        })
        .ok_or_else(|| "没有活跃的执行会话。".to_string())?;

    // Verify Git workspace is still available before advancing the confirmation transaction.
    let ws = match get_execution_workspace_status_inner(&project_path) {
        Ok(ws) => ws,
        Err(e) => {
            mark_confirmation_blocked_with_source(
                &mut proj,
                project::GitConfirmationFailureKind::GitMetadataUnavailable,
                e.clone(),
                operation_source,
            );
            crate::save_project(&proj)?;
            return Err(e);
        }
    };
    let git_metadata_ready = ws.path_exists
        && ws.is_directory
        && ws.is_git_repo
        && ws.has_commits
        && ws.git_user_available
        && ws.git_email_available;
    if !git_metadata_ready {
        let message = format!("Git 工作区不可用，无法标记确认：{}", ws.status_message);
        mark_confirmation_blocked_with_source(
            &mut proj,
            project::GitConfirmationFailureKind::GitMetadataUnavailable,
            message.clone(),
            operation_source,
        );
        crate::save_project(&proj)?;
        return Err(message);
    }

    let now = chrono::Utc::now().to_rfc3339();

    // 只有准备阶段读取工作区并生成宪法更新；提交后的重试只读取已记录的提交。
    let task_diff_result = if confirmation_phase == project::ConfirmationPhase::Preparing {
        crate::git_ops::capture_authorized_diff(&project_path, &authorized_paths)
    } else if !confirmation_commit.is_empty() {
        crate::git_ops::capture_commit_diff(&project_path, &confirmation_commit, &authorized_paths)
    } else {
        Ok(String::new())
    };
    let mut task_diff_text = String::new();
    let mut pending_constitution_entry: Option<project::ConstitutionChangeEntry> = None;

    let generated_file_result = match task_diff_result {
        Ok(diff_text) => {
            task_diff_text = diff_text;
            let diff_summary = crate::diff::extract_diff_summary(&task_diff_text);
            let constitution_path = std::path::Path::new(&project_path).join("CONSTITUTION.md");
            if confirmation_phase == project::ConfirmationPhase::Preparing
                && constitution_path.exists()
            {
                let old_constitution = std::fs::read_to_string(&constitution_path)
                    .map_err(|error| format!("读取 CONSTITUTION.md 失败：{}", error));
                match old_constitution {
                    Ok(old_constitution) => {
                        match crate::constitution::update_constitution_with_context(
                            old_constitution.clone(),
                            diff_summary.clone(),
                            crate::cost_ledger::ModelCallContext {
                                project_name: project_name.clone(),
                                milestone_id: milestone_id.clone(),
                                stage_id: if mid_stage_id.is_empty() {
                                    milestone_id.clone()
                                } else {
                                    mid_stage_id.clone()
                                },
                                task_id: subtask_id.clone(),
                                purpose: Some(
                                    crate::cost_ledger::ModelCallPurpose::ConstitutionSummary,
                                ),
                                ..Default::default()
                            },
                        )
                        .await
                        {
                            Ok(updated_constitution) => {
                                if updated_constitution != old_constitution {
                                    let part2 = extract_constitution_part2(&updated_constitution);
                                    pending_constitution_entry =
                                        Some(project::ConstitutionChangeEntry {
                                            timestamp: now.clone(),
                                            subtask_id: subtask_id.clone(),
                                            subtask_title: subtask_title.clone(),
                                            change_summary: build_constitution_change_summary(
                                                &diff_summary,
                                            ),
                                            token_estimate: crate::constitution::estimate_tokens(
                                                &part2,
                                            ),
                                        });
                                    Ok(Some(crate::git_ops::GeneratedFileUpdate::constitution(
                                        old_constitution,
                                        updated_constitution,
                                    )))
                                } else {
                                    Ok(None)
                                }
                            }
                            Err(error) => Err(format!("更新 CONSTITUTION.md 失败：{}", error)),
                        }
                    }
                    Err(error) => Err(error),
                }
            } else {
                Ok(None)
            }
        }
        Err(error) => Err(error),
    };

    let mut generated_file = match generated_file_result {
        Ok(generated_file) => generated_file,
        Err(message) => {
            mark_confirmation_blocked_with_source(
                &mut proj,
                project::GitConfirmationFailureKind::CommitFailed,
                message.clone(),
                operation_source,
            );
            crate::save_project(&proj)?;
            return Err(format!("确认提交失败：{}", message));
        }
    };

    if confirmation_phase == project::ConfirmationPhase::Preparing {
        write_execution_history_with_source(
            &mut proj,
            "info",
            project::ExecutionEventType::GitConfirmationStarted,
            operation_source,
            format!("开始 Git 确认事务：{}", candidate_tag),
            Some(&milestone_id),
            Some(&mid_stage_id),
            Some(&subtask_id),
        );
    }

    let tag_name = loop {
        let progress = crate::git_ops::git_save_subtask(
            project_path.clone(),
            milestone_id.clone(),
            mid_stage_id.clone(),
            subtask_id.clone(),
            transaction_id.clone(),
            (subtask_idx + 1) as u32,
            plan_target_version.clone(),
            subtask_title.clone(),
            authorized_paths.clone(),
            generated_file.take(),
            confirmation_phase.clone(),
            confirmation_commit.clone(),
        )
        .await;

        match progress {
            Ok(crate::git_ops::GitSaveProgress::CommitCreated { commit, tag }) => {
                confirmation_phase = project::ConfirmationPhase::CommitCreated;
                confirmation_commit = commit.clone();
                if let Some(session) = proj.execution_session.as_mut() {
                    session.confirmation_phase = confirmation_phase.clone();
                    session.confirmation_commit = commit.clone();
                    session.confirmation_candidate_tag = tag;
                    session.confirmation_failure_kind = None;
                }
                write_execution_history_with_source(
                    &mut proj,
                    "info",
                    project::ExecutionEventType::GitConfirmationCommitCreated,
                    operation_source,
                    format!("Git 确认提交已创建：{}", commit),
                    Some(&milestone_id),
                    Some(&mid_stage_id),
                    Some(&subtask_id),
                );
                if let Err(error) = crate::save_project(&proj) {
                    let message = format!("确认提交已创建，但事务阶段保存失败：{}", error);
                    mark_confirmation_blocked_with_source(
                        &mut proj,
                        project::GitConfirmationFailureKind::ProjectFinalizationFailed,
                        message.clone(),
                        operation_source,
                    );
                    let _ = crate::save_project(&proj);
                    return Err(message);
                }
            }
            Ok(crate::git_ops::GitSaveProgress::TagCreated { commit, tag }) => {
                confirmation_phase = project::ConfirmationPhase::TagCreated;
                confirmation_commit = commit;
                if let Some(session) = proj.execution_session.as_mut() {
                    session.confirmation_phase = confirmation_phase.clone();
                    session.confirmation_commit = confirmation_commit.clone();
                    session.confirmation_candidate_tag = tag.clone();
                    session.confirmation_failure_kind = None;
                }
                if let Err(error) = crate::save_project(&proj) {
                    let message = format!("确认标签已创建，但事务阶段保存失败：{}", error);
                    mark_confirmation_blocked_with_source(
                        &mut proj,
                        project::GitConfirmationFailureKind::ProjectFinalizationFailed,
                        message.clone(),
                        operation_source,
                    );
                    let _ = crate::save_project(&proj);
                    return Err(message);
                }
                break tag;
            }
            Err(error) => {
                let message = error.message;
                mark_confirmation_blocked_with_source(
                    &mut proj,
                    error.kind,
                    message.clone(),
                    operation_source,
                );
                crate::save_project(&proj)?;
                return Err(format!("确认提交失败：{}", message));
            }
        }
    };

    if task_diff_text.is_empty() {
        task_diff_text = match crate::git_ops::capture_commit_diff(
            &project_path,
            &confirmation_commit,
            &authorized_paths,
        ) {
            Ok(diff) => diff,
            Err(error) => {
                let message = format!("确认标签已创建，但读取提交差异失败：{}", error);
                mark_confirmation_blocked_with_source(
                    &mut proj,
                    project::GitConfirmationFailureKind::ProjectFinalizationFailed,
                    message.clone(),
                    operation_source,
                );
                crate::save_project(&proj)?;
                return Err(message);
            }
        };
    }
    let constitution_changed = match crate::git_ops::commit_changed_path(
        &project_path,
        &confirmation_commit,
        "CONSTITUTION.md",
    ) {
        Ok(changed) => changed,
        Err(error) => {
            let message = format!("确认标签已创建，但读取收口文件失败：{}", error);
            mark_confirmation_blocked_with_source(
                &mut proj,
                project::GitConfirmationFailureKind::ProjectFinalizationFailed,
                message.clone(),
                operation_source,
            );
            crate::save_project(&proj)?;
            return Err(message);
        }
    };
    if pending_constitution_entry.is_none()
        && !proj
            .constitution_change_history
            .iter()
            .any(|entry| entry.subtask_id == subtask_id)
        && constitution_changed
    {
        let constitution_path = std::path::Path::new(&project_path).join("CONSTITUTION.md");
        if let Ok(content) = std::fs::read_to_string(constitution_path) {
            let diff_summary = crate::diff::extract_diff_summary(&task_diff_text);
            pending_constitution_entry = Some(project::ConstitutionChangeEntry {
                timestamp: now.clone(),
                subtask_id: subtask_id.clone(),
                subtask_title: subtask_title.clone(),
                change_summary: build_constitution_change_summary(&diff_summary),
                token_estimate: crate::constitution::estimate_tokens(&extract_constitution_part2(
                    &content,
                )),
            });
        }
    }

    if let Some(session) = proj.execution_session.as_mut() {
        session.confirmation_phase = project::ConfirmationPhase::ProjectFinalizing;
    }
    if let Err(error) = crate::save_project(&proj) {
        let message = format!("Git 标签已创建，但项目收口准备失败：{}", error);
        mark_confirmation_blocked_with_source(
            &mut proj,
            project::GitConfirmationFailureKind::ProjectFinalizationFailed,
            message.clone(),
            operation_source,
        );
        let _ = crate::save_project(&proj);
        return Err(message);
    }

    let completion_task = crate::task_tree::find_task(&proj, &subtask_id)?
        .ok_or_else(|| "完成裁决目标叶子任务不存在。".to_string())?;
    let completion_quality = quality_evaluation_for_completion(&proj, completion_task)?;
    let completion_decision =
        crate::quality_gate::decide_completion(completion_task, Some(&completion_quality), true);
    match completion_decision {
        crate::quality_gate::CompletionDecision::Completed => {}
        crate::quality_gate::CompletionDecision::AwaitingConfirmation => {
            let message = "唯一完成裁决阻断：确认事务未达到完成阶段".to_string();
            mark_confirmation_blocked_with_source(
                &mut proj,
                project::GitConfirmationFailureKind::ProjectFinalizationFailed,
                message.clone(),
                operation_source,
            );
            crate::save_project(&proj)?;
            return Err(message);
        }
        crate::quality_gate::CompletionDecision::Blocked(reason) => {
            let message = format!("唯一完成裁决阻断：{}", reason);
            mark_confirmation_blocked_with_source(
                &mut proj,
                project::GitConfirmationFailureKind::ProjectFinalizationFailed,
                message.clone(),
                operation_source,
            );
            crate::save_project(&proj)?;
            return Err(message);
        }
    }

    let st = crate::task_tree::find_task_mut(&mut proj, &subtask_id)?
        .ok_or_else(|| "确认目标叶子任务不存在。".to_string())?;
    if !st.child_tasks.is_empty() {
        return Err("确认目标已经变成父任务，拒绝写入确认结果。".to_string());
    }
    st.status = if st.human_verification.as_ref().is_some_and(|verification| {
        verification.resolution == project::HumanResolution::AcceptDeviation
    }) {
        project::SubtaskStatus::AcceptedDeviation
    } else {
        project::SubtaskStatus::Passed
    };
    st.confirmed_by_user = Some(true);
    st.confirmed_at = Some(now.clone());
    st.auto_tag = Some(tag_name);
    let aggregation = crate::task_aggregation::aggregate_ancestors(&mut proj, &subtask_id)?;
    if aggregation.contract_conflict {
        write_execution_history_with_source(
            &mut proj,
            "error",
            project::ExecutionEventType::QualityGateBlocked,
            operation_source,
            "父任务验收证据发生契约冲突，等待人工处理".to_string(),
            Some(&milestone_id),
            Some(&mid_stage_id),
            Some(&subtask_id),
        );
    }

    // === 记录本次授权代码变更历史，不把系统生成的宪法 diff 混入任务范围 ===
    {
        let diff_text = task_diff_text;
        if !diff_text.is_empty() {
            let files = extract_changed_files(&diff_text);
            let max_diff_len = 8000usize;
            let (truncated_diff, was_truncated) = if diff_text.len() > max_diff_len {
                (
                    diff_text.chars().take(max_diff_len).collect::<String>() + "\n…（diff 已截断）",
                    true,
                )
            } else {
                (diff_text, false)
            };
            proj.change_history.push(project::ChangeHistoryEntry {
                subtask_id: subtask_id.clone(),
                subtask_title: subtask_title.clone(),
                recorded_at: now.clone(),
                files_changed: files,
                diff_text: truncated_diff,
                diff_truncated: was_truncated,
            });
            // 限制历史上限
            const MAX_CHANGE_HISTORY: usize = 60;
            if proj.change_history.len() > MAX_CHANGE_HISTORY {
                let excess = proj.change_history.len() - MAX_CHANGE_HISTORY;
                proj.change_history.drain(0..excess);
            }
        }
    }

    if let Some(entry) = pending_constitution_entry {
        proj.constitution_change_history.push(entry);
        const MAX_CONSTITUTION_HISTORY: usize = 50;
        if proj.constitution_change_history.len() > MAX_CONSTITUTION_HISTORY {
            let excess = proj.constitution_change_history.len() - MAX_CONSTITUTION_HISTORY;
            proj.constitution_change_history.drain(0..excess);
        }
    }

    // === 中阶段完成检测与工作流推进 ===
    let mid_title_for_node_tag = proj
        .milestones
        .iter()
        .find(|m| m.id == milestone_id)
        .and_then(|ms| ms.mid_stages.iter().find(|m| m.id == mid_stage_id))
        .map(|mid| mid.title.clone())
        .unwrap_or_default();
    let mid_version_for_node_tag = plan_target_version.clone();
    let mid_stage_id_for_node_tag = mid_stage_id.clone();

    let (all_subtasks_passed, milestone_completed) =
        reconcile_terminal_stage(&mut proj, &milestone_id, &mid_stage_id)?;

    if all_subtasks_passed {
        if !mid_stage_id.is_empty() {
            write_execution_history_with_source(
                &mut proj,
                "success",
                project::ExecutionEventType::MidStageComplete,
                operation_source,
                format!(
                    "✅ 中阶段完成：{} (v{})",
                    mid_title_for_node_tag, mid_version_for_node_tag
                ),
                Some(&milestone_id),
                Some(&mid_stage_id),
                None,
            );
        }
        if milestone_completed {
            write_execution_history_with_source(
                &mut proj,
                "success",
                project::ExecutionEventType::AdvanceMilestoneReview,
                operation_source,
                format!("📋 推进到大阶段审阅：{}", milestone_title),
                Some(&milestone_id),
                None,
                None,
            );
        } else {
            write_execution_history_with_source(
                &mut proj,
                "success",
                project::ExecutionEventType::AdvanceNextMidStage,
                operation_source,
                "➡ 推进到下一中阶段选择".to_string(),
                Some(&milestone_id),
                None,
                None,
            );
        }
    }

    let (confirm_event, confirm_text) = confirmation_audit(operation_source, &subtask_title);
    write_execution_history_with_source(
        &mut proj,
        "success",
        confirm_event,
        operation_source,
        confirm_text,
        Some(&milestone_id),
        Some(&mid_stage_id),
        Some(&subtask_id),
    );
    write_execution_history_with_source(
        &mut proj,
        "success",
        project::ExecutionEventType::GitConfirmationCompleted,
        operation_source,
        format!("Git 确认事务完成：{}", transaction_id),
        Some(&milestone_id),
        Some(&mid_stage_id),
        Some(&subtask_id),
    );

    let autopilot_active = proj.workflow_state.autopilot_active;
    if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
        if autopilot.recovery_action == project::AutopilotRecoveryAction::RetryGitConfirmation {
            autopilot.recovery_action = project::AutopilotRecoveryAction::None;
            autopilot.error_message.clear();
            autopilot.last_action = format!("Git 确认完成：{}", subtask_title);
            autopilot.last_action_at = now.clone();
            if autopilot_active && autopilot.run_status == project::AutopilotRunStatus::ErrorStopped
            {
                autopilot.run_status = project::AutopilotRunStatus::Running;
            }
        }
    }

    // Clear execution session before saving (小阶段已确认)
    proj.execution_session = None;

    // ED Stop 处理：质量门禁、Git 标签和项目事实全部成功后检查
    let ed_stop_requested = proj
        .pause_context
        .as_ref()
        .map(|pc| pc.pending_action == "ed_stop_requested")
        .unwrap_or(false);
    if ed_stop_requested {
        let resume_step = proj.workflow_state.current_step.clone();
        let autopilot_was_active = proj.workflow_state.autopilot_active;
        proj.workflow_state.current_step = project::WorkflowStep::PauseDecision;
        proj.workflow_state.pause_reason = project::PauseReason::EDStop;
        if let Some(ref mut pc) = proj.pause_context {
            pc.resume_step = Some(resume_step);
            pc.autopilot_was_active = autopilot_was_active;
            pc.pending_action = String::new(); // 消费暂停请求
        }
        // 暂停自动驾驶
        if autopilot_was_active {
            if let Some(ref mut ap) = proj.workflow_state.autopilot_state {
                ap.run_status = project::AutopilotRunStatus::Paused;
                ap.last_action = "ED Stop：任务完成后暂停".to_string();
                ap.last_action_at = now.clone();
            }
        }
    }

    let mut proj = match crate::save_and_reload_project(&proj) {
        Ok(project) => project,
        Err(error) => {
            let message = format!("Git 标签已创建，但项目收口失败：{}", error);
            if let Ok(mut persisted) = crate::load_project(&project_name) {
                mark_confirmation_blocked_with_source(
                    &mut persisted,
                    project::GitConfirmationFailureKind::ProjectFinalizationFailed,
                    message.clone(),
                    operation_source,
                );
                let _ = crate::save_project(&persisted);
            }
            return Err(message);
        }
    };

    // === 中阶段节点 Git 标签（项目状态已持久化，标签为补充元数据） ===
    if all_subtasks_passed && !mid_stage_id_for_node_tag.is_empty() {
        match crate::git_ops::git_save_node(
            project_path.clone(),
            milestone_id.clone(),
            mid_stage_id.clone(),
            transaction_id.clone(),
            mid_version_for_node_tag,
            mid_title_for_node_tag,
        )
        .await
        {
            Ok(node_tag) => {
                // 更新中阶段的 git_tag 字段
                if let Err(e) = crate::git_ops::save_tag_to_mid_stage(
                    &project_name,
                    &mid_stage_id_for_node_tag,
                    &node_tag,
                ) {
                    eprintln!(
                        "[execution] 中阶段 git_tag 写入失败（项目状态已推进）：{}",
                        e
                    );
                } else {
                    proj = crate::load_project(&project_name)?;
                }
            }
            Err(e) => {
                eprintln!(
                    "[execution] 中阶段节点标签创建失败（项目状态已推进）：{}",
                    e
                );
            }
        }
    }

    // Clear pipeline state
    {
        let mut guard = pipeline_state.lock().await;
        if let Some(s) = guard.as_mut() {
            s.status = PipelineStatus::Idle;
            s.awaiting_confirmation = false;
            append_log(s, "success", format!("✅ 已确认: {}", subtask_title));
        }
    }

    Ok(proj)
}

/// 对结构化 Git 确认阻断执行同一事务的幂等续跑。
#[tauri::command]
pub(crate) async fn retry_git_confirmation(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    let proj = crate::load_project(&project_name)?;
    let session = proj
        .execution_session
        .as_ref()
        .filter(|session| {
            session.parsed_status() == project::ExecutionSessionStatus::ConfirmationBlocked
        })
        .ok_or("当前没有受阻的 Git 确认事务。".to_string())?;
    if !confirmation_failure_is_retryable(session.confirmation_failure_kind.as_ref()) {
        return Err(
            "当前 Git 确认阻断属于不可变标签或事务完整性问题，需要人工核对，禁止机械重试。"
                .to_string(),
        );
    }
    let updated =
        retry_git_confirmation_with_pipeline(&state.pipeline_state, project_name.clone()).await?;
    state
        .autopilot_runtime
        .start_if_active(state.pipeline_state.clone(), project_name)
        .await?;
    Ok(updated)
}

pub(crate) async fn retry_git_confirmation_with_pipeline(
    pipeline_state: &std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
    project_name: String,
) -> Result<project::Project, String> {
    retry_git_confirmation_with_source(pipeline_state, project_name, project::OperationSource::User)
        .await
}

pub(crate) async fn retry_git_confirmation_with_source(
    pipeline_state: &std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
    project_name: String,
    operation_source: project::OperationSource,
) -> Result<project::Project, String> {
    let proj = crate::load_project(&project_name)?;
    let session = proj
        .execution_session
        .as_ref()
        .filter(|session| {
            session.parsed_status() == project::ExecutionSessionStatus::ConfirmationBlocked
        })
        .ok_or("当前没有受阻的 Git 确认事务。".to_string())?;
    if !confirmation_failure_is_retryable(session.confirmation_failure_kind.as_ref()) {
        return Err(
            "当前 Git 确认阻断属于不可变标签或事务完整性问题，需要人工核对，禁止机械重试。"
                .to_string(),
        );
    }
    confirm_subtask_result_with_source(pipeline_state, project_name, operation_source).await
}

/// V1 驳回小阶段执行结果（用户点击"发现问题"）
#[tauri::command]
pub(crate) async fn reject_subtask_result(
    state: tauri::State<'_, AppState>,
    project_name: String,
    reason: String,
) -> Result<project::Project, String> {
    // 与确认共用认领：全程持流水线锁完成驳回，杜绝与自动确认并发。
    let mut guard = state.pipeline_state.lock().await;
    let mut proj = crate::load_project(&project_name)?;
    claim_awaiting_confirmation_under_lock(&mut proj, "rejecting")?;

    let (milestone_id, mid_stage_id, session_task_id) = proj
        .execution_session
        .as_ref()
        .map(|session| {
            (
                session.milestone_id.clone(),
                session.mid_stage_id.clone(),
                session.subtask_id.clone(),
            )
        })
        .ok_or_else(|| "没有活跃的执行会话。".to_string())?;

    let locate = (|| {
        let address = crate::task_tree::locate_task(&proj, &session_task_id)?
            .ok_or_else(|| "执行会话中的任务不存在。".to_string())?;
        let task = crate::task_tree::find_task(&proj, &session_task_id)?
            .ok_or_else(|| "执行会话中的任务不存在。".to_string())?;
        if !task.child_tasks.is_empty()
            || task.status != project::SubtaskStatus::AwaitingConfirmation
        {
            return Err("执行会话中的叶子任务未处于待驳回状态。".to_string());
        }
        Ok::<_, String>((address, task.id.clone(), task.title.clone()))
    })();

    let (_address, subtask_id, subtask_title) = match locate {
        Ok(v) => v,
        Err(msg) => {
            release_confirmation_claim(&mut proj, "awaiting_confirmation");
            let _ = crate::save_project(&proj);
            return Err(msg);
        }
    };

    let now = chrono::Utc::now().to_rfc3339();
    let st = crate::task_tree::find_task_mut(&mut proj, &subtask_id)?
        .ok_or_else(|| "执行会话中的任务不存在。".to_string())?;
    st.status = project::SubtaskStatus::Rejected;
    st.confirmed_by_user = Some(false);
    st.confirmed_at = Some(now.clone());
    st.confirmation_notes = Some(reason.clone());

    write_execution_history(
        &mut proj,
        "error",
        project::ExecutionEventType::UserReject,
        format!("❌ 用户驳回：{} — {}", subtask_title, reason),
        Some(&milestone_id),
        Some(&mid_stage_id),
        Some(&subtask_id),
    );

    if proj.workflow_state.autopilot_active {
        crate::recovery::begin_rejected_recovery(&mut proj, &reason)?;
    } else {
        proj.execution_session = None;
    }
    crate::save_project(&proj)?;

    if let Some(s) = guard.as_mut() {
        s.status = PipelineStatus::Idle;
        s.awaiting_confirmation = false;
        append_log(s, "error", format!("❌ 已驳回: {}", reason));
    }
    drop(guard);

    crate::load_project(&project_name)
}

/// V1 重试当前小阶段：先恢复基线并验证干净，成功后才清除失败会话并增加重试次数
#[tauri::command]
pub(crate) async fn retry_current_subtask(
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;
    let project_path = proj.project_path.clone();

    let milestone_id = proj
        .execution_session
        .as_ref()
        .map(|session| session.milestone_id.clone())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| proj.current_milestone_id.clone());
    let mid_stage_id = proj
        .execution_session
        .as_ref()
        .map(|session| session.mid_stage_id.clone())
        .unwrap_or_else(|| proj.current_mid_stage_id.clone());
    if milestone_id.is_empty() {
        return Err("当前恢复目标缺少大阶段身份。".to_string());
    }

    // 可由会话状态直接定位可恢复任务，不得依赖 retry_count > 0
    let recoverable_subtask_id = proj
        .execution_session
        .as_ref()
        .filter(|session| session.is_recoverable_failure())
        .map(|session| session.subtask_id.clone());

    let leaves = crate::task_tree::leaf_addresses_in_scope(&proj, &milestone_id, &mid_stage_id)?;
    let retry_address = leaves
        .iter()
        .find(|address| {
            crate::task_tree::find_task(&proj, &address.task_id)
                .ok()
                .flatten()
                .is_some_and(|task| {
                    matches!(
                        task.status,
                        project::SubtaskStatus::Rejected
                            | project::SubtaskStatus::AwaitingConfirmation
                    ) || (task.status == project::SubtaskStatus::Pending
                        && recoverable_subtask_id
                            .as_ref()
                            .is_some_and(|id| id == &task.id))
                        || (task.status == project::SubtaskStatus::Pending && task.retry_count > 0)
                })
        })
        .ok_or(
            "没有可重试的小阶段。只有测试失败、执行失败、人工驳回或恢复中断的任务可以重试。"
                .to_string(),
        )?;
    let subtask = crate::task_tree::find_task(&proj, &retry_address.task_id)?
        .ok_or_else(|| "可重试叶子任务不存在。".to_string())?;

    // 禁止重试已通过的任务
    if subtask.status == project::SubtaskStatus::Passed {
        return Err("已通过的小阶段不能重试，请使用回退流程。".to_string());
    }

    let subtask_id = subtask.id.clone();
    let subtask_title = subtask.title.clone();

    // 优先使用执行会话基线，其次最近通过标签，最后显式恢复当前 HEAD。
    // Git 恢复失败时保留失败会话、基线和错误证据。
    let session_base = proj.execution_session.as_ref().and_then(|session| {
        if session.base_commit.is_empty() {
            None
        } else {
            Some(session.base_commit.clone())
        }
    });
    let last_passed_tag = find_last_passed_subtask(&proj).and_then(|subtask| subtask.auto_tag);
    let restore_target = session_base
        .or(last_passed_tag)
        .unwrap_or_else(|| "HEAD".to_string());
    restore_git_execution_baseline(&project_path, &restore_target).map_err(|outcome| {
        format!(
            "Git 基线恢复失败：{}。失败证据已保留。",
            outcome.error_message()
        )
    })?;

    let now = chrono::Utc::now().to_rfc3339();

    // 基线恢复成功后才清理旧结果并递增重试次数（每次人工确认只 +1）
    let st = crate::task_tree::find_task_mut(&mut proj, &subtask_id)?
        .ok_or_else(|| "可重试叶子任务不存在。".to_string())?;
    let new_retry_count = st.retry_count.saturating_add(1);
    st.status = project::SubtaskStatus::Pending;
    st.execution_result = None;
    st.test_result = None;
    st.retry_count = new_retry_count;

    // 清除失败会话
    proj.execution_session = None;
    proj.workflow_state.recovery_state = None;

    // 记录重试事件
    write_execution_history(
        &mut proj,
        "info",
        project::ExecutionEventType::RetryScheduled,
        format!(
            "🔄 重试小阶段（第 {} 次）：{}",
            new_retry_count, subtask_title
        ),
        Some(&milestone_id),
        Some(&mid_stage_id),
        Some(&subtask_id),
    );

    // 如果自动驾驶处于 ErrorStopped，恢复为 Running 并清除恢复动作
    if proj.workflow_state.autopilot_active {
        if let Some(ref mut ap) = proj.workflow_state.autopilot_state {
            if ap.run_status == project::AutopilotRunStatus::ErrorStopped {
                ap.run_status = project::AutopilotRunStatus::Running;
                ap.last_action =
                    format!("重试小阶段（第 {} 次）：{}", new_retry_count, subtask_title);
                ap.last_action_at = now.clone();
                ap.error_message = String::new();
                ap.recovery_action = project::AutopilotRecoveryAction::None;
            }
        }
    }

    crate::save_and_reload_project(&proj).map_err(|e| format!("重试状态保存失败：{}", e))
}

// ===================================================================
// V1 执行工作区探测与准备
// ===================================================================

/// 探测项目路径的 Git 工作区是否满足执行前置条件（只读）
#[tauri::command]
pub(crate) async fn get_execution_workspace_status(
    project_name: String,
) -> Result<project::ExecutionWorkspaceStatus, String> {
    let proj = crate::load_project(&project_name)?;
    let path = &proj.project_path;

    if path.is_empty() {
        return Ok(project::ExecutionWorkspaceStatus {
            path_exists: false,
            is_directory: false,
            is_git_repo: false,
            has_commits: false,
            git_user_available: false,
            git_email_available: false,
            working_tree_clean: false,
            git_metadata_ready: false,
            ready_for_new_execution: false,
            has_managed_task_changes: false,
            has_external_changes: false,
            ready: false,
            status_message: "项目路径未设置。".to_string(),
            issues: vec![project::ExecutionWorkspaceIssue::PathMissing],
            changes: vec![],
        });
    }
    get_execution_workspace_status_for_project(&proj)
}

/// 准备执行工作区：在批准前或执行阶段由用户显式初始化 Git 并创建首次提交。
#[tauri::command]
pub(crate) async fn prepare_execution_workspace(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<project::ExecutionWorkspaceStatus, String> {
    let status = prepare_execution_workspace_inner(project_name.clone()).await?;
    state
        .autopilot_runtime
        .start_if_active(state.pipeline_state.clone(), project_name)
        .await?;
    Ok(status)
}

pub(crate) async fn prepare_execution_workspace_inner(
    project_name: String,
) -> Result<project::ExecutionWorkspaceStatus, String> {
    let mut proj = crate::load_project(&project_name)?;

    if !matches!(
        proj.workflow_state.current_step,
        project::WorkflowStep::PlanApproving | project::WorkflowStep::Execution
    ) {
        return Err(format!(
            "当前步骤为 {:?}，只有 PlanApproving 或 Execution 步骤可以准备执行工作区",
            proj.workflow_state.current_step
        ));
    }

    // Write execution history: user requested workspace preparation
    write_execution_history(
        &mut proj,
        "info",
        project::ExecutionEventType::WorkspacePrepare,
        "🔧 用户点击准备执行环境".to_string(),
        None,
        None,
        None,
    );
    crate::save_project(&proj)?;

    let path = proj.project_path.clone();
    if path.is_empty() {
        return Err("项目路径未设置。".to_string());
    }

    let path_std = std::path::Path::new(&path);
    if !path_std.exists() {
        return Err(format!("项目路径 {} 不存在。", path));
    }
    if !path_std.is_dir() {
        return Err(format!("项目路径 {} 不是目录。", path));
    }

    let git_path = path_std.join(".git");

    // Init git repo if needed
    if !git_path.exists() {
        let init = std::process::Command::new("git")
            .args(["init"])
            .current_dir(&path)
            .output()
            .map_err(|e| format!("git init 失败：{}", e))?;
        if !init.status.success() {
            let stderr = String::from_utf8_lossy(&init.stderr);
            return Err(format!(
                "git init 失败：{}",
                stderr.chars().take(200).collect::<String>()
            ));
        }
    }

    // Check git identity
    let user_name = std::process::Command::new("git")
        .args(["config", "user.name"])
        .current_dir(&path)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let user_email = std::process::Command::new("git")
        .args(["config", "user.email"])
        .current_dir(&path)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    if user_name.is_empty() || user_email.is_empty() {
        write_execution_history(
            &mut proj,
            "error",
            project::ExecutionEventType::WorkspacePrepareFailed,
            format!(
                "Git 身份未配置（user.name={:?}, user.email={:?}）",
                user_name, user_email
            ),
            None,
            None,
            None,
        );
        crate::save_project(&proj)?;
        return Err(format!(
            "Git 身份未配置（user.name={:?}, user.email={:?}）。请在项目目录下执行 git config user.name 和 git config user.email。",
            user_name, user_email
        ));
    }

    // Create initial commit if no commits exist
    let has_commits = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !has_commits {
        let add = std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&path)
            .output()
            .map_err(|e| format!("git add 失败：{}", e))?;
        if !add.status.success() {
            return Err(format!(
                "git add 失败：{}",
                String::from_utf8_lossy(&add.stderr).trim()
            ));
        }
        let commit = std::process::Command::new("git")
            .args([
                "commit",
                "--allow-empty",
                "-m",
                "初始提交（由 Metheus 自动创建）",
            ])
            .current_dir(&path)
            .output()
            .map_err(|e| format!("git commit 失败：{}", e))?;
        if !commit.status.success() {
            let stderr = String::from_utf8_lossy(&commit.stderr);
            if !stderr.contains("nothing to commit") {
                return Err(format!(
                    "git commit 失败：{}",
                    stderr.chars().take(200).collect::<String>()
                ));
            }
        }
    }

    let final_status = get_execution_workspace_status_for_project(&proj)?;
    if final_status.ready {
        write_execution_history(
            &mut proj,
            "success",
            project::ExecutionEventType::WorkspaceReady,
            "Git 工作区已就绪，可以执行小阶段。".to_string(),
            None,
            None,
            None,
        );
        if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
            if matches!(
                autopilot.recovery_action,
                project::AutopilotRecoveryAction::PrepareExecutionWorkspace
                    | project::AutopilotRecoveryAction::ResolveWorkspaceChanges
            ) {
                autopilot.recovery_action = project::AutopilotRecoveryAction::None;
                autopilot.error_message.clear();
                autopilot.last_action = "Git 工作区已准备完成".to_string();
                autopilot.last_action_at = chrono::Utc::now().to_rfc3339();
                if autopilot.run_status == project::AutopilotRunStatus::ErrorStopped {
                    autopilot.run_status = project::AutopilotRunStatus::Running;
                }
            }
        }
    } else {
        write_execution_history(
            &mut proj,
            "error",
            project::ExecutionEventType::WorkspacePrepareFailed,
            final_status.status_message.clone(),
            None,
            None,
            None,
        );
    }
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
    crate::save_project(&proj)?;

    Ok(final_status)
}

/// 用户在应用外处理完 Git 变更后只刷新事实，不执行 git init/add/commit。
#[tauri::command]
pub(crate) async fn refresh_execution_workspace(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<project::ExecutionWorkspaceStatus, String> {
    let status = refresh_execution_workspace_inner(project_name.clone()).await?;
    state
        .autopilot_runtime
        .start_if_active(state.pipeline_state.clone(), project_name)
        .await?;
    Ok(status)
}

pub(crate) async fn refresh_execution_workspace_inner(
    project_name: String,
) -> Result<project::ExecutionWorkspaceStatus, String> {
    let mut proj = crate::load_project(&project_name)?;
    let status = get_execution_workspace_status_for_project(&proj)?;
    if status.ready {
        let mut resumed = false;
        if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
            if autopilot.recovery_action
                == project::AutopilotRecoveryAction::ResolveWorkspaceChanges
            {
                autopilot.recovery_action = project::AutopilotRecoveryAction::None;
                autopilot.run_status = project::AutopilotRunStatus::Running;
                autopilot.error_message.clear();
                autopilot.last_action = "工作区状态已刷新，继续自动驾驶".to_string();
                autopilot.last_action_at = chrono::Utc::now().to_rfc3339();
                resumed = true;
            }
        }
        if resumed {
            proj.workflow_state.data_revision = proj.workflow_state.data_revision.saturating_add(1);
            proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
            crate::save_project(&proj)?;
        }
    }
    Ok(status)
}

/// Internal helper: probe workspace status from path
fn parse_workspace_changes(output: &[u8]) -> Vec<project::ExecutionWorkspaceChange> {
    let entries: Vec<&[u8]> = output
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .collect();
    let mut changes = Vec::new();
    let mut index = 0;
    while index < entries.len() {
        let entry = entries[index];
        if entry.len() < 3 {
            index += 1;
            continue;
        }
        let index_status = entry[0] as char;
        let worktree_status = entry[1] as char;
        let mut path = String::from_utf8_lossy(&entry[3..]).to_string();
        let is_rename = matches!(index_status, 'R' | 'C') || matches!(worktree_status, 'R' | 'C');
        if is_rename {
            if let Some(source) = entries.get(index + 1) {
                path = format!("{} -> {}", String::from_utf8_lossy(source), path);
            }
        }
        changes.push(project::ExecutionWorkspaceChange {
            path,
            index_status: index_status.to_string(),
            worktree_status: worktree_status.to_string(),
            tracked: index_status != '?' || worktree_status != '?',
            managed: false,
        });
        index += 1;
        if is_rename {
            // porcelain -z appends the source path as a second NUL-delimited field.
            index += 1;
        }
    }
    changes
}

pub(crate) fn get_execution_workspace_status_inner(
    path: &str,
) -> Result<project::ExecutionWorkspaceStatus, String> {
    let path_std = std::path::Path::new(path);
    let path_exists = path_std.exists();
    let is_directory = path_std.is_dir();

    if !path_exists || !is_directory {
        return Ok(project::ExecutionWorkspaceStatus {
            path_exists,
            is_directory,
            is_git_repo: false,
            has_commits: false,
            git_user_available: false,
            git_email_available: false,
            working_tree_clean: false,
            git_metadata_ready: false,
            ready_for_new_execution: false,
            has_managed_task_changes: false,
            has_external_changes: false,
            ready: false,
            status_message: if !path_exists {
                format!("项目路径 {} 不存在。", path)
            } else {
                format!("项目路径 {} 不是目录。", path)
            },
            issues: vec![if !path_exists {
                project::ExecutionWorkspaceIssue::PathMissing
            } else {
                project::ExecutionWorkspaceIssue::NotDirectory
            }],
            changes: vec![],
        });
    }

    let git_path = path_std.join(".git");
    let is_git_repo = git_path.exists();

    let has_commits = if is_git_repo {
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(path)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    } else {
        false
    };

    let git_user_available = std::process::Command::new("git")
        .args(["config", "user.name"])
        .current_dir(path)
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false);

    let git_email_available = std::process::Command::new("git")
        .args(["config", "user.email"])
        .current_dir(path)
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false);

    let changes = if is_git_repo {
        let status_output = std::process::Command::new("git")
            .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
            .current_dir(path)
            .output()
            .map_err(|error| format!("git status 失败：{}", error))?;
        if !status_output.status.success() {
            return Err(format!(
                "git status 失败：{}",
                String::from_utf8_lossy(&status_output.stderr).trim()
            ));
        }
        parse_workspace_changes(&status_output.stdout)
    } else {
        vec![]
    };
    let working_tree_clean = is_git_repo && changes.is_empty();

    let git_metadata_ready =
        is_git_repo && has_commits && git_user_available && git_email_available;
    let ready_for_new_execution = git_metadata_ready && working_tree_clean;

    let mut issues = Vec::new();
    if !is_git_repo {
        issues.push(project::ExecutionWorkspaceIssue::NotGitRepository);
    }
    if is_git_repo && !has_commits {
        issues.push(project::ExecutionWorkspaceIssue::NoCommits);
    }
    if !git_user_available {
        issues.push(project::ExecutionWorkspaceIssue::MissingGitUserName);
    }
    if !git_email_available {
        issues.push(project::ExecutionWorkspaceIssue::MissingGitUserEmail);
    }
    if is_git_repo && !working_tree_clean {
        issues.push(project::ExecutionWorkspaceIssue::DirtyWorkingTree);
    }

    let status_message = if ready_for_new_execution {
        "Git 工作区已就绪，可以执行小阶段。".to_string()
    } else {
        let mut missing = Vec::new();
        if issues.contains(&project::ExecutionWorkspaceIssue::NotGitRepository) {
            missing.push("Git 仓库未初始化");
        }
        if issues.contains(&project::ExecutionWorkspaceIssue::NoCommits) {
            missing.push("尚无首次提交");
        }
        if issues.contains(&project::ExecutionWorkspaceIssue::MissingGitUserName) {
            missing.push("Git user.name 未配置");
        }
        if issues.contains(&project::ExecutionWorkspaceIssue::MissingGitUserEmail) {
            missing.push("Git user.email 未配置");
        }
        if issues.contains(&project::ExecutionWorkspaceIssue::DirtyWorkingTree) {
            missing.push("工作区存在未提交或未跟踪修改");
        }
        format!("Git 工作区未就绪：{}。", missing.join("、"))
    };

    Ok(project::ExecutionWorkspaceStatus {
        path_exists,
        is_directory,
        is_git_repo,
        has_commits,
        git_user_available,
        git_email_available,
        working_tree_clean,
        git_metadata_ready,
        ready_for_new_execution,
        has_managed_task_changes: false,
        has_external_changes: !changes.is_empty(),
        ready: ready_for_new_execution,
        status_message,
        issues,
        changes,
    })
}

fn get_execution_workspace_status_for_project(
    proj: &project::Project,
) -> Result<project::ExecutionWorkspaceStatus, String> {
    let mut status = get_execution_workspace_status_inner(&proj.project_path)?;
    if !status.git_metadata_ready || status.changes.is_empty() {
        return Ok(status);
    }

    let managed_paths = proj.execution_session.as_ref().and_then(|session| {
        if !session.active || session.base_commit.is_empty() || session.subtask_id.is_empty() {
            return None;
        }
        let subtask = crate::task_tree::find_task(proj, &session.subtask_id)
            .ok()
            .flatten()?;
        Some(
            subtask
                .allowed_file_paths
                .iter()
                .chain(subtask.new_file_paths.iter())
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
        )
    });

    if let Some(managed_paths) = managed_paths {
        for change in &mut status.changes {
            change.managed = managed_paths.contains(&change.path);
        }
    }
    status.has_managed_task_changes = status.changes.iter().any(|change| change.managed);
    status.has_external_changes = status.changes.iter().any(|change| !change.managed);
    status.status_message = if status.has_external_changes && status.has_managed_task_changes {
        "当前任务有待确认的代码变更，同时存在任务范围外改动。".to_string()
    } else if status.has_external_changes {
        "Git 工作区包含当前任务范围外的未提交或未跟踪修改。".to_string()
    } else if status.has_managed_task_changes {
        "当前任务有待确认的代码变更。".to_string()
    } else {
        status.status_message
    };
    Ok(status)
}

// ===================================================================
// V1 暂停与回退命令
// ===================================================================

/// Structured baseline restore facts. Never includes stash contents or secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BaselineRestoreOutcome {
    pub status: project::RecoveryBaselineStatus,
    pub target_summary: String,
    pub stash_created: bool,
    pub error: Option<String>,
}

impl BaselineRestoreOutcome {
    pub(crate) fn is_restored(&self) -> bool {
        self.status == project::RecoveryBaselineStatus::Restored
    }

    pub(crate) fn error_message(&self) -> String {
        self.error
            .clone()
            .unwrap_or_else(|| "基线恢复失败".to_string())
    }

    pub(crate) fn into_unit_result(self) -> Result<(), String> {
        if self.is_restored() {
            Ok(())
        } else {
            Err(self.error_message())
        }
    }
}

pub(crate) fn summarize_baseline_target(target: &str) -> String {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return "HEAD".to_string();
    }
    let looks_like_full_sha =
        trimmed.len() >= 40 && trimmed.chars().all(|ch| ch.is_ascii_hexdigit());
    if looks_like_full_sha {
        trimmed.chars().take(12).collect()
    } else {
        trimmed.chars().take(64).collect()
    }
}

pub(crate) fn apply_baseline_restore_outcome(
    recovery: &mut project::RecoveryState,
    outcome: &BaselineRestoreOutcome,
) {
    recovery.baseline_status = outcome.status.clone();
    recovery.baseline_target_summary = outcome.target_summary.clone();
    recovery.baseline_stash_created = outcome.stash_created;
}

fn baseline_failed(
    target_summary: &str,
    stash_created: bool,
    error: String,
) -> BaselineRestoreOutcome {
    BaselineRestoreOutcome {
        status: project::RecoveryBaselineStatus::RestoreFailed,
        target_summary: target_summary.to_string(),
        stash_created,
        error: Some(error),
    }
}

/// Restore execution baseline with stash-include-untracked then reset --hard.
/// Returns structured Restored/RestoreFailed facts; never returns stash contents.
pub(crate) fn restore_git_execution_baseline(
    project_path: &str,
    target: &str,
) -> Result<BaselineRestoreOutcome, BaselineRestoreOutcome> {
    let target_summary = summarize_baseline_target(target);
    let status_output = match std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(project_path)
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return Err(baseline_failed(
                &target_summary,
                false,
                format!("git status 失败：{}", error),
            ));
        }
    };
    if !status_output.status.success() {
        return Err(baseline_failed(
            &target_summary,
            false,
            format!(
                "git status 失败：{}",
                String::from_utf8_lossy(&status_output.stderr).trim()
            ),
        ));
    }
    let has_changes = !String::from_utf8_lossy(&status_output.stdout)
        .trim()
        .is_empty();
    let mut stash_created = false;
    if has_changes {
        let stash_output = match std::process::Command::new("git")
            .args([
                "stash",
                "push",
                "--include-untracked",
                "-m",
                "metheus_execution_safety_stash",
            ])
            .current_dir(project_path)
            .output()
        {
            Ok(output) => output,
            Err(error) => {
                return Err(baseline_failed(
                    &target_summary,
                    false,
                    format!("git stash 失败：{}", error),
                ));
            }
        };
        if !stash_output.status.success() {
            return Err(baseline_failed(
                &target_summary,
                false,
                format!(
                    "git stash 失败：{}",
                    String::from_utf8_lossy(&stash_output.stderr).trim()
                ),
            ));
        }
        stash_created = true;
    }

    let reset_output = match std::process::Command::new("git")
        .args(["reset", "--hard", target])
        .current_dir(project_path)
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return Err(baseline_failed(
                &target_summary,
                stash_created,
                format!("git reset --hard {} 失败：{}", target_summary, error),
            ));
        }
    };
    if !reset_output.status.success() {
        let reset_error = String::from_utf8_lossy(&reset_output.stderr)
            .trim()
            .to_string();
        if stash_created {
            let pop_output = match std::process::Command::new("git")
                .args(["stash", "pop"])
                .current_dir(project_path)
                .output()
            {
                Ok(output) => output,
                Err(error) => {
                    return Err(baseline_failed(
                        &target_summary,
                        stash_created,
                        format!(
                            "回退到 {} 失败：{}；恢复安全暂存也失败：{}",
                            target_summary, reset_error, error
                        ),
                    ));
                }
            };
            if !pop_output.status.success() {
                return Err(baseline_failed(
                    &target_summary,
                    stash_created,
                    format!(
                        "回退到 {} 失败：{}；恢复安全暂存也失败：{}",
                        target_summary,
                        reset_error,
                        String::from_utf8_lossy(&pop_output.stderr).trim()
                    ),
                ));
            }
        }
        return Err(baseline_failed(
            &target_summary,
            stash_created,
            format!("回退到 {} 失败：{}", target_summary, reset_error),
        ));
    }

    let workspace = match get_execution_workspace_status_inner(project_path) {
        Ok(workspace) => workspace,
        Err(error) => {
            return Err(baseline_failed(&target_summary, stash_created, error));
        }
    };
    if !workspace.working_tree_clean {
        return Err(baseline_failed(
            &target_summary,
            stash_created,
            "Git 回退后工作区仍有残留修改，安全基线验证失败。".to_string(),
        ));
    }
    Ok(BaselineRestoreOutcome {
        status: project::RecoveryBaselineStatus::Restored,
        target_summary,
        stash_created,
        error: None,
    })
}

fn recoverable_execution_session(
    proj: &project::Project,
) -> Result<project::ExecutionSession, String> {
    if proj.execution_session.as_ref().is_some_and(|session| {
        session.parsed_status() == project::ExecutionSessionStatus::ConfirmationBlocked
    }) {
        return Err("当前是 Git 确认受阻；请使用“重新确认提交”，不得恢复执行基线。".to_string());
    }
    proj.execution_session
        .as_ref()
        .filter(|session| {
            matches!(
                session.parsed_status(),
                project::ExecutionSessionStatus::SessionLost
                    | project::ExecutionSessionStatus::StopFailed
                    | project::ExecutionSessionStatus::ExecutionFailed
            )
        })
        .cloned()
        .ok_or("当前没有需要恢复的执行失败会话。".to_string())
}

fn execution_restore_target(
    proj: &project::Project,
    session: &project::ExecutionSession,
) -> String {
    if session.base_commit.is_empty() {
        find_last_passed_subtask(proj)
            .and_then(|subtask| subtask.auto_tag)
            .unwrap_or_else(|| "HEAD".to_string())
    } else {
        session.base_commit.clone()
    }
}

fn execution_recovery_context(
    proj: &project::Project,
    action: &str,
) -> Result<(project::ExecutionSession, String), String> {
    if action == "acknowledge_execution_recovery" {
        let session = recoverable_execution_session(proj)?;
        let target = execution_restore_target(proj, &session);
        return Ok((session, target));
    }
    if !matches!(action, "restore_and_retry" | "skip_task") {
        return Err(format!("不支持预览恢复动作：{action}"));
    }
    let recovery = proj
        .workflow_state
        .recovery_state
        .as_ref()
        .ok_or_else(|| "当前没有可预览的人工恢复状态。".to_string())?;
    let session = proj
        .execution_session
        .as_ref()
        .cloned()
        .ok_or_else(|| "当前没有可预览的执行会话。".to_string())?;
    let target = if !recovery.baseline_commit.is_empty() {
        recovery.baseline_commit.clone()
    } else if !session.base_commit.is_empty() {
        session.base_commit.clone()
    } else {
        "HEAD".to_string()
    };
    Ok((session, target))
}

fn preview_execution_recovery_impact_for_project(
    proj: &project::Project,
    action: &str,
) -> Result<project::ExecutionRecoveryImpact, String> {
    let (session, restore_target) = execution_recovery_context(proj, action)?;
    let commit_impact = crate::git_ops::preview_reset_impact(&proj.project_path, &restore_target)?;
    let mut workspace = get_execution_workspace_status_inner(&proj.project_path)?;
    let managed_paths = crate::task_tree::find_task(&proj, &session.subtask_id)?
        .map(|subtask| {
            subtask
                .allowed_file_paths
                .iter()
                .chain(subtask.new_file_paths.iter())
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        })
        .unwrap_or_default();
    for change in &mut workspace.changes {
        change.managed = managed_paths.contains(&change.path)
            || change
                .path
                .split(" -> ")
                .any(|path| managed_paths.contains(path));
    }
    workspace.changes.sort_by(|left, right| {
        (
            left.path.as_str(),
            left.index_status.as_str(),
            left.worktree_status.as_str(),
            left.tracked,
            left.managed,
        )
            .cmp(&(
                right.path.as_str(),
                right.index_status.as_str(),
                right.worktree_status.as_str(),
                right.tracked,
                right.managed,
            ))
    });

    let mut affected = commit_impact.changed_since_target.clone();
    affected.extend(workspace.changes.iter().map(|change| change.path.clone()));
    affected.sort();
    affected.dedup();
    let mut untracked_files = workspace
        .changes
        .iter()
        .filter(|change| !change.tracked)
        .map(|change| change.path.clone())
        .collect::<Vec<_>>();
    let mut managed_changes = workspace
        .changes
        .iter()
        .filter(|change| change.managed)
        .map(|change| change.path.clone())
        .collect::<Vec<_>>();
    let mut external_changes = workspace
        .changes
        .iter()
        .filter(|change| !change.managed)
        .map(|change| change.path.clone())
        .collect::<Vec<_>>();
    for values in [
        &mut untracked_files,
        &mut managed_changes,
        &mut external_changes,
    ] {
        values.sort();
        values.dedup();
    }
    let has_workspace_changes = !workspace.changes.is_empty();
    let has_destructive_changes =
        !affected.is_empty() || commit_impact.current_head != commit_impact.target_commit;
    let workspace_fingerprint =
        crate::git_ops::recovery_workspace_fingerprint(&proj.project_path, &untracked_files)?;
    let recovery_fingerprint =
        crate::recovery_presentation::present_recovery(proj).state_fingerprint;
    let mut fingerprint_input = format!(
        "{}\n{}\n{}\n{}\n{}\n",
        recovery_fingerprint,
        action,
        commit_impact.target_commit,
        commit_impact.current_head,
        format!(
            "{}\n{}",
            commit_impact.changed_since_target.join("\0"),
            workspace_fingerprint,
        ),
    );
    for change in &workspace.changes {
        fingerprint_input.push_str(&format!(
            "{}\0{}\0{}\0{}\0{}\n",
            change.path,
            change.index_status,
            change.worktree_status,
            change.tracked,
            change.managed,
        ));
    }
    let state_fingerprint = format!("sha256:{:x}", Sha256::digest(fingerprint_input.as_bytes()));
    let action_label = match action {
        "skip_task" => "跳过当前任务",
        "restore_and_retry" => "恢复基线并重试",
        _ => "恢复执行基线",
    }
    .to_string();
    let confirmation_title = format!("确认{}", action_label);
    let presentation_description =
        "恢复会从工作区移除下列内容；未提交内容会先写入安全暂存。".to_string();
    let safety_stash_summary = if has_workspace_changes {
        "未提交及未跟踪内容会先进入 Metheus 安全暂存，然后从当前工作区移除。".to_string()
    } else {
        "工作区没有未提交内容。".to_string()
    };
    Ok(project::ExecutionRecoveryImpact {
        action_label,
        confirmation_title,
        presentation_description,
        safety_stash_summary,
        baseline_commit: commit_impact.target_commit,
        current_head: commit_impact.current_head,
        affected_files: affected.clone(),
        untracked_files,
        managed_changes,
        external_changes,
        discarded_files: affected,
        creates_safety_stash: has_workspace_changes,
        has_destructive_changes,
        state_fingerprint,
    })
}

pub(crate) fn preview_execution_recovery_impact_inner(
    project_name: &str,
) -> Result<project::ExecutionRecoveryImpact, String> {
    let proj = crate::load_project(project_name)?;
    preview_execution_recovery_impact_for_project(&proj, "acknowledge_execution_recovery")
}

pub(crate) fn verify_execution_recovery_preview(
    proj: &project::Project,
    action: &str,
    expected_state_fingerprint: Option<&str>,
) -> Result<project::ExecutionRecoveryImpact, String> {
    let expected = expected_state_fingerprint
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "恢复前必须先取得最新影响预览。".to_string())?;
    let current = preview_execution_recovery_impact_for_project(proj, action)?;
    if current.state_fingerprint != expected {
        return Err(
            "恢复影响预览已过期：Git HEAD、工作区内容或恢复状态已经变化，请重新预览。".to_string(),
        );
    }
    Ok(current)
}

#[tauri::command]
pub(crate) async fn preview_execution_recovery_impact(
    state: tauri::State<'_, AppState>,
    project_name: String,
    action: Option<String>,
) -> Result<project::ExecutionRecoveryImpact, String> {
    let _pipeline_guard = state.pipeline_state.lock().await;
    let proj = crate::load_project(&project_name)?;
    preview_execution_recovery_impact_for_project(
        &proj,
        action
            .as_deref()
            .unwrap_or("acknowledge_execution_recovery"),
    )
}

#[cfg(unix)]
fn unix_process_is_running(pid: u32) -> Result<bool, String> {
    let output = std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map_err(|error| format!("检查进程 {} 状态失败：{}", pid, error))?;
    if !output.status.success() {
        return Ok(false);
    }

    // `kill -0` 对尚未被父进程回收的僵尸进程仍返回成功，但僵尸进程已经退出。
    let process_state = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .map_err(|error| format!("读取进程 {} 状态失败：{}", pid, error))?;
    if !process_state.status.success() {
        return Ok(false);
    }
    let state = String::from_utf8_lossy(&process_state.stdout);
    Ok(!state.trim_start().starts_with('Z'))
}

#[cfg(unix)]
async fn terminate_execution_process(pid: u32) -> Result<(), String> {
    if !unix_process_is_running(pid)? {
        return Ok(());
    }
    let terminate = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .output()
        .map_err(|error| format!("终止进程 {} 失败：{}", pid, error))?;
    if !terminate.status.success() && unix_process_is_running(pid)? {
        return Err(format!(
            "终止进程 {} 失败：{}",
            pid,
            String::from_utf8_lossy(&terminate.stderr).trim()
        ));
    }

    let graceful_deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < graceful_deadline {
        if !unix_process_is_running(pid)? {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let force = std::process::Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .output()
        .map_err(|error| format!("强制终止进程 {} 失败：{}", pid, error))?;
    if !force.status.success() && unix_process_is_running(pid)? {
        return Err(format!(
            "强制终止进程 {} 失败：{}",
            pid,
            String::from_utf8_lossy(&force.stderr).trim()
        ));
    }
    let force_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < force_deadline {
        if !unix_process_is_running(pid)? {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Err(format!("进程 {} 在终止期限内未退出。", pid))
}

#[cfg(not(unix))]
async fn terminate_execution_process(pid: u32) -> Result<(), String> {
    let output = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .output()
        .map_err(|error| format!("终止进程 {} 失败：{}", pid, error))?;
    if !output.status.success() {
        return Err(format!(
            "终止进程 {} 失败：{}",
            pid,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

async fn persist_in_stop_failure(
    pipeline_state: &std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
    proj: &mut project::Project,
    error: &str,
) -> String {
    {
        let mut guard = pipeline_state.lock().await;
        if let Some(pipeline) = guard.as_mut() {
            pipeline.status = PipelineStatus::Failed;
            pipeline.last_error = Some(error.to_string());
            append_log(pipeline, "error", format!("In Stop 失败：{}", error));
        }
    }
    let now = chrono::Utc::now().to_rfc3339();
    let truncated: String = error.chars().take(2048).collect();
    if let Some(session) = proj.execution_session.as_mut() {
        session.active = false;
        session.status = "stop_failed".to_string();
        session.failure_message = truncated.clone();
        session.state_entered_at = now.clone();
    }
    if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
        autopilot.run_status = project::AutopilotRunStatus::ErrorStopped;
        autopilot.last_action = format!("In Stop 失败：{}", error);
        autopilot.last_action_at = now;
        autopilot.error_message = truncated;
        autopilot.recovery_action = project::AutopilotRecoveryAction::RestoreExecutionBaseline;
    }
    let milestone_id = proj.current_milestone_id.clone();
    let mid_stage_id = proj.current_mid_stage_id.clone();
    let subtask_id = proj
        .execution_session
        .as_ref()
        .map(|session| session.subtask_id.clone());
    write_execution_history(
        proj,
        "error",
        project::ExecutionEventType::UserInStop,
        format!("In Stop 失败：{}", error),
        Some(&milestone_id),
        Some(&mid_stage_id),
        subtask_id.as_deref(),
    );
    match crate::save_project(proj) {
        Ok(()) => error.to_string(),
        Err(save_error) => format!("{}；阻断状态保存失败：{}", error, save_error),
    }
}

/// V1 In Stop：立即终止当前子进程，回到上一个稳定检查点
#[tauri::command]
/// 统一 In Stop 逻辑：杀进程 + 等退出 + Git 回退 + 修状态。
/// 供 `request_in_stop` 和 `autopilot_pause` 共用。
pub(crate) async fn perform_in_stop(
    state: &tauri::State<'_, AppState>,
    proj: &mut project::Project,
) -> Result<(), String> {
    perform_in_stop_with_pipeline_state(state.pipeline_state.clone(), proj).await
}

/// In Stop implementation that accepts the shared pipeline state directly.
/// This keeps the command wrapper thin and makes the stop contract testable
/// without constructing a Tauri runtime state.
pub(crate) async fn perform_in_stop_with_pipeline_state(
    pipeline_state: std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
    proj: &mut project::Project,
) -> Result<(), String> {
    let current_attempt = find_current_subtask(proj);
    let last_passed = find_last_passed_subtask(proj);
    let execution_id = proj
        .execution_session
        .as_ref()
        .filter(|session| session.active && session.status == "executing")
        .map(|session| session.execution_id.clone())
        .filter(|id| !id.is_empty())
        .ok_or("当前没有真实执行中的小阶段，无法请求 In Stop。")?;

    // 1. 先标记受控暂停，让后台任务停止写入，再等待子进程 PID 出现。
    {
        let mut guard = pipeline_state.lock().await;
        let pipeline = guard
            .as_mut()
            .filter(|pipeline| {
                pipeline.execution_id == execution_id && pipeline.status == PipelineStatus::Running
            })
            .ok_or("内存执行状态与项目会话不一致，无法安全暂停。")?;
        pipeline.status = PipelineStatus::Paused;
        append_log(pipeline, "pause", "⏹ In Stop：正在受控暂停".to_string());
    }

    let pid_deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let child_pid = loop {
        let pid = {
            let mut guard = pipeline_state.lock().await;
            guard.as_mut().and_then(|pipeline| {
                if pipeline.execution_id == execution_id {
                    pipeline.child_pid.take()
                } else {
                    None
                }
            })
        };
        if pid.is_some() || std::time::Instant::now() >= pid_deadline {
            break pid;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };

    // 2. 终止子进程并确认已退出。
    if let Some(pid) = child_pid {
        if let Err(error) = terminate_execution_process(pid).await {
            let persisted_error = persist_in_stop_failure(&pipeline_state, proj, &error).await;
            return Err(persisted_error);
        }
    }

    // 3. 回退到会话基线，其次最近通过标签，最后显式使用当前 HEAD。
    let base_commit = proj.execution_session.as_ref().and_then(|s| {
        if s.base_commit.is_empty() {
            None
        } else {
            Some(s.base_commit.clone())
        }
    });
    let restore_target = if let Some(commit) = base_commit {
        commit
    } else {
        let last_passed = find_last_passed_subtask(proj);
        last_passed
            .and_then(|last| last.auto_tag)
            .unwrap_or_else(|| "HEAD".to_string())
    };
    if let Err(outcome) = restore_git_execution_baseline(&proj.project_path, &restore_target) {
        let error = outcome.error_message();
        let persisted_error = persist_in_stop_failure(&pipeline_state, proj, &error).await;
        return Err(persisted_error);
    }

    // 4. 只有进程退出且 Git 基线验证通过后，才进入暂停决策。
    let now = chrono::Utc::now().to_rfc3339();
    let active_task_id = proj
        .execution_session
        .as_ref()
        .map(|session| session.subtask_id.clone())
        .filter(|id| !id.is_empty());
    if let Some(task_id) = active_task_id {
        if let Some(st) = crate::task_tree::find_task_mut(proj, &task_id)? {
            if st.status == project::SubtaskStatus::Executing
                || st.status == project::SubtaskStatus::AwaitingConfirmation
            {
                st.status = project::SubtaskStatus::Pending;
                st.execution_result = None;
                st.test_result = None;
            }
        }
    }
    proj.execution_session = None;

    proj.workflow_state.current_step = project::WorkflowStep::PauseDecision;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now.clone();
    proj.pause_context = Some(project::PauseContext {
        pause_type: "in_stop".to_string(),
        current_subtask_id: current_attempt
            .as_ref()
            .map(|subtask| subtask.id.clone())
            .unwrap_or_default(),
        last_passed_subtask_id: last_passed
            .as_ref()
            .map(|subtask| subtask.id.clone())
            .unwrap_or_default(),
        stable_tag: last_passed
            .as_ref()
            .and_then(|subtask| subtask.auto_tag.clone())
            .unwrap_or_default(),
        paused_at: now,
        discussion_start_revision: proj.discussion_revision,
        pending_action: String::new(),
        resume_step: None,
        autopilot_was_active: proj.workflow_state.autopilot_active,
    });

    Ok(())
}

#[tauri::command]
pub(crate) async fn request_in_stop(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    request_in_stop_with_pipeline_state(state.pipeline_state.clone(), &mut proj).await?;

    crate::save_and_reload_project(&proj)
}

/// Complete In Stop transition and append its user-facing history entry.
/// Persistence remains the command wrapper's responsibility.
pub(crate) async fn request_in_stop_with_pipeline_state(
    pipeline_state: std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
    proj: &mut project::Project,
) -> Result<(), String> {
    // Find current subtask for history/logging
    let current_attempt = find_current_subtask(proj);
    let last_passed = find_last_passed_subtask(proj);

    // Delegate to unified stop logic
    perform_in_stop_with_pipeline_state(pipeline_state, proj).await?;

    // Save PauseContext
    let now = chrono::Utc::now().to_rfc3339();
    proj.pause_context = Some(project::PauseContext {
        pause_type: "in_stop".to_string(),
        current_subtask_id: current_attempt
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_default(),
        last_passed_subtask_id: last_passed
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_default(),
        stable_tag: last_passed
            .as_ref()
            .and_then(|s| s.auto_tag.clone())
            .unwrap_or_default(),
        paused_at: now.clone(),
        discussion_start_revision: proj.discussion_revision,
        pending_action: String::new(),
        resume_step: None,
        autopilot_was_active: proj.workflow_state.autopilot_active,
    });

    // Write execution history
    let history_milestone_id = current_attempt
        .as_ref()
        .map(|_| proj.current_milestone_id.clone());
    let history_mid_stage_id = proj.current_mid_stage_id.clone();
    let history_subtask_id = current_attempt.as_ref().map(|s| s.id.clone());
    write_execution_history(
        proj,
        "pause",
        project::ExecutionEventType::UserInStop,
        "⏹ 用户请求立即暂停 (In Stop)".to_string(),
        history_milestone_id.as_deref(),
        Some(&history_mid_stage_id),
        history_subtask_id.as_deref(),
    );

    Ok(())
}

/// V1 ED Stop：先取得流水线互斥权，再加载最新项目，在同一互斥周期内写盘后返回。
#[tauri::command]
pub(crate) async fn request_ed_stop(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    let pipeline_state = state.pipeline_state.clone();
    let mut pipeline_guard = pipeline_state.lock().await;
    let mut proj = crate::load_project(&project_name)?;

    request_ed_stop_under_lock(&mut pipeline_guard, &mut proj)?;
    crate::save_project(&proj)?;
    drop(pipeline_guard);
    crate::load_project(&project_name)
}

/// 测试与内部入口：取得流水线互斥权后，再加载/修改调用方提供的项目事实。
/// 注意：生产路径由 `request_ed_stop` 在锁内加载最新磁盘项目；本函数假定调用方已在锁内
/// 持有最新事实，或仅用于单线程测试。
#[cfg(test)]
pub(crate) async fn request_ed_stop_with_pipeline_state(
    pipeline_state: std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
    proj: &mut project::Project,
) -> Result<(), String> {
    let mut pipeline_guard = pipeline_state.lock().await;
    request_ed_stop_under_lock(&mut pipeline_guard, proj)
}

/// 在调用方已经取得流水线互斥权后修改最新项目事实（不自行取锁、不自行保存）。
/// 暂停请求写入失败时由调用方决定是否保存；本函数失败时不得只保留内存日志。
fn request_ed_stop_under_lock(
    pipeline_guard: &mut Option<PipelineState>,
    proj: &mut project::Project,
) -> Result<(), String> {
    // 重复请求是幂等操作，必须在修改日志和历史之前返回。
    if proj
        .pause_context
        .as_ref()
        .map(|pc| pc.pending_action.as_str())
        == Some("ed_stop_requested")
    {
        return Ok(());
    }

    let execution_id = proj
        .execution_session
        .as_ref()
        .filter(|session| session.active && session.status == "executing")
        .map(|session| session.execution_id.clone())
        .filter(|id| !id.is_empty())
        .ok_or("只有小阶段真实执行中才能请求完成后暂停。")?;

    let pipeline = match pipeline_guard.as_mut() {
        Some(pipeline)
            if pipeline.execution_id == execution_id
                && pipeline.status == PipelineStatus::Running =>
        {
            pipeline
        }
        Some(pipeline)
            if pipeline.execution_id == execution_id
                && (pipeline.status == PipelineStatus::Paused
                    || pipeline.status == PipelineStatus::Completed
                    || pipeline.awaiting_confirmation) =>
        {
            return Err("任务已经完成，无法登记完成后暂停".to_string());
        }
        _ => {
            return Err("内存执行状态与项目会话不一致，无法请求完成后暂停。".to_string());
        }
    };
    append_log(
        pipeline,
        "pause",
        "⏸ ED Stop：当前任务完成后将暂停".to_string(),
    );

    let now = chrono::Utc::now().to_rfc3339();
    let current = find_current_subtask(proj);
    proj.pause_context = Some(project::PauseContext {
        pause_type: "ed_stop".to_string(),
        current_subtask_id: current.as_ref().map(|s| s.id.clone()).unwrap_or_default(),
        last_passed_subtask_id: String::new(),
        stable_tag: String::new(),
        paused_at: now.clone(),
        discussion_start_revision: proj.discussion_revision,
        pending_action: "ed_stop_requested".to_string(),
        resume_step: None,
        autopilot_was_active: proj.workflow_state.autopilot_active,
    });

    proj.workflow_state.pause_reason = project::PauseReason::EDStop;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    let milestone_id = proj.current_milestone_id.clone();
    let mid_stage_id = proj.current_mid_stage_id.clone();
    write_execution_history(
        proj,
        "pause",
        project::ExecutionEventType::UserEdStop,
        "⏸ 用户请求完成后暂停 (ED Stop)".to_string(),
        Some(&milestone_id),
        Some(&mid_stage_id),
        None,
    );

    Ok(())
}

/// V1 暂停决策：继续 / 调整 / 回退
#[tauri::command]
pub(crate) async fn resolve_pause_decision(
    project_name: String,
    action: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    if proj.workflow_state.current_step != project::WorkflowStep::PauseDecision {
        return Err(format!(
            "当前步骤 {:?} 不是 PauseDecision",
            proj.workflow_state.current_step
        ));
    }

    match action.as_str() {
        "continue" => {
            // 读取 resume_step：ED Stop 保存了后续步骤，In Stop 默认回 Execution
            let resume_step = proj
                .pause_context
                .as_ref()
                .and_then(|pc| pc.resume_step.clone())
                .unwrap_or(project::WorkflowStep::Execution);
            proj.workflow_state.current_step = resume_step;
            proj.workflow_state.pause_reason = project::PauseReason::None;

            // 恢复自动驾驶（如果暂停时活跃）
            let autopilot_was_active = proj
                .pause_context
                .as_ref()
                .map(|pc| pc.autopilot_was_active)
                .unwrap_or(false);
            proj.pause_context = None;

            if autopilot_was_active && proj.workflow_state.autopilot_active {
                if let Some(ref mut ap) = proj.workflow_state.autopilot_state {
                    if ap.run_status == project::AutopilotRunStatus::Paused {
                        ap.run_status = project::AutopilotRunStatus::Running;
                        ap.last_action = "暂停决策选择继续，自动驾驶已恢复".to_string();
                        ap.last_action_at = chrono::Utc::now().to_rfc3339();
                    }
                }
            }

            write_execution_history(
                &mut proj,
                "info",
                project::ExecutionEventType::UserContinue,
                "▶ 用户选择继续执行".to_string(),
                None,
                None,
                None,
            );
        }
        "adjust" => {
            // Enter Discussion with PauseAdjustment scope
            proj.workflow_state.current_step = project::WorkflowStep::Discussion;
            proj.workflow_state.discussion_scope = project::DiscussionScope::PauseAdjustment;
            let milestone_id = proj.current_milestone_id.clone();
            let review_cycle_id =
                format!("pause:{}:{}", milestone_id, chrono::Utc::now().to_rfc3339());
            proj.activate_discussion_thread(
                project::DiscussionScope::PauseAdjustment,
                &milestone_id,
                &review_cycle_id,
            );
            // Keep pause_context for reference
            write_execution_history(
                &mut proj,
                "info",
                project::ExecutionEventType::UserAdjust,
                "🔧 用户选择调整后续方案".to_string(),
                None,
                None,
                None,
            );
        }
        "rollback" => {
            // Enter RollbackPreview
            proj.workflow_state.current_step = project::WorkflowStep::RollbackPreview;
            write_execution_history(
                &mut proj,
                "pause",
                project::ExecutionEventType::UserRollback,
                "↩ 用户选择回退到更早稳定点".to_string(),
                None,
                None,
                None,
            );
        }
        _ => return Err(format!("未知暂停动作：{}", action)),
    }

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

/// V1 预览回退影响范围
#[tauri::command]
pub(crate) async fn preview_rollback_impact(
    project_name: String,
    checkpoint_subtask_id: String,
) -> Result<project::RollbackImpact, String> {
    let proj = crate::load_project(&project_name)?;

    // Collect all subtasks across all mid-stages
    let mut all_subtasks: Vec<(&str, &str, &project::Subtask)> = Vec::new();
    for ms in &proj.milestones {
        for mid in &ms.mid_stages {
            for st in &mid.subtasks {
                all_subtasks.push((ms.id.as_str(), mid.id.as_str(), st));
            }
        }
    }

    // Find checkpoint position
    let cp_idx = all_subtasks
        .iter()
        .position(|(_, _, st)| st.id == checkpoint_subtask_id)
        .ok_or("未找到检查点小阶段".to_string())?;

    let retained: Vec<String> = all_subtasks[..=cp_idx]
        .iter()
        .map(|(_, _, st)| st.title.clone())
        .collect();
    let discarded: Vec<String> = all_subtasks[cp_idx + 1..]
        .iter()
        .map(|(_, _, st)| st.title.clone())
        .collect();
    let target_tag = all_subtasks[cp_idx]
        .2
        .auto_tag
        .clone()
        .unwrap_or_else(|| "无标签（代码将回退到该检查点的 Git 提交）".to_string());

    Ok(project::RollbackImpact {
        target_checkpoint: format!("{} (tag: {})", all_subtasks[cp_idx].2.title, target_tag),
        retained_nodes: retained,
        discarded_nodes: discarded,
        // 不可变标签是审计事实；回退只移动工作树和项目引用，不删除任何标签。
        deleted_tags: Vec::new(),
        regeneration_scope: format!("从「{}」之后重新生成执行计划", all_subtasks[cp_idx].2.title),
        includes_code_rollback: true,
    })
}

/// V1 确认回退：执行 Git 回退并更新项目数据
#[tauri::command]
pub(crate) async fn confirm_rollback(
    project_name: String,
    checkpoint_subtask_id: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;
    let project_path = proj.project_path.clone();

    // Find checkpoint subtask. Later immutable labels remain in Git as audit history.
    let mut checkpoint_tag: Option<String> = None;
    let mut checkpoint_found = false;

    for ms in &proj.milestones {
        for mid in &ms.mid_stages {
            for st in &mid.subtasks {
                if st.id == checkpoint_subtask_id {
                    checkpoint_tag = st.auto_tag.clone();
                    checkpoint_found = true;
                }
            }
        }
    }
    if !checkpoint_found {
        return Err("未找到检查点小阶段".to_string());
    }

    // Execute git rollback. A checkpoint without an immutable tag is not a safe target.
    let checkpoint_tag = checkpoint_tag.ok_or("检查点缺少 Git 标签，拒绝回退".to_string())?;
    crate::git_ops::git_reset_to_tag_clean(&project_path, &checkpoint_tag)
        .map_err(|e| format!("Git 回退失败：{}", e))?;

    // Update project data in the same global order used by the preview.
    let mut passed_checkpoint = false;
    for ms in &mut proj.milestones {
        let mut milestone_changed = false;
        for mid in &mut ms.mid_stages {
            let mut mid_changed = false;
            for st in &mut mid.subtasks {
                if st.id == checkpoint_subtask_id {
                    passed_checkpoint = true;
                    continue;
                }
                if passed_checkpoint {
                    st.status = project::SubtaskStatus::RolledBack;
                    st.auto_tag = None;
                    st.execution_result = None;
                    st.test_result = None;
                    st.retry_count = 0;
                    mid_changed = true;
                }
            }
            if mid_changed {
                mid.status = project::MidStageStatus::Pending;
                mid.git_tag.clear();
                mid.completed_at = None;
                milestone_changed = true;
            }
        }
        if milestone_changed && ms.status == project::MilestoneStatus::Completed {
            ms.status = project::MilestoneStatus::InProgress;
        }
    }

    proj.workflow_state.current_step = project::WorkflowStep::PlanGeneration;
    proj.close_active_discussion_thread();
    proj.workflow_state.discussion_scope = project::DiscussionScope::FirstDiscussion;
    proj.workflow_state.pause_reason = project::PauseReason::None;
    proj.pause_context = None;
    proj.execution_session = None;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();

    crate::save_and_reload_project(&proj)
}

// === 辅助函数 ===

/// 从 diff 文本中提取变更文件列表（仅文件名，去重）
fn extract_changed_files(diff_text: &str) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    for line in diff_text.lines() {
        if line.starts_with("diff --git ") {
            // 格式: diff --git a/path b/path
            if let Some(b_path) = line.split(" b/").nth(1) {
                let clean = b_path.trim();
                if !files.contains(&clean.to_string()) {
                    files.push(clean.to_string());
                }
            }
        }
    }
    files
}

/// 从宪法文本中提取第二部分内容（从 "## 第 2 部分" 开始到文末）
fn extract_constitution_part2(constitution: &str) -> String {
    if let Some(pos) = constitution.find("## 第 2 部分") {
        constitution[pos..].to_string()
    } else {
        // Fallback: try "## Part 2" or "## 2."
        if let Some(pos) = constitution.find("## Part 2") {
            constitution[pos..].to_string()
        } else {
            String::new()
        }
    }
}

/// 从 DiffSummary 构建宪法变更摘要描述
fn build_constitution_change_summary(diff: &crate::project::DiffSummary) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !diff.new_files.is_empty() {
        parts.push(format!("新增文件：{}", diff.new_files.join("、")));
    }
    if !diff.modified_files.is_empty() {
        parts.push(format!("修改文件：{}", diff.modified_files.join("、")));
    }
    if !diff.new_functions.is_empty() {
        parts.push(format!("新增函数：{}", diff.new_functions.join("、")));
    }
    if !diff.modified_functions.is_empty() {
        parts.push(format!("修改函数：{}", diff.modified_functions.join("、")));
    }
    if !diff.changed_dependencies.is_empty() {
        parts.push(format!(
            "依赖变更：{}",
            diff.changed_dependencies.join("、")
        ));
    }
    if parts.is_empty() {
        "无结构性变更".to_string()
    } else {
        parts.join("；")
    }
}

pub(crate) fn find_last_passed_subtask(proj: &project::Project) -> Option<project::Subtask> {
    let mut last: Option<project::Subtask> = None;
    for ms in &proj.milestones {
        if ms.mid_stages.is_empty() {
            for address in crate::task_tree::leaf_addresses_in_scope(proj, &ms.id, "").ok()? {
                if let Some(st) = crate::task_tree::find_task(proj, &address.task_id)
                    .ok()
                    .flatten()
                    .filter(|task| task.status == project::SubtaskStatus::Passed)
                {
                    last = Some(st.clone());
                }
            }
        } else {
            for mid in &ms.mid_stages {
                for address in
                    crate::task_tree::leaf_addresses_in_scope(proj, &ms.id, &mid.id).ok()?
                {
                    if let Some(st) = crate::task_tree::find_task(proj, &address.task_id)
                        .ok()
                        .flatten()
                        .filter(|task| task.status == project::SubtaskStatus::Passed)
                    {
                        last = Some(st.clone());
                    }
                }
            }
        }
    }
    last
}

/// 执行状态对账结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionReconciliation {
    /// 正常停留在 Execution，当前没有活跃会话，等待启动下一个小阶段
    IdleAtExecution,
    /// 真执行中：磁盘 session 为 executing，内存 PipelineState 为 Running
    Executing,
    /// 待确认：磁盘 session 为 awaiting_confirmation
    AwaitingConfirmation,
    /// 会话失联：磁盘 session 为 executing 但进程已死
    SessionLost,
    /// 会话无效：session 字段缺失或 active=false
    SessionInvalid,
    /// 数据冲突：session 与当前 milestone/mid_stage 不匹配
    DataConflict,
}

/// 对账执行状态（启动恢复时调用）
///
/// 区分六种情况：
/// - Executing: 磁盘 session=executing + 内存 Running → 恢复轮询
/// - AwaitingConfirmation: 磁盘 session=awaiting_confirmation → 恢复确认界面
/// - SessionLost: 磁盘 session=executing 且内存有状态但非 Running → 进程已死
/// - SessionInvalid: active=false 或字段缺失 → 清理 session
/// - IdleAtExecution: Execution 步骤中无会话，属于两个任务之间的正常空闲态
/// - DataConflict: 与当前 milestone/mid_stage 不匹配 → cleanup
pub fn reconcile_execution_state(
    proj: &project::Project,
    pipeline_status: Option<&PipelineState>,
) -> ExecutionReconciliation {
    let session = match proj.execution_session.as_ref() {
        Some(s) => s,
        None => {
            if proj.workflow_state.current_step == project::WorkflowStep::Execution {
                return ExecutionReconciliation::IdleAtExecution;
            }
            return ExecutionReconciliation::SessionInvalid;
        }
    };

    // 已落盘的可恢复失败会话：即使 active=false 也必须保留证据
    if session.is_recoverable_failure()
        || matches!(
            session.status.as_str(),
            "quality_blocked" | "QualityBlocked" | "confirmation_blocked" | "ConfirmationBlocked"
        )
    {
        if session.subtask_id.is_empty() {
            return ExecutionReconciliation::SessionInvalid;
        }
        return ExecutionReconciliation::AwaitingConfirmation;
    }

    if session.subtask_id.is_empty() {
        return ExecutionReconciliation::SessionInvalid;
    }

    // Check data consistency: session milestone/mid_stage match current
    if proj.current_milestone_id != session.milestone_id
        || proj.current_mid_stage_id != session.mid_stage_id
    {
        return ExecutionReconciliation::DataConflict;
    }

    // Check if referenced subtask still exists
    let subtask_exists = crate::task_tree::locate_task(proj, &session.subtask_id)
        .ok()
        .flatten()
        .is_some_and(|address| {
            address.milestone_id == session.milestone_id
                && address.mid_stage_id == session.mid_stage_id
                && (session.task_path.is_empty() || address.task_path() == session.task_path)
        });

    if !subtask_exists {
        return ExecutionReconciliation::DataConflict;
    }

    let queued_recovery = matches!(session.status.as_str(), "replanning" | "replan_ready")
        && proj
            .workflow_state
            .recovery_state
            .as_ref()
            .is_some_and(|recovery| {
                recovery.subtask_id == session.subtask_id
                    && (recovery.phase == project::RecoveryPhase::Replanning
                        || (session.status == "replan_ready"
                            && recovery.replan_attempted
                            && recovery.phase == project::RecoveryPhase::Diagnosing))
            });
    if queued_recovery {
        return ExecutionReconciliation::AwaitingConfirmation;
    }
    let queued_validation_retry = session.status.eq_ignore_ascii_case("recovering")
        && proj
            .workflow_state
            .recovery_state
            .as_ref()
            .is_some_and(|recovery| {
                recovery.subtask_id == session.subtask_id
                    && recovery.execution_id == session.execution_id
                    && crate::recovery::validation_retry_can_resume(recovery)
            });
    if queued_validation_retry {
        return ExecutionReconciliation::AwaitingConfirmation;
    }

    // Check session validity after recognizing persisted recovery queue states.
    if !session.active {
        return ExecutionReconciliation::SessionInvalid;
    }

    match session.status.as_str() {
        "executing" | "recovering" => {
            match pipeline_status {
                // 内存 PipelineState 存在且正在运行 → 真执行中
                Some(ps) if pipeline_owner_matches(Some(ps), &session.execution_id) => {
                    ExecutionReconciliation::Executing
                }
                // 内存 PipelineState 存在但不在运行 → 进程已死
                Some(_) => ExecutionReconciliation::SessionLost,
                // 内存 PipelineState 尚未建立（应用重启后必然是 None）
                // → 判定为进程失联，不再保留 StartupRecoverable
                None => ExecutionReconciliation::SessionLost,
            }
        }
        // confirming/rejecting：进程崩溃后的半途认领，按待确认恢复，允许人工重试
        "awaiting_confirmation" | "confirming" | "rejecting" => {
            ExecutionReconciliation::AwaitingConfirmation
        }
        _ => ExecutionReconciliation::SessionInvalid,
    }
}

/// 清理无效的执行会话并修正工作流状态
///
/// 根据对账结果更新 Project，返回是否做了修改。
pub fn apply_execution_reconciliation(
    proj: &mut project::Project,
    reconciliation: &ExecutionReconciliation,
) -> bool {
    match reconciliation {
        ExecutionReconciliation::IdleAtExecution
        | ExecutionReconciliation::Executing
        | ExecutionReconciliation::AwaitingConfirmation => {
            // Valid states — keep session, don't modify
            false
        }
        ExecutionReconciliation::SessionLost => {
            // Process died — mark session as lost and reset the stuck subtask
            let now = chrono::Utc::now().to_rfc3339();
            let mut lost_task_id = None;
            if let Some(ref mut session) = proj.execution_session {
                // 已是 session_lost 时不重复清空证据
                if session.status != "session_lost" {
                    session.status = "session_lost".to_string();
                    session.active = false;
                    if session.failure_message.is_empty() {
                        session.failure_message =
                            "执行进程失联，工作区可能残留未提交修改。".to_string();
                    }
                    session.state_entered_at = now.clone();
                }
                lost_task_id = Some(session.subtask_id.clone());
            }
            // Task identity, not container shape, is authoritative for both direct and
            // mid-stage plans (including a split child leaf).
            if let Some(task_id) = lost_task_id {
                if let Ok(Some(task)) = crate::task_tree::find_task_mut(proj, &task_id) {
                    if task.status == project::SubtaskStatus::Executing
                        || (task.status == project::SubtaskStatus::AwaitingConfirmation
                            && task
                                .execution_result
                                .as_ref()
                                .map(|result| !result.success)
                                .unwrap_or(false))
                    {
                        task.status = project::SubtaskStatus::Pending;
                        task.execution_result = None;
                        task.test_result = None;
                    }
                }
            }
            let replan_execution_lost = proj
                .workflow_state
                .recovery_state
                .as_ref()
                .is_some_and(|recovery| recovery.replan_execution_attempted);
            // 自动驾驶显式标记恢复动作，不得靠错误文本猜测
            if proj.workflow_state.autopilot_active {
                if let Some(ref mut ap) = proj.workflow_state.autopilot_state {
                    // The persisted runtime owner belongs to the lost process. Keep
                    // heartbeat/history for diagnosis, but never let startup treat it as live.
                    ap.job_owner = project::AutopilotJobOwner::None;
                    ap.current_action_id.clear();
                    ap.current_action_kind.clear();
                    ap.action_started_at.clear();
                    let interrupted_recovery = proj.workflow_state.recovery_state.is_some();
                    ap.run_status = if replan_execution_lost {
                        project::AutopilotRunStatus::ErrorStopped
                    } else if interrupted_recovery {
                        project::AutopilotRunStatus::Running
                    } else {
                        project::AutopilotRunStatus::ErrorStopped
                    };
                    ap.last_action = if replan_execution_lost {
                        "重规划后的任务执行失联，等待人工处理".to_string()
                    } else if interrupted_recovery {
                        "自动修复进程失联，准备从基线重新执行".to_string()
                    } else {
                        "执行会话失联，需要恢复执行基线".to_string()
                    };
                    ap.last_action_at = now;
                    if ap.error_message.is_empty() {
                        ap.error_message = "执行进程失联，请先恢复执行基线后再继续。".to_string();
                    }
                    ap.recovery_action = if replan_execution_lost {
                        project::AutopilotRecoveryAction::WaitHumanDecision
                    } else if interrupted_recovery {
                        project::AutopilotRecoveryAction::RunAutomaticRecovery
                    } else {
                        project::AutopilotRecoveryAction::RestoreExecutionBaseline
                    };
                }
            }
            if let Some(recovery) = proj.workflow_state.recovery_state.as_mut() {
                if recovery.replan_execution_attempted {
                    recovery.error_kind = project::RecoveryErrorKind::HumanRequired;
                    recovery.phase = project::RecoveryPhase::WaitingHuman;
                    recovery.last_repair_summary =
                        "重规划后的任务执行进程失联，禁止自动重复启动".to_string();
                    recovery.updated_at = chrono::Utc::now().to_rfc3339();
                } else if recovery.phase == project::RecoveryPhase::Replanning {
                    recovery.last_repair_summary =
                        "重规划进程失联；下次尝试将重新开始当前任务重规划".to_string();
                    recovery.updated_at = chrono::Utc::now().to_rfc3339();
                } else {
                    recovery.error_kind = project::RecoveryErrorKind::ExecutionError;
                    recovery.phase = project::RecoveryPhase::Diagnosing;
                    recovery.last_repair_summary =
                        "恢复进程中断；下次尝试将先恢复执行基线".to_string();
                    recovery.updated_at = chrono::Utc::now().to_rfc3339();
                }
            }
            proj.workflow_state.data_revision += 1;
            true
        }
        ExecutionReconciliation::SessionInvalid => {
            proj.execution_session = None;
            if proj.workflow_state.current_step == project::WorkflowStep::Execution {
                proj.workflow_state.current_step =
                    crate::workflow_resolution::execution_recovery_selection_step(proj);
                proj.workflow_state.data_revision += 1;
                proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
            }
            true
        }
        ExecutionReconciliation::DataConflict => {
            // Data mismatch — full cleanup
            proj.execution_session = None;
            // Go back to a safe state
            if proj.workflow_state.current_step == project::WorkflowStep::Execution
                || proj.workflow_state.current_step == project::WorkflowStep::PauseDecision
            {
                proj.workflow_state.current_step =
                    crate::workflow_resolution::execution_recovery_selection_step(proj);
                proj.workflow_state.data_revision += 1;
                proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
            }
            true
        }
    }
}

fn infer_startup_resource_facts(proj: &mut project::Project) -> bool {
    let observation = crate::snapshot::load_snapshot(&proj.name)
        .ok()
        .flatten()
        .and_then(|snapshot| snapshot.startup_process_observation);
    let Some(session) = proj.execution_session.as_ref() else {
        return false;
    };
    if !matches!(
        session.parsed_status(),
        project::ExecutionSessionStatus::SessionLost
            | project::ExecutionSessionStatus::ExecutionFailed
    ) {
        return false;
    }
    let (observation, failure_kind, message, intentional_exit) = match observation {
        Some(observation) => match observation.kind {
            crate::snapshot::StartupProcessObservationKind::Killed => (
                project::ResourceObservationSummary {
                    state: project::ResourceObservationState::KilledSuspected,
                    source: project::ResourceObservationSource::Unknown,
                    sampled_at: Some(observation.observed_at.clone()),
                    ..Default::default()
                },
                Some(project::ResourceFailureKind::ResourceKilled),
                format!(
                    "启动对账发现执行子进程 PID={} 已被清理；无法据此确认 OOM，资源终止标记为 KilledSuspected。",
                    observation.pid
                ),
                false,
            ),
            crate::snapshot::StartupProcessObservationKind::IntentionalExit => (
                project::ResourceObservationSummary {
                    sampled_at: Some(observation.observed_at.clone()),
                    ..Default::default()
                },
                None,
                format!(
                    "应用已正常退出，执行子进程 PID={} 的退出属于 intentional exit；资源来源保持 Unknown。",
                    observation.pid
                ),
                true,
            ),
            crate::snapshot::StartupProcessObservationKind::AlreadyExited => (
                project::ResourceObservationSummary {
                    sampled_at: Some(observation.observed_at.clone()),
                    ..Default::default()
                },
                None,
                format!(
                    "启动对账发现执行子进程 PID={} 已退出；没有资源终止证据，保持 Unknown。",
                    observation.pid
                ),
                false,
            ),
            crate::snapshot::StartupProcessObservationKind::TerminationUnknown
            | crate::snapshot::StartupProcessObservationKind::IdentityUnverified => (
                project::ResourceObservationSummary {
                    sampled_at: Some(observation.observed_at.clone()),
                    ..Default::default()
                },
                None,
                format!(
                    "启动对账无法证明执行子进程 PID={} 的归属或终止来源，保持 SessionLost + Unknown；请先核对工作区基线。",
                    observation.pid
                ),
                false,
            ),
        },
        None => (
            project::ResourceObservationSummary::default(),
            None,
            "启动对账无法证明资源终止来源，保持 SessionLost + Unknown；请先核对工作区基线。"
                .to_string(),
            false,
        ),
    };
    if intentional_exit {
        if let Some(session) = proj.execution_session.as_mut() {
            session.failure_message = message.clone();
        }
    }
    crate::recovery::record_resource_facts(proj, observation, failure_kind, &message)
}

/// 在调用方已持有流水线互斥权时，对最新项目快照做执行对账并可选写盘。
///
/// 必须在持有 `pipeline_state` 锁期间调用：先取锁、再 load、再对账/保存，
/// 避免“先读旧盘 → 后台完成写盘 → 用旧快照覆盖”的窗口（与 ED Stop 同构）。
fn normalize_legacy_confirmation_conflict_kind(proj: &mut project::Project) -> bool {
    let Some(session) = proj.execution_session.as_ref() else {
        return false;
    };
    if session.confirmation_failure_kind
        != Some(project::GitConfirmationFailureKind::TagIdentityConflict)
    {
        return false;
    }
    let is_legacy_v1_conflict = session.confirmation_transaction_id.is_empty()
        && session.confirmation_candidate_tag.is_empty()
        && session.confirmation_commit.is_empty()
        && session.confirmation_phase == project::ConfirmationPhase::NotStarted;
    let normalized = if is_legacy_v1_conflict {
        project::GitConfirmationFailureKind::LegacyV1TagConflict
    } else {
        project::GitConfirmationFailureKind::V2TagIntegrityConflict
    };
    if let Some(session) = proj.execution_session.as_mut() {
        session.confirmation_failure_kind = Some(normalized.clone());
    }
    if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
        autopilot.recovery_action = confirmation_recovery_action(&normalized);
    }
    true
}

fn migrate_legacy_v1_confirmation_conflict(proj: &mut project::Project) -> bool {
    let Some(session) = proj.execution_session.as_ref() else {
        return false;
    };
    if !session.confirmation_transaction_id.is_empty()
        || !matches!(
            session.status.as_str(),
            "execution_failed"
                | "ExecutionFailed"
                | "awaiting_confirmation"
                | "AwaitingConfirmation"
        )
    {
        return false;
    }

    let milestone_id = session.milestone_id.clone();
    let mid_stage_id = session.mid_stage_id.clone();
    let subtask_id = session.subtask_id.clone();
    let Some(mid_version) = proj
        .milestones
        .iter()
        .find(|milestone| milestone.id == milestone_id)
        .and_then(|milestone| {
            milestone
                .mid_stages
                .iter()
                .find(|mid_stage| mid_stage.id == mid_stage_id)
        })
        .map(|mid_stage| mid_stage.version.clone())
    else {
        return false;
    };
    let Some((subtask_index, subtask)) =
        crate::task_tree::leaf_addresses_in_scope(proj, &milestone_id, &mid_stage_id)
            .ok()
            .and_then(|leaves| {
                leaves
                    .iter()
                    .position(|address| address.task_id == subtask_id)
                    .and_then(|index| {
                        crate::task_tree::find_task(proj, &subtask_id)
                            .ok()
                            .flatten()
                            .map(|task| ((index + 1) as u32, task))
                    })
            })
    else {
        return false;
    };
    if subtask.status != project::SubtaskStatus::AwaitingConfirmation {
        return false;
    }
    let Some(authorized_paths) =
        crate::plan_contract::validate_subtask(subtask, &format!("第 {} 个小阶段", subtask_index))
            .ok()
    else {
        return false;
    };

    let mut quality_candidate = proj.clone();
    if let Some(candidate_session) = quality_candidate.execution_session.as_mut() {
        candidate_session.status = "awaiting_confirmation".to_string();
        candidate_session.active = true;
    }
    if validate_subtask_quality_gate(&quality_candidate).is_err() {
        return false;
    }
    let legacy_conflict = crate::git_ops::is_legacy_v1_tag_conflict(
        &proj.project_path,
        &mid_version,
        subtask_index,
        &authorized_paths,
    )
    .unwrap_or(false);
    if !legacy_conflict {
        return false;
    }

    let message = "检测到旧 V1 标签身份碰撞，可改用 V2 标签重新确认提交。".to_string();
    mark_confirmation_blocked(
        proj,
        project::GitConfirmationFailureKind::LegacyV1TagConflict,
        message,
    );
    true
}

pub(crate) fn reconcile_loaded_project_under_pipeline_lock(
    proj: &mut project::Project,
    pipeline_status: Option<&PipelineState>,
) -> bool {
    if matches!(
        crate::control_action_executor::classify_control_action_occupancy(
            &proj.task_control,
            crate::project_state_bus::process_start_id(),
            chrono::Utc::now(),
        ),
        crate::control_action_executor::ControlActionOccupancy::ActiveLocal(_)
            | crate::control_action_executor::ControlActionOccupancy::ActiveForeign(_)
    ) {
        // 活跃控制动作拥有项目事实修改权；启动/同步对账不得覆盖其心跳或中间状态。
        return false;
    }
    let mut modified = crate::provability::migrate_project_metadata(proj);
    modified |= normalize_legacy_confirmation_conflict_kind(proj);
    modified |= migrate_legacy_v1_confirmation_conflict(proj);
    let reconciliation = reconcile_execution_state(proj, pipeline_status);
    if reconciliation == ExecutionReconciliation::AwaitingConfirmation {
        let interrupted_claim = proj
            .execution_session
            .as_ref()
            .map(|session| session.status.clone());
        match interrupted_claim.as_deref() {
            Some("confirming") => {
                let has_transaction = proj
                    .execution_session
                    .as_ref()
                    .is_some_and(|session| !session.confirmation_transaction_id.is_empty());
                if has_transaction {
                    mark_confirmation_blocked(
                        proj,
                        project::GitConfirmationFailureKind::ProjectFinalizationFailed,
                        "上次 Git 确认在收口前中断。".to_string(),
                    );
                } else if let Some(session) = proj.execution_session.as_mut() {
                    session.status = "awaiting_confirmation".to_string();
                    session.state_entered_at = chrono::Utc::now().to_rfc3339();
                }
                modified = true;
            }
            Some("rejecting") => {
                if let Some(session) = proj.execution_session.as_mut() {
                    session.status = "awaiting_confirmation".to_string();
                    session.state_entered_at = chrono::Utc::now().to_rfc3339();
                }
                modified = true;
            }
            _ => {}
        }
    }
    modified |= apply_execution_reconciliation(proj, &reconciliation);
    if reconciliation == ExecutionReconciliation::SessionLost {
        modified |= infer_startup_resource_facts(proj);
    }
    let lock_reconciliation = crate::control_action_executor::reconcile_stale_control_action_lock(
        proj,
        crate::project_state_bus::process_start_id(),
        chrono::Utc::now(),
    );
    modified |= lock_reconciliation.changed();
    modified
}

/// 启动时对账执行状态：取流水线锁 → 加载最新项目 → reconcile → apply → 保存。
///
/// 与独立函数 `reconcile_execution_state` + `apply_execution_reconciliation` 的区别：
/// 本命令是一个完整的持久化流程，返回对账并保存后的磁盘事实，供前端启动恢复使用。
/// 全程与后台完成路径共用 `pipeline_state` 互斥，禁止在取锁前缓存项目快照。
#[tauri::command]
pub(crate) async fn reconcile_on_startup(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    // 先取锁，再 load：与后台完成/ED Stop 同一互斥周期，杜绝旧快照覆盖新结果。
    let guard = state.pipeline_state.lock().await;
    let (result, should_start_autopilot, should_start_managed) =
        crate::mutate_project_for_control(&project_name, |proj| {
            let persisted_revision = proj.workflow_state.data_revision;
            let mut modified = reconcile_loaded_project_under_pipeline_lock(proj, guard.as_ref());
            crate::commands::workflow::reconcile_autopilot_in_migration(proj);
            let closure_changed =
                crate::commands::workflow::reconcile_workflow_closure_state(proj)?;
            if closure_changed {
                proj.workflow_state.data_revision = persisted_revision.saturating_add(1);
            }
            modified |= closure_changed;
            let should_start_autopilot = crate::autopilot_runtime::reconcile_startup_job(proj);
            let should_start_managed = crate::managed_runtime::reconcile_startup_job(proj);
            modified |= proj.workflow_state.autopilot_state.is_some();
            modified |= proj.workflow_state.managed_flow_state.is_some();
            Ok((
                (proj.clone(), should_start_autopilot, should_start_managed),
                modified,
            ))
        })?;
    drop(guard);
    if should_start_autopilot {
        state
            .autopilot_runtime
            .start(state.pipeline_state.clone(), project_name.clone())
            .await?;
    }
    if should_start_managed {
        state.managed_runtime.start(project_name).await?;
    }
    Ok(result)
}

/// 应用启动恢复确认：实际恢复 Git 基线；失败时保留会话与证据，禁止谎称已恢复
#[tauri::command]
pub(crate) async fn acknowledge_execution_recovery(
    state: tauri::State<'_, AppState>,
    project_name: String,
    expected_state_fingerprint: Option<String>,
) -> Result<project::Project, String> {
    let mut pipeline = state.pipeline_state.lock().await;
    let (updated, recovered) = acknowledge_execution_recovery_detailed(
        project_name.clone(),
        expected_state_fingerprint.as_deref(),
        true,
        &mut pipeline,
    )
    .await?;
    drop(pipeline);
    if !recovered {
        return Ok(updated);
    }
    state
        .autopilot_runtime
        .start_if_active(state.pipeline_state.clone(), project_name)
        .await?;
    Ok(updated)
}

pub(crate) async fn acknowledge_execution_recovery_inner(
    project_name: String,
) -> Result<project::Project, String> {
    let mut pipeline = None;
    acknowledge_execution_recovery_detailed(project_name, None, false, &mut pipeline)
        .await
        .map(|(project, _)| project)
}

pub(crate) async fn acknowledge_execution_recovery_with_pipeline(
    pipeline_state: &std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
    project_name: String,
) -> Result<project::Project, String> {
    let mut pipeline = pipeline_state.lock().await;
    acknowledge_execution_recovery_detailed(project_name, None, false, &mut pipeline)
        .await
        .map(|(project, _)| project)
}

async fn acknowledge_execution_recovery_detailed(
    project_name: String,
    expected_state_fingerprint: Option<&str>,
    require_preview: bool,
    pipeline: &mut Option<PipelineState>,
) -> Result<(project::Project, bool), String> {
    let mut proj = crate::load_project(&project_name)?;
    let already_closed = proj.execution_session.is_none()
        && proj.workflow_state.recovery_state.is_none()
        && !proj
            .workflow_state
            .autopilot_state
            .as_ref()
            .is_some_and(|autopilot| {
                autopilot.recovery_action
                    == project::AutopilotRecoveryAction::RestoreExecutionBaseline
            });
    if already_closed {
        return Ok((proj, false));
    }
    let project_path = proj.project_path.clone();
    let waiting_engine = proj
        .workflow_state
        .recovery_state
        .as_ref()
        .is_some_and(|recovery| {
            recovery.phase == project::RecoveryPhase::WaitingEngine
                && recovery.error_kind == project::RecoveryErrorKind::EngineBlocked
        });

    let session = recoverable_execution_session(&proj)?;

    let subtask_id = session.subtask_id.clone();
    let subtask_title = session.subtask_title.clone();
    let milestone_id = session.milestone_id.clone();
    let mid_stage_id = session.mid_stage_id.clone();

    let restore_target = execution_restore_target(&proj, &session);

    if require_preview {
        verify_execution_recovery_preview(
            &proj,
            "acknowledge_execution_recovery",
            expected_state_fingerprint,
        )?;
    }

    // Git 恢复失败：保留失败会话、基线和错误证据，自动驾驶保持 ErrorStopped
    restore_git_execution_baseline(&project_path, &restore_target).map_err(|outcome| {
        format!(
            "Git 基线恢复失败：{}。失败证据已保留，请勿认为已恢复到安全状态。",
            outcome.error_message()
        )
    })?;

    if waiting_engine {
        let prepared_engine = crate::engine::prepare_engine(&proj.execution_profile).await?;
        if prepared_engine.health.status.blocks_execution() {
            return Err(format!(
                "执行基线已恢复，但 {} 健康检查未通过：{}。引擎阻断仍保留。",
                proj.execution_profile.provider.display_name(),
                prepared_engine.health.message
            ));
        }
        let confirmed_revision = prepared_engine.settings().revision;
        if let Some(current_session) = proj.execution_session.as_mut() {
            current_session.engine_snapshot = proj.execution_profile.clone();
            current_session.engine_settings_revision = confirmed_revision;
            current_session.engine_source_revision =
                if proj.execution_profile.runtime == project::ExecutionRuntime::BuiltIn {
                    prepared_engine
                        .health
                        .source_revision
                        .clone()
                        .unwrap_or_default()
                } else {
                    String::new()
                };
            current_session.engine_api_backend =
                if proj.execution_profile.runtime == project::ExecutionRuntime::BuiltIn {
                    prepared_engine
                        .settings()
                        .built_in_grok_build
                        .api_backend
                        .as_str()
                        .to_string()
                } else {
                    String::new()
                };
            current_session.engine_model =
                if proj.execution_profile.runtime == project::ExecutionRuntime::BuiltIn {
                    prepared_engine.settings().built_in_grok_build.model.clone()
                } else {
                    String::new()
                };
            current_session.endpoint_fingerprint =
                if proj.execution_profile.runtime == project::ExecutionRuntime::BuiltIn {
                    crate::settings::endpoint_fingerprint(
                        &prepared_engine.settings().built_in_grok_build.api_base_url,
                    )
                } else {
                    String::new()
                };
            current_session.engine_executable_path = prepared_engine
                .health
                .executable_path
                .clone()
                .unwrap_or_default();
        }
        if session.engine_snapshot != proj.execution_profile
            || session.engine_settings_revision != confirmed_revision
        {
            let audit_message = format!(
                "用户确认执行引擎恢复：{:?}/{}（设置修订 {}） -> {:?}/{}（设置修订 {}）",
                session.engine_snapshot.runtime,
                session.engine_snapshot.provider.display_name(),
                session.engine_settings_revision,
                proj.execution_profile.runtime,
                proj.execution_profile.provider.display_name(),
                confirmed_revision,
            );
            write_execution_history(
                &mut proj,
                "info",
                project::ExecutionEventType::EngineProfileChanged,
                audit_message,
                Some(&milestone_id),
                Some(&mid_stage_id),
                Some(&subtask_id),
            );
        }
    }

    // 基线恢复成功后才清除会话
    proj.execution_session = None;
    proj.workflow_state.recovery_state = None;

    // 确保受影响任务为 Pending，可再次执行
    if let Some(st) = crate::task_tree::find_task_mut(&mut proj, &subtask_id)? {
        if st.status != project::SubtaskStatus::Passed {
            st.status = project::SubtaskStatus::Pending;
            st.execution_result = None;
            st.test_result = None;
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    write_execution_history(
        &mut proj,
        "info",
        project::ExecutionEventType::RetryScheduled,
        format!("🔧 已恢复执行基线：{}", subtask_title),
        Some(&milestone_id),
        Some(&mid_stage_id),
        Some(&subtask_id),
    );

    if let Some(ref mut ap) = proj.workflow_state.autopilot_state {
        if waiting_engine
            || ap.recovery_action == project::AutopilotRecoveryAction::RestoreExecutionBaseline
        {
            ap.recovery_action = project::AutopilotRecoveryAction::None;
            ap.error_message = String::new();
            ap.last_action = if waiting_engine {
                format!(
                    "{} 健康检查通过，已恢复基线并准备重试：{}",
                    proj.execution_profile.provider.display_name(),
                    subtask_title
                )
            } else {
                format!("已恢复执行基线：{}", subtask_title)
            };
            ap.last_action_at = now.clone();
            // 基线恢复是完整恢复命令，成功后直接回到自动推进。
            if ap.run_status == project::AutopilotRunStatus::ErrorStopped {
                ap.run_status = project::AutopilotRunStatus::Running;
            }
        }
    }

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    if pipeline
        .as_ref()
        .is_some_and(|value| value.project_name.is_empty() || value.project_name == project_name)
    {
        *pipeline = None;
    }
    crate::save_and_reload_project(&proj).map(|project| (project, true))
}

fn find_current_subtask(proj: &project::Project) -> Option<project::Subtask> {
    let address = crate::task_tree::select_current_leaf(proj).ok().flatten()?;
    crate::task_tree::find_task(proj, &address.task_id)
        .ok()
        .flatten()
        .cloned()
}

// ===================================================================
// 测试
// ===================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[test]
    fn resource_failure_retains_guard_observation_for_finalizer() {
        let failure = BackgroundExecutionFailure::resource(
            project::RecoveryErrorKind::ExecutionError,
            "resource hard stop".to_string(),
            None,
            Some(project::ResourceObservationSummary {
                state: project::ResourceObservationState::HardStop,
                peak_rss_bytes: Some(900),
                headroom_bytes: Some(100),
                ..Default::default()
            }),
        );

        assert_eq!(
            failure.resource_observation.state,
            project::ResourceObservationState::HardStop
        );
        assert_eq!(failure.resource_observation.peak_rss_bytes, Some(900));
        assert_eq!(failure.resource_observation.headroom_bytes, Some(100));
        assert_eq!(
            failure.resource_failure_kind,
            Some(project::ResourceFailureKind::ResourcePressure)
        );
    }

    #[test]
    fn adaptive_execution_contract_builtin_interruptions_reach_cost_ledger_with_facts() {
        let partial_result = || project::ExecutionResult {
            file_changes: vec!["first.txt".to_string(), "continuation.txt".to_string()],
            token_usage: Some(crate::cost_ledger::ProviderUsage {
                input_tokens: Some(13),
                output_tokens: Some(8),
                total_tokens: Some(21),
                cached_input_tokens: None,
            }),
            ..Default::default()
        };
        for error in [
            crate::engine::EngineError::cancelled_with_result(partial_result()),
            crate::engine::EngineError::timeout_with_result(partial_result()),
        ] {
            let (usage, produced_change, failure_kind) =
                interrupted_execution_cost_facts(&error).expect("interruption facts");
            let mut ledger = crate::cost_ledger::CostLedger::default();
            crate::cost_ledger::record_execution_call(
                &mut ledger,
                failure_kind,
                &crate::cost_ledger::ModelCallContext::default(),
                "Grok Build",
                "test-model",
                "started".to_string(),
                "ended".to_string(),
                42,
                usage,
                produced_change,
                failure_kind,
            );
            let call = ledger.calls.last().expect("recorded execution call");
            assert_eq!(call.failure_kind, failure_kind);
            assert_eq!(call.input_tokens, Some(13));
            assert_eq!(call.output_tokens, Some(8));
            assert_eq!(call.total_tokens, Some(21));
            assert!(call.produced_change);
            assert!(!call.no_progress);
        }
    }

    #[test]
    fn operation_source_is_preserved_and_legacy_writer_is_conservative() {
        let mut project = project::Project::new("audit");
        write_execution_history_with_source(
            &mut project,
            "info",
            project::ExecutionEventType::AutopilotExecute,
            project::OperationSource::Autopilot,
            "auto".to_string(),
            None,
            None,
            None,
        );
        write_execution_history(
            &mut project,
            "info",
            project::ExecutionEventType::SystemAdvance,
            "system".to_string(),
            None,
            None,
            None,
        );
        assert_eq!(
            project.execution_history[0].source,
            project::OperationSource::Autopilot
        );
        assert_eq!(
            project.execution_history[1].source,
            project::OperationSource::System
        );
    }

    #[test]
    fn autopilot_operation_source_is_not_attributed_to_user() {
        let (execute_event, execute_text) =
            execution_request_audit(project::OperationSource::Autopilot, 1, 2, "task", "engine");
        let (confirm_event, confirm_text) =
            confirmation_audit(project::OperationSource::Autopilot, "task");
        assert_eq!(execute_event, project::ExecutionEventType::AutopilotExecute);
        assert_eq!(confirm_event, project::ExecutionEventType::AutopilotConfirm);
        assert!(!execute_text.contains("用户"));
        assert!(!confirm_text.contains("用户"));
    }

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

    struct TempGitRepo {
        path: PathBuf,
    }

    impl TempGitRepo {
        fn new(label: &str) -> Result<Self, String> {
            let path =
                std::env::temp_dir().join(format!("metheus-{}-{}", label, uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path)
                .map_err(|error| format!("创建临时 Git 目录失败：{}", error))?;
            let repo = Self { path };
            repo.git(&["init", "--quiet"])?;
            repo.git(&["config", "user.name", "Metheus Test"])?;
            repo.git(&["config", "user.email", "metheus-test@example.invalid"])?;
            std::fs::write(repo.path.join("tracked.txt"), "baseline\n")
                .map_err(|error| format!("写入 Git 测试基线失败：{}", error))?;
            repo.git(&["add", "tracked.txt"])?;
            repo.git(&["commit", "--quiet", "-m", "baseline"])?;
            Ok(repo)
        }

        fn git(&self, args: &[&str]) -> Result<String, String> {
            let output = Command::new("git")
                .args(args)
                .current_dir(&self.path)
                .output()
                .map_err(|error| format!("运行 git {:?} 失败：{}", args, error))?;
            if !output.status.success() {
                return Err(format!(
                    "git {:?} 失败：{}",
                    args,
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }

        fn path_string(&self) -> String {
            self.path.to_string_lossy().to_string()
        }

        fn head(&self) -> Result<String, String> {
            self.git(&["rev-parse", "HEAD"])
        }
    }

    impl Drop for TempGitRepo {
        fn drop(&mut self) {
            if let Err(error) = std::fs::remove_dir_all(&self.path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("清理临时 Git 目录 {} 失败：{}", self.path.display(), error);
                }
            }
        }
    }

    fn unique_project_name(label: &str) -> String {
        format!("test-{}-{}", label, uuid::Uuid::new_v4())
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

    fn test_mid_stage(status: project::SubtaskStatus) -> project::MidStage {
        project::MidStage {
            id: "mid-1".to_string(),
            title: "测试中阶段".to_string(),
            version: "v0.1.1".to_string(),
            order: Some(1),
            status: project::MidStageStatus::InProgress,
            subtasks: vec![test_subtask(status)],
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
            plan_approved_at: Some("2026-07-20T00:00:00Z".to_string()),
            plan_revision: 1,
            plan_draft_revision: 1,
            plan_generated_at: Some("2026-07-20T00:00:00Z".to_string()),
            plan_regeneration_count: 0,
            last_plan_failure_fingerprint: String::new(),
            last_plan_issue_count: 0,
            plan_no_progress_count: 0,
        }
    }

    fn test_milestone(subtask_status: project::SubtaskStatus) -> project::Milestone {
        project::Milestone {
            id: "milestone-1".to_string(),
            version: "v0.1".to_string(),
            title: "测试大阶段".to_string(),
            description: String::new(),
            tech_stack: String::new(),
            status: project::MilestoneStatus::InProgress,
            mode: project::StageMode::Professional,
            mid_stages: vec![test_mid_stage(subtask_status)],
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

    fn execution_session(
        status: &str,
        execution_id: &str,
        base_commit: &str,
    ) -> project::ExecutionSession {
        project::ExecutionSession {
            execution_id: execution_id.to_string(),
            active: true,
            milestone_id: "milestone-1".to_string(),
            mid_stage_id: "mid-1".to_string(),
            subtask_id: "subtask-1".to_string(),
            subtask_title: "测试小阶段".to_string(),
            status: status.to_string(),
            base_commit: base_commit.to_string(),
            failure_message: String::new(),
            verification_stage: project::VerificationStage::NotStarted,
            confirmation_transaction_id: String::new(),
            confirmation_phase: project::ConfirmationPhase::NotStarted,
            confirmation_candidate_tag: String::new(),
            confirmation_commit: String::new(),
            confirmation_failure_kind: None,
            started_at: "2026-07-20T00:00:00Z".to_string(),
            state_entered_at: "2026-07-20T00:00:00Z".to_string(),
            plan_revision: 1,
            subtask_index: 0,
            total_subtasks: 1,
            task_path: vec!["subtask-1".to_string()],
            parent_task_id: String::new(),
            top_level_task_id: "subtask-1".to_string(),
            task_tree_revision: 0,
            contract_fingerprint: String::new(),
            node_depth: 0,
            engine_snapshot: project::ExecutionProfile::default(),
            engine_settings_revision: 0,
            engine_source_revision: String::new(),
            engine_api_backend: String::new(),
            engine_model: String::new(),
            endpoint_fingerprint: String::new(),
            engine_executable_path: String::new(),
            human_review_cadence: project::HumanReviewCadence::PerTask,
        }
    }

    fn execution_project(
        project_name: &str,
        project_path: &Path,
        subtask_status: project::SubtaskStatus,
        session: Option<project::ExecutionSession>,
    ) -> project::Project {
        let mut proj = project::Project::new(project_name);
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
            .expect("professional execution profile"),
        );
        proj.project_path = project_path.to_string_lossy().to_string();
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id = "mid-1".to_string();
        proj.milestones = vec![test_milestone(subtask_status)];
        proj.execution_session = session;
        proj
    }

    fn quick_execution_project(
        subtask_status: project::SubtaskStatus,
        session_status: Option<&str>,
    ) -> project::Project {
        let mut proj = project::Project::new("quick-execution");
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
                0,
            )
            .expect("quick execution profile"),
        );
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id.clear();
        proj.milestones = vec![project::Milestone {
            id: "milestone-1".to_string(),
            version: "v0.1".to_string(),
            title: "Quick".to_string(),
            status: project::MilestoneStatus::InProgress,
            mode: project::StageMode::Quick,
            subtasks: vec![test_subtask(subtask_status)],
            plan_approved_at: Some("2026-08-01T00:00:00Z".to_string()),
            plan_revision: 1,
            ..Default::default()
        }];
        proj.execution_session = session_status.map(|status| {
            let mut session = execution_session(status, "quick-execution-1", "abc123");
            session.mid_stage_id.clear();
            session
        });
        proj
    }

    fn pipeline_state(execution_id: &str, status: PipelineStatus) -> PipelineState {
        PipelineState {
            execution_id: execution_id.to_string(),
            mid_stage_id: "mid-1".to_string(),
            status,
            current_subtask_index: 0,
            total_subtasks: 1,
            subtask_statuses: vec![],
            current_log: String::new(),
            last_error: None,
            child_pid: None,
            project_name: String::new(),
            milestone_id: "milestone-1".to_string(),
            plan_revision: 1,
            current_subtask_id: "subtask-1".to_string(),
            awaiting_confirmation: false,
            log_history: vec![],
        }
    }

    #[test]
    fn pipeline_owner_requires_exact_non_empty_execution_identity() {
        let running = pipeline_state("execution-1", PipelineStatus::Running);
        assert!(pipeline_owner_matches(Some(&running), "execution-1"));
        assert!(!pipeline_owner_matches(Some(&running), "execution-old"));
        assert!(!pipeline_owner_matches(Some(&running), ""));

        let failed = pipeline_state("execution-1", PipelineStatus::Failed);
        assert!(!pipeline_owner_matches(Some(&failed), "execution-1"));
    }

    #[test]
    fn intentional_exit_keeps_session_active_and_records_only_once() {
        let mut project = execution_project(
            "intentional-exit",
            Path::new(""),
            project::SubtaskStatus::Executing,
            Some(execution_session("executing", "execution-1", "HEAD")),
        );
        let mut pipeline = pipeline_state("execution-1", PipelineStatus::Running);
        pipeline.project_name = project.name.clone();
        pipeline.child_pid = Some(42);

        assert!(record_intentional_exit(&mut project, Some(&pipeline)));
        assert!(!record_intentional_exit(&mut project, Some(&pipeline)));
        assert!(project.execution_session.as_ref().unwrap().active);
        assert_eq!(project.execution_history.len(), 1);
        assert!(project.execution_history[0]
            .text
            .contains("应用正常退出：执行会话保留"));
    }

    #[test]
    fn validate_quality_gate_requires_session() {
        let proj = crate::project::Project::new("test-qg");
        let result = validate_subtask_quality_gate(&proj);
        assert!(result.is_err());
        assert!(result
            .err()
            .is_some_and(|error| error.contains("没有活跃的执行会话")));
    }

    #[test]
    fn execution_history_is_appended_in_order_and_survives_reload() -> Result<(), String> {
        let project_name = unique_project_name("history");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        write_execution_history(
            &mut proj,
            "info",
            project::ExecutionEventType::UserExecute,
            "first".to_string(),
            None,
            None,
            None,
        );
        write_execution_history(
            &mut proj,
            "pause",
            project::ExecutionEventType::UserEdStop,
            "second".to_string(),
            None,
            None,
            None,
        );
        write_execution_history(
            &mut proj,
            "error",
            project::ExecutionEventType::QualityGateBlocked,
            "third".to_string(),
            None,
            None,
            None,
        );
        crate::save_project(&proj)?;
        let reloaded = crate::load_project(&project_name)?;
        let texts: Vec<&str> = reloaded
            .execution_history
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        assert_eq!(texts, vec!["first", "second", "third"]);
        Ok(())
    }

    #[test]
    fn old_pipeline_state_without_execution_id_defaults_to_empty() -> Result<(), String> {
        let mut value = serde_json::to_value(pipeline_state("execution-1", PipelineStatus::Idle))
            .map_err(|error| format!("序列化流水线状态失败：{}", error))?;
        let object = value
            .as_object_mut()
            .ok_or("流水线状态未序列化为对象".to_string())?;
        object.remove("execution_id");
        let restored: PipelineState = serde_json::from_value(value)
            .map_err(|error| format!("反序列化旧流水线状态失败：{}", error))?;
        assert!(restored.execution_id.is_empty());
        Ok(())
    }

    #[test]
    fn execution_reconciliation_covers_idle_matching_and_lost_sessions() {
        let empty_path = Path::new("");
        let idle = execution_project("idle", empty_path, project::SubtaskStatus::Pending, None);
        let idle_result = reconcile_execution_state(&idle, None);
        assert!(matches!(
            idle_result,
            ExecutionReconciliation::IdleAtExecution
        ));
        let mut idle_copy = idle.clone();
        assert!(!apply_execution_reconciliation(
            &mut idle_copy,
            &idle_result
        ));
        assert_eq!(
            idle_copy.workflow_state.current_step,
            project::WorkflowStep::Execution
        );

        let running_session = execution_session("executing", "execution-1", "HEAD");
        let running = execution_project(
            "running",
            empty_path,
            project::SubtaskStatus::Executing,
            Some(running_session),
        );
        let matching_pipeline = pipeline_state("execution-1", PipelineStatus::Running);
        assert!(matches!(
            reconcile_execution_state(&running, Some(&matching_pipeline)),
            ExecutionReconciliation::Executing
        ));

        let stale_pipeline = pipeline_state("execution-stale", PipelineStatus::Running);
        assert!(matches!(
            reconcile_execution_state(&running, Some(&stale_pipeline)),
            ExecutionReconciliation::SessionLost
        ));
        let mut lost = running.clone();
        let lost_result = reconcile_execution_state(&lost, None);
        assert!(apply_execution_reconciliation(&mut lost, &lost_result));
        assert_eq!(
            lost.execution_session
                .as_ref()
                .map(|session| session.status.as_str()),
            Some("session_lost")
        );
        assert_eq!(
            lost.milestones[0].mid_stages[0].subtasks[0].status,
            project::SubtaskStatus::Pending
        );

        let awaiting = execution_project(
            "awaiting",
            empty_path,
            project::SubtaskStatus::AwaitingConfirmation,
            Some(execution_session(
                "awaiting_confirmation",
                "execution-2",
                "HEAD",
            )),
        );
        assert!(matches!(
            reconcile_execution_state(&awaiting, None),
            ExecutionReconciliation::AwaitingConfirmation
        ));

        let mut replanning = execution_project(
            "replanning",
            empty_path,
            project::SubtaskStatus::AwaitingConfirmation,
            Some(execution_session("replanning", "recovery-plan", "HEAD")),
        );
        replanning.workflow_state.recovery_state = Some(project::RecoveryState {
            phase: project::RecoveryPhase::Replanning,
            subtask_id: "subtask-1".to_string(),
            execution_id: "recovery-plan".to_string(),
            ..Default::default()
        });
        assert!(matches!(
            reconcile_execution_state(&replanning, None),
            ExecutionReconciliation::AwaitingConfirmation
        ));

        let recovery = replanning.workflow_state.recovery_state.as_mut().unwrap();
        recovery.phase = project::RecoveryPhase::Diagnosing;
        recovery.replan_attempted = true;
        let session = replanning.execution_session.as_mut().unwrap();
        session.active = false;
        session.status = "replan_ready".to_string();
        assert!(matches!(
            reconcile_execution_state(&replanning, None),
            ExecutionReconciliation::AwaitingConfirmation
        ));
    }

    #[test]
    fn lost_replanned_execution_stops_for_human() {
        let session = execution_session("recovering", "replan-execution", "HEAD");
        let mut proj = execution_project(
            "replan-execution-lost",
            Path::new(""),
            project::SubtaskStatus::Executing,
            Some(session),
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: "milestone-1".to_string(),
            run_status: project::AutopilotRunStatus::Running,
            last_action: String::new(),
            last_action_at: String::new(),
            error_message: String::new(),
            recovery_action: project::AutopilotRecoveryAction::RunAutomaticRecovery,
            job_id: "lost-runtime-job".to_string(),
            job_generation: 4,
            job_owner: project::AutopilotJobOwner::BackendRuntime,
            current_action_id: "lost-action".to_string(),
            current_action_kind: "run_error_recovery".to_string(),
            action_started_at: "2026-08-11T11:59:00Z".to_string(),
            ..Default::default()
        });
        proj.workflow_state.recovery_state = Some(project::RecoveryState {
            error_kind: project::RecoveryErrorKind::TestFailure,
            phase: project::RecoveryPhase::Repairing,
            subtask_id: "subtask-1".to_string(),
            execution_id: "replan-execution".to_string(),
            replan_attempted: true,
            replan_execution_attempted: true,
            ..Default::default()
        });

        assert!(apply_execution_reconciliation(
            &mut proj,
            &ExecutionReconciliation::SessionLost,
        ));
        assert_eq!(
            proj.workflow_state
                .recovery_state
                .as_ref()
                .map(|state| &state.phase),
            Some(&project::RecoveryPhase::WaitingHuman)
        );
        assert_eq!(
            proj.workflow_state
                .autopilot_state
                .as_ref()
                .map(|state| &state.recovery_action),
            Some(&project::AutopilotRecoveryAction::WaitHumanDecision)
        );
        let autopilot = proj.workflow_state.autopilot_state.as_ref().unwrap();
        assert_eq!(autopilot.job_owner, project::AutopilotJobOwner::None);
        assert!(autopilot.current_action_id.is_empty());
        assert!(autopilot.current_action_kind.is_empty());
        assert!(autopilot.action_started_at.is_empty());
    }

    #[test]
    fn dirty_git_workspace_is_not_ready() -> Result<(), String> {
        let repo = TempGitRepo::new("workspace")?;
        let clean = get_execution_workspace_status_inner(&repo.path_string())?;
        assert!(clean.working_tree_clean);
        assert!(clean.ready);

        std::fs::write(repo.path.join("tracked.txt"), "dirty\n")
            .map_err(|error| format!("写入脏工作区失败：{}", error))?;
        let dirty = get_execution_workspace_status_inner(&repo.path_string())?;
        assert!(!dirty.working_tree_clean);
        assert!(!dirty.ready);

        std::fs::write(repo.path.join("untracked.txt"), "untracked\n")
            .map_err(|error| format!("写入未跟踪文件失败：{}", error))?;
        let with_untracked = get_execution_workspace_status_inner(&repo.path_string())?;
        assert!(with_untracked
            .changes
            .iter()
            .any(|change| { change.path == "untracked.txt" && !change.tracked }));
        Ok(())
    }

    #[test]
    fn runtime_fix_workspace_classifies_managed_external_and_mixed() -> Result<(), String> {
        let repo = TempGitRepo::new("managed-workspace")?;
        let session = execution_session("awaiting_confirmation", "execution-1", &repo.head()?);
        let project = execution_project(
            "managed-workspace",
            &repo.path,
            project::SubtaskStatus::AwaitingConfirmation,
            Some(session),
        );
        std::fs::write(repo.path.join("tracked.txt"), "managed change\n")
            .map_err(|error| error.to_string())?;

        let managed = get_execution_workspace_status_for_project(&project)?;
        assert!(managed.git_metadata_ready);
        assert!(!managed.ready_for_new_execution);
        assert!(managed.has_managed_task_changes);
        assert!(!managed.has_external_changes);
        assert!(managed.changes.iter().all(|change| change.managed));
        assert!(managed.status_message.contains("待确认"));

        std::fs::write(repo.path.join("outside.txt"), "outside\n")
            .map_err(|error| error.to_string())?;
        let mixed = get_execution_workspace_status_for_project(&project)?;
        assert!(mixed.has_managed_task_changes);
        assert!(mixed.has_external_changes);
        assert!(mixed.status_message.contains("范围外"));
        Ok(())
    }

    #[test]
    fn runtime_fix_workspace_keeps_split_leaf_managed_after_prior_commit() -> Result<(), String> {
        let repo = TempGitRepo::new("split-managed-workspace")?;
        let split_base_commit = repo.head()?;

        std::fs::write(repo.path.join("first-leaf.txt"), "first leaf result\n")
            .map_err(|error| error.to_string())?;
        repo.git(&["add", "first-leaf.txt"])?;
        repo.git(&["commit", "--quiet", "-m", "complete first split leaf"])?;
        assert_ne!(repo.head()?, split_base_commit);

        let mut session =
            execution_session("awaiting_confirmation", "execution-2", &split_base_commit);
        session.subtask_id = "subtask-2".to_string();
        session.subtask_title = "第二个 split 叶子".to_string();
        session.subtask_index = 1;
        session.total_subtasks = 2;
        session.task_path = vec!["subtask-2".to_string()];
        session.top_level_task_id = "subtask-2".to_string();

        let mut project = execution_project(
            "split-managed-workspace",
            &repo.path,
            project::SubtaskStatus::Passed,
            Some(session),
        );
        let mut second_leaf = test_subtask(project::SubtaskStatus::AwaitingConfirmation);
        second_leaf.id = "subtask-2".to_string();
        second_leaf.title = "第二个 split 叶子".to_string();
        second_leaf.order = 2;
        project.milestones[0].mid_stages[0]
            .subtasks
            .push(second_leaf);

        std::fs::write(repo.path.join("tracked.txt"), "second leaf change\n")
            .map_err(|error| error.to_string())?;
        let status = get_execution_workspace_status_for_project(&project)?;

        assert!(status.has_managed_task_changes);
        assert!(!status.has_external_changes);
        assert!(status.changes.iter().all(|change| change.managed));
        Ok(())
    }

    #[test]
    fn workspace_distinguishes_missing_repo_from_missing_head() -> Result<(), String> {
        let path =
            std::env::temp_dir().join(format!("metheus-workspace-state-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path)
            .map_err(|error| format!("创建工作区测试目录失败：{}", error))?;
        let repo = TempGitRepo { path };

        let missing_repo = get_execution_workspace_status_inner(&repo.path_string())?;
        assert!(missing_repo
            .issues
            .contains(&project::ExecutionWorkspaceIssue::NotGitRepository));
        assert!(!missing_repo
            .issues
            .contains(&project::ExecutionWorkspaceIssue::NoCommits));

        repo.git(&["init", "--quiet"])?;
        repo.git(&["config", "user.name", "Metheus Test"])?;
        repo.git(&["config", "user.email", "metheus-test@example.invalid"])?;
        let missing_head = get_execution_workspace_status_inner(&repo.path_string())?;
        assert!(!missing_head
            .issues
            .contains(&project::ExecutionWorkspaceIssue::NotGitRepository));
        assert!(missing_head
            .issues
            .contains(&project::ExecutionWorkspaceIssue::NoCommits));
        Ok(())
    }

    #[tokio::test]
    async fn stale_background_execution_id_cannot_overwrite_current_session() -> Result<(), String>
    {
        let project_name = unique_project_name("stale-background");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let proj = execution_project(
            &project_name,
            Path::new(""),
            project::SubtaskStatus::Executing,
            Some(execution_session("executing", "execution-current", "HEAD")),
        );
        crate::save_project(&proj)?;
        let pipeline = Arc::new(Mutex::new(Some(pipeline_state(
            "execution-stale",
            PipelineStatus::Running,
        ))));

        let failure = BackgroundExecutionFailure::new(
            project::RecoveryErrorKind::ExecutionError,
            "旧后台任务失败".to_string(),
        );
        finalize_background_execution_failure(
            &project_name,
            "milestone-1",
            "mid-1",
            "subtask-1",
            "测试小阶段",
            0,
            1,
            "execution-stale",
            &failure,
            pipeline,
            project::OperationSource::User,
        )
        .await?;

        let persisted = crate::load_project(&project_name)?;
        assert_eq!(
            persisted
                .execution_session
                .as_ref()
                .map(|session| session.execution_id.as_str()),
            Some("execution-current")
        );
        assert_eq!(
            persisted.milestones[0].mid_stages[0].subtasks[0].status,
            project::SubtaskStatus::Executing
        );
        assert!(persisted.execution_history.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn retry_prefers_session_baseline_and_falls_back_to_head() -> Result<(), String> {
        let repo = TempGitRepo::new("retry-session")?;
        let baseline = repo.head()?;
        std::fs::write(repo.path.join("tracked.txt"), "second commit\n")
            .map_err(|error| format!("写入第二次提交失败：{}", error))?;
        repo.git(&["add", "tracked.txt"])?;
        repo.git(&["commit", "--quiet", "-m", "second"])?;
        assert_ne!(repo.head()?, baseline);

        let project_name = unique_project_name("retry-session");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let proj = execution_project(
            &project_name,
            &repo.path,
            project::SubtaskStatus::Rejected,
            Some(execution_session(
                "execution_failed",
                "execution-retry",
                &baseline,
            )),
        );
        crate::save_project(&proj)?;
        let updated = retry_current_subtask(project_name).await?;
        assert_eq!(repo.head()?, baseline);
        assert_eq!(
            updated.milestones[0].mid_stages[0].subtasks[0].status,
            project::SubtaskStatus::Pending
        );
        assert_eq!(
            updated.milestones[0].mid_stages[0].subtasks[0].retry_count,
            1
        );
        assert!(updated.execution_session.is_none());
        assert!(updated
            .execution_history
            .iter()
            .any(|entry| entry.event_type == project::ExecutionEventType::RetryScheduled));

        let head_repo = TempGitRepo::new("retry-head")?;
        std::fs::write(head_repo.path.join("tracked.txt"), "dirty tracked\n")
            .map_err(|error| format!("写入 HEAD 回退测试修改失败：{}", error))?;
        std::fs::write(head_repo.path.join("untracked.txt"), "dirty untracked\n")
            .map_err(|error| format!("写入 HEAD 回退测试新文件失败：{}", error))?;
        let head_project_name = unique_project_name("retry-head");
        let _head_guard = ProjectDataGuard::new(&head_project_name)?;
        let head_project = execution_project(
            &head_project_name,
            &head_repo.path,
            project::SubtaskStatus::Rejected,
            None,
        );
        crate::save_project(&head_project)?;
        retry_current_subtask(head_project_name).await?;
        let workspace = get_execution_workspace_status_inner(&head_repo.path_string())?;
        assert!(workspace.working_tree_clean);
        assert!(workspace.ready);
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn in_stop_primitives_terminate_process_and_restore_clean_git() -> Result<(), String> {
        let repo = TempGitRepo::new("in-stop")?;
        let baseline = repo.head()?;
        std::fs::write(repo.path.join("tracked.txt"), "execution change\n")
            .map_err(|error| format!("写入 In Stop 测试修改失败：{}", error))?;
        std::fs::write(repo.path.join("untracked.txt"), "execution output\n")
            .map_err(|error| format!("写入 In Stop 测试新文件失败：{}", error))?;

        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .map_err(|error| format!("启动 In Stop 测试进程失败：{}", error))?;
        terminate_execution_process(child.id()).await?;
        child
            .wait()
            .map_err(|error| format!("等待 In Stop 测试进程退出失败：{}", error))?;

        let outcome = restore_git_execution_baseline(&repo.path_string(), &baseline)
            .map_err(|outcome| outcome.error_message())?;
        assert!(outcome.is_restored());
        assert!(outcome.stash_created);
        assert_eq!(repo.head()?, baseline);
        let workspace = get_execution_workspace_status_inner(&repo.path_string())?;
        assert!(workspace.working_tree_clean);
        assert!(workspace.ready);
        Ok(())
    }

    #[tokio::test]
    async fn duplicate_execution_start_is_rejected_before_launch() -> Result<(), String> {
        let pipeline = Arc::new(Mutex::new(Some(pipeline_state(
            "execution-active",
            PipelineStatus::Running,
        ))));

        let result = acquire_pipeline_start(&pipeline).await;
        assert!(result.is_err());
        assert!(result
            .err()
            .is_some_and(|error| error.contains("已有小阶段正在执行")));
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn in_stop_transition_restores_project_and_persists_history() -> Result<(), String> {
        let repo = TempGitRepo::new("in-stop-transition")?;
        let baseline = repo.head()?;
        std::fs::write(repo.path.join("tracked.txt"), "execution change\n")
            .map_err(|error| format!("写入 In Stop 跟踪修改失败：{}", error))?;
        std::fs::write(repo.path.join("untracked.txt"), "execution output\n")
            .map_err(|error| format!("写入 In Stop 未跟踪文件失败：{}", error))?;

        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .map_err(|error| format!("启动 In Stop 测试进程失败：{}", error))?;
        let project_name = unique_project_name("in-stop-transition");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = execution_project(
            &project_name,
            &repo.path,
            project::SubtaskStatus::Executing,
            Some(execution_session("executing", "execution-stop", &baseline)),
        );
        let mut pipeline_value = pipeline_state("execution-stop", PipelineStatus::Running);
        pipeline_value.child_pid = Some(child.id());
        let pipeline = Arc::new(Mutex::new(Some(pipeline_value)));

        request_in_stop_with_pipeline_state(pipeline.clone(), &mut proj).await?;
        child
            .wait()
            .map_err(|error| format!("等待 In Stop 测试进程退出失败：{}", error))?;

        assert_eq!(
            proj.workflow_state.current_step,
            project::WorkflowStep::PauseDecision
        );
        assert!(proj.execution_session.is_none());
        assert_eq!(
            proj.milestones[0].mid_stages[0].subtasks[0].status,
            project::SubtaskStatus::Pending
        );
        assert_eq!(
            proj.pause_context
                .as_ref()
                .map(|context| context.pause_type.as_str()),
            Some("in_stop")
        );
        assert!(proj
            .execution_history
            .iter()
            .any(|entry| entry.event_type == project::ExecutionEventType::UserInStop));

        let pipeline_after = pipeline.lock().await;
        assert_eq!(
            pipeline_after.as_ref().map(|state| &state.status),
            Some(&PipelineStatus::Paused)
        );
        drop(pipeline_after);

        crate::save_project(&proj)?;
        let reloaded = crate::load_project(&project_name)?;
        assert_eq!(
            reloaded.workflow_state.current_step,
            project::WorkflowStep::PauseDecision
        );
        assert!(reloaded
            .execution_history
            .iter()
            .any(|entry| entry.event_type == project::ExecutionEventType::UserInStop));
        assert_eq!(repo.head()?, baseline);
        let workspace = get_execution_workspace_status_inner(&repo.path_string())?;
        assert!(workspace.working_tree_clean);
        assert!(workspace.ready);
        Ok(())
    }

    #[tokio::test]
    async fn ed_stop_requires_running_session_and_is_idempotent() -> Result<(), String> {
        let mut executing = execution_project(
            "ed-stop",
            Path::new(""),
            project::SubtaskStatus::Executing,
            Some(execution_session("executing", "execution-ed", "HEAD")),
        );
        let pipeline = Arc::new(Mutex::new(Some(pipeline_state(
            "execution-ed",
            PipelineStatus::Running,
        ))));

        request_ed_stop_with_pipeline_state(pipeline.clone(), &mut executing).await?;
        assert_eq!(
            executing
                .pause_context
                .as_ref()
                .map(|context| context.pending_action.as_str()),
            Some("ed_stop_requested")
        );
        assert_eq!(executing.execution_history.len(), 1);
        assert_eq!(
            executing.workflow_state.pause_reason,
            project::PauseReason::EDStop
        );
        assert_eq!(executing.workflow_state.data_revision, 1);

        request_ed_stop_with_pipeline_state(pipeline, &mut executing).await?;
        assert_eq!(executing.execution_history.len(), 1);
        assert_eq!(executing.workflow_state.data_revision, 1);

        let mut planning = execution_project(
            "ed-stop-planning",
            Path::new(""),
            project::SubtaskStatus::Pending,
            None,
        );
        let planning_pipeline = Arc::new(Mutex::new(None));
        let result = request_ed_stop_with_pipeline_state(planning_pipeline, &mut planning).await;
        assert!(result.is_err());
        assert!(planning.execution_history.is_empty());
        assert!(planning.pause_context.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn first_execution_failure_is_retryable_without_retry_count() -> Result<(), String> {
        let repo = TempGitRepo::new("first-fail")?;
        let baseline = repo.head()?;
        std::fs::write(repo.path.join("tracked.txt"), "dirty after fail\n")
            .map_err(|error| format!("写入失败残留失败：{}", error))?;
        std::fs::write(repo.path.join("untracked.txt"), "untracked residue\n")
            .map_err(|error| format!("写入未跟踪残留失败：{}", error))?;

        let project_name = unique_project_name("first-fail");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut session = execution_session("execution_failed", "execution-first", &baseline);
        session.active = false;
        session.failure_message = "执行超时".to_string();
        let mut proj = execution_project(
            &project_name,
            &repo.path,
            project::SubtaskStatus::Pending,
            Some(session),
        );
        // 首次失败：retry_count 必须为 0 仍可恢复
        proj.milestones[0].mid_stages[0].subtasks[0].retry_count = 0;
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: "milestone-1".to_string(),
            run_status: project::AutopilotRunStatus::ErrorStopped,
            last_action: "执行超时".to_string(),
            last_action_at: "2026-07-20T00:00:00Z".to_string(),
            error_message: "执行超时".to_string(),
            recovery_action: project::AutopilotRecoveryAction::RestoreExecutionBaseline,
            ..Default::default()
        });
        crate::save_project(&proj)?;

        let updated = retry_current_subtask(project_name.clone()).await?;
        assert_eq!(repo.head()?, baseline);
        assert_eq!(
            updated.milestones[0].mid_stages[0].subtasks[0].retry_count,
            1
        );
        assert!(updated.execution_session.is_none());
        assert_eq!(
            updated
                .workflow_state
                .autopilot_state
                .as_ref()
                .map(|ap| &ap.run_status),
            Some(&project::AutopilotRunStatus::Running)
        );
        assert_eq!(
            updated
                .workflow_state
                .autopilot_state
                .as_ref()
                .map(|ap| &ap.recovery_action),
            Some(&project::AutopilotRecoveryAction::None)
        );
        let workspace = get_execution_workspace_status_inner(&repo.path_string())?;
        assert!(workspace.working_tree_clean);
        Ok(())
    }

    #[tokio::test]
    async fn session_lost_acknowledge_restores_baseline() -> Result<(), String> {
        let repo = TempGitRepo::new("session-lost-ack")?;
        let baseline = repo.head()?;
        std::fs::write(repo.path.join("tracked.txt"), "interrupted change\n")
            .map_err(|error| format!("写入失联残留失败：{}", error))?;

        let project_name = unique_project_name("session-lost-ack");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut session = execution_session("session_lost", "execution-lost", &baseline);
        session.active = false;
        session.failure_message = "执行进程失联".to_string();
        let mut proj = execution_project(
            &project_name,
            &repo.path,
            project::SubtaskStatus::Pending,
            Some(session),
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: "milestone-1".to_string(),
            run_status: project::AutopilotRunStatus::ErrorStopped,
            last_action: "session lost".to_string(),
            last_action_at: "2026-07-20T00:00:00Z".to_string(),
            error_message: "失联".to_string(),
            recovery_action: project::AutopilotRecoveryAction::RestoreExecutionBaseline,
            ..Default::default()
        });
        crate::save_project(&proj)?;

        let updated = acknowledge_execution_recovery_inner(project_name.clone()).await?;
        assert!(updated.execution_session.is_none());
        assert_eq!(repo.head()?, baseline);
        let workspace = get_execution_workspace_status_inner(&repo.path_string())?;
        assert!(workspace.working_tree_clean);
        assert_eq!(
            updated
                .workflow_state
                .autopilot_state
                .as_ref()
                .map(|ap| &ap.run_status),
            Some(&project::AutopilotRunStatus::Running)
        );
        let revision = updated.workflow_state.data_revision;
        let repeated = acknowledge_execution_recovery_inner(project_name).await?;
        assert_eq!(repeated.workflow_state.data_revision, revision);
        assert_eq!(repo.head()?, baseline);
        Ok(())
    }

    #[test]
    fn phase1_runtime_contract_recovery_preview_uses_backend_presentation_without_mutation(
    ) -> Result<(), String> {
        let repo = TempGitRepo::new("recovery-preview")?;
        let baseline = repo.head()?;
        std::fs::write(repo.path.join("tracked.txt"), "managed change\n")
            .map_err(|error| format!("写入受管预览变更失败：{error}"))?;
        std::fs::write(repo.path.join("external.txt"), "external change\n")
            .map_err(|error| format!("写入外部预览变更失败：{error}"))?;
        let project_name = unique_project_name("recovery-preview");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut session = execution_session("execution_failed", "preview-execution", &baseline);
        session.active = false;
        let project = execution_project(
            &project_name,
            &repo.path,
            project::SubtaskStatus::Pending,
            Some(session),
        );
        crate::save_project(&project)?;

        let impact = preview_execution_recovery_impact_inner(&project_name)?;
        assert_eq!(impact.baseline_commit, baseline);
        assert!(impact.managed_changes.contains(&"tracked.txt".to_string()));
        assert!(impact
            .external_changes
            .contains(&"external.txt".to_string()));
        assert!(impact.untracked_files.contains(&"external.txt".to_string()));
        assert!(impact.creates_safety_stash);
        assert!(impact.has_destructive_changes);
        assert_eq!(impact.action_label, "恢复执行基线");
        assert_eq!(impact.confirmation_title, "确认恢复执行基线");
        assert!(impact.presentation_description.contains("安全暂存"));
        assert!(impact.safety_stash_summary.contains("安全暂存"));
        assert_eq!(
            std::fs::read_to_string(repo.path.join("tracked.txt"))
                .map_err(|error| format!("读取预览后受管文件失败：{error}"))?,
            "managed change\n"
        );
        Ok(())
    }

    #[test]
    fn clean_recovery_preview_can_skip_confirmation() -> Result<(), String> {
        let repo = TempGitRepo::new("clean-recovery-preview")?;
        let baseline = repo.head()?;
        let project_name = unique_project_name("clean-recovery-preview");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut session = execution_session("execution_failed", "clean-preview", &baseline);
        session.active = false;
        let project = execution_project(
            &project_name,
            &repo.path,
            project::SubtaskStatus::Pending,
            Some(session),
        );
        crate::save_project(&project)?;

        let impact = preview_execution_recovery_impact_inner(&project_name)?;
        assert!(!impact.has_destructive_changes);
        assert!(impact.affected_files.is_empty());
        assert!(!impact.creates_safety_stash);
        Ok(())
    }

    #[test]
    fn recovery_rejects_preview_after_workspace_changes() -> Result<(), String> {
        let repo = TempGitRepo::new("stale-recovery-preview")?;
        let baseline = repo.head()?;
        let project_name = unique_project_name("stale-recovery-preview");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut session = execution_session("execution_failed", "stale-preview", &baseline);
        session.active = false;
        let project = execution_project(
            &project_name,
            &repo.path,
            project::SubtaskStatus::Pending,
            Some(session),
        );
        crate::save_project(&project)?;

        std::fs::write(repo.path.join("tracked.txt"), "changed before preview\n")
            .map_err(|error| format!("写入预览前变更失败：{error}"))?;
        let preview = preview_execution_recovery_impact_inner(&project_name)?;
        std::fs::write(repo.path.join("tracked.txt"), "changed after preview\n")
            .map_err(|error| format!("写入预览后变更失败：{error}"))?;
        let latest = crate::load_project(&project_name)?;
        let error = verify_execution_recovery_preview(
            &latest,
            "acknowledge_execution_recovery",
            Some(&preview.state_fingerprint),
        )
        .expect_err("工作区变化后必须拒绝旧预览");

        assert!(error.contains("预览已过期"));
        assert_eq!(repo.head()?, baseline);
        assert_eq!(
            std::fs::read_to_string(repo.path.join("tracked.txt"))
                .map_err(|error| format!("读取拒绝后的工作区失败：{error}"))?,
            "changed after preview\n"
        );
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_automatic_recovery_is_idempotent_and_clears_pipeline() -> Result<(), String>
    {
        let repo = TempGitRepo::new("concurrent-recovery")?;
        let baseline = repo.head()?;
        std::fs::write(repo.path.join("tracked.txt"), "failed execution\n")
            .map_err(|error| format!("写入并发恢复残留失败：{error}"))?;
        let project_name = unique_project_name("concurrent-recovery");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut session = execution_session("execution_failed", "concurrent", &baseline);
        session.active = false;
        let project = execution_project(
            &project_name,
            &repo.path,
            project::SubtaskStatus::Pending,
            Some(session),
        );
        crate::save_project(&project)?;
        let pipeline = Arc::new(Mutex::new(Some(pipeline_state(
            "concurrent",
            PipelineStatus::Failed,
        ))));

        let (first, second) = tokio::join!(
            acknowledge_execution_recovery_with_pipeline(&pipeline, project_name.clone()),
            acknowledge_execution_recovery_with_pipeline(&pipeline, project_name.clone()),
        );
        let first = first?;
        let second = second?;

        assert!(pipeline.lock().await.is_none());
        assert!(first.execution_session.is_none());
        assert!(second.execution_session.is_none());
        assert_eq!(
            first.workflow_state.data_revision,
            second.workflow_state.data_revision
        );
        assert_eq!(repo.head()?, baseline);
        assert!(get_execution_workspace_status_inner(&repo.path_string())?.working_tree_clean);
        Ok(())
    }

    #[tokio::test]
    async fn workspace_refresh_resumes_without_preparing_git_again() -> Result<(), String> {
        let repo = TempGitRepo::new("workspace-refresh")?;
        let project_name = unique_project_name("workspace-refresh");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = execution_project(
            &project_name,
            &repo.path,
            project::SubtaskStatus::Pending,
            None,
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: "milestone-1".to_string(),
            run_status: project::AutopilotRunStatus::ErrorStopped,
            last_action: "dirty".to_string(),
            last_action_at: String::new(),
            error_message: "dirty".to_string(),
            recovery_action: project::AutopilotRecoveryAction::ResolveWorkspaceChanges,
            ..Default::default()
        });
        crate::save_project(&proj)?;

        let status = refresh_execution_workspace_inner(project_name.clone()).await?;
        assert!(status.ready);
        let updated = crate::load_project(&project_name)?;
        let autopilot = updated.workflow_state.autopilot_state.unwrap();
        assert_eq!(autopilot.run_status, project::AutopilotRunStatus::Running);
        assert_eq!(
            autopilot.recovery_action,
            project::AutopilotRecoveryAction::None
        );
        Ok(())
    }

    #[tokio::test]
    async fn workspace_refresh_is_read_only_for_non_git_directory() -> Result<(), String> {
        let path = std::env::temp_dir().join(format!(
            "metheus-refresh-read-only-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path)
            .map_err(|error| format!("创建刷新测试目录失败：{}", error))?;
        let project_name = unique_project_name("workspace-refresh-read-only");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = project::Project::new(&project_name);
        proj.project_path = path.to_string_lossy().to_string();
        crate::save_project(&proj)?;

        let status = refresh_execution_workspace_inner(project_name).await?;
        assert!(!status.is_git_repo);
        assert!(!path.join(".git").exists());
        std::fs::remove_dir_all(&path)
            .map_err(|error| format!("清理刷新测试目录失败：{}", error))?;
        Ok(())
    }

    #[test]
    fn structured_test_failure_enters_automatic_recovery() -> Result<(), String> {
        let mut proj = execution_project(
            "quality-recovery",
            Path::new(""),
            project::SubtaskStatus::AwaitingConfirmation,
            Some(execution_session(
                "awaiting_confirmation",
                "execution-quality",
                "abc123",
            )),
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: "milestone-1".to_string(),
            run_status: project::AutopilotRunStatus::Running,
            last_action: String::new(),
            last_action_at: String::new(),
            error_message: String::new(),
            recovery_action: project::AutopilotRecoveryAction::None,
            ..Default::default()
        });
        let subtask = &mut proj.milestones[0].mid_stages[0].subtasks[0];
        subtask.execution_result = Some(project::ExecutionResult {
            success: true,
            output: String::new(),
            error_log: String::new(),
            file_changes: vec!["tracked.txt".to_string()],
            ..Default::default()
        });
        subtask.test_result = Some(project::TestResult {
            passed: false,
            issues: vec!["assertion failed".to_string()],
            automated_test_status: project::AutomatedTestStatus::Failed,
            ..Default::default()
        });

        assert!(crate::recovery::ensure_quality_recovery(
            &mut proj,
            "test failed"
        )?);
        let recovery = proj
            .workflow_state
            .recovery_state
            .as_ref()
            .ok_or_else(|| "质量恢复状态意外丢失".to_string())?;
        assert_eq!(recovery.error_kind, project::RecoveryErrorKind::TestFailure);
        assert_eq!(recovery.phase, project::RecoveryPhase::Diagnosing);
        assert_eq!(recovery.max_attempts, 2);
        assert_eq!(
            proj.workflow_state
                .autopilot_state
                .as_ref()
                .map(|state| &state.recovery_action),
            Some(&project::AutopilotRecoveryAction::RunAutomaticRecovery)
        );
        Ok(())
    }

    #[test]
    fn unavailable_test_enters_human_block() -> Result<(), String> {
        let mut proj = execution_project(
            "quality-unavailable",
            Path::new(""),
            project::SubtaskStatus::AwaitingConfirmation,
            Some(execution_session(
                "awaiting_confirmation",
                "execution-unavailable",
                "abc123",
            )),
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: "milestone-1".to_string(),
            run_status: project::AutopilotRunStatus::Running,
            last_action: String::new(),
            last_action_at: String::new(),
            error_message: String::new(),
            recovery_action: project::AutopilotRecoveryAction::None,
            ..Default::default()
        });
        let subtask = &mut proj.milestones[0].mid_stages[0].subtasks[0];
        subtask.execution_result = Some(project::ExecutionResult {
            success: true,
            output: String::new(),
            error_log: String::new(),
            file_changes: vec!["tracked.txt".to_string()],
            ..Default::default()
        });
        subtask.test_result = Some(project::TestResult {
            passed: false,
            issues: vec!["environment unavailable".to_string()],
            automated_test_status: project::AutomatedTestStatus::Unavailable,
            ..Default::default()
        });

        assert!(!crate::recovery::ensure_quality_recovery(
            &mut proj,
            "test unavailable"
        )?);
        assert_eq!(
            proj.workflow_state
                .recovery_state
                .as_ref()
                .map(|state| &state.phase),
            Some(&project::RecoveryPhase::WaitingHuman)
        );
        assert_eq!(
            proj.workflow_state
                .autopilot_state
                .as_ref()
                .map(|state| &state.recovery_action),
            Some(&project::AutopilotRecoveryAction::WaitHumanDecision)
        );
        Ok(())
    }

    #[test]
    fn review_protocol_failure_initializes_bounded_validation_recovery() -> Result<(), String> {
        let mut proj = execution_project(
            "quality-review-protocol",
            Path::new(""),
            project::SubtaskStatus::AwaitingConfirmation,
            Some(execution_session(
                "awaiting_confirmation",
                "review-protocol",
                "abc123",
            )),
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            run_status: project::AutopilotRunStatus::Running,
            ..Default::default()
        });
        let subtask = &mut proj.milestones[0].mid_stages[0].subtasks[0];
        subtask.execution_result = Some(project::ExecutionResult {
            success: true,
            file_changes: vec!["tracked.txt".to_string()],
            ..Default::default()
        });
        subtask.test_result = Some(project::TestResult {
            automated_test_status: project::AutomatedTestStatus::Passed,
            review_status: project::ReviewStatus::Failed,
            review_failure_kind: Some(project::ReviewFailureKind::FieldTypeMismatch),
            review_protocol_attempts: 1,
            review_diagnostic_summary: "$.review_issues[0].actual type mismatch".to_string(),
            ..Default::default()
        });

        assert!(crate::recovery::ensure_quality_recovery(
            &mut proj,
            "review protocol failed"
        )?);
        let recovery = proj.workflow_state.recovery_state.as_ref().unwrap();
        assert_eq!(
            recovery.error_kind,
            project::RecoveryErrorKind::ReviewProtocolFailure
        );
        assert_eq!(recovery.phase, project::RecoveryPhase::Retesting);
        assert_eq!(recovery.attempt, 0);
        assert_eq!(recovery.validation_retry_count, 0);
        assert_eq!(recovery.max_validation_retries, 2);
        assert!(recovery.next_validation_retry_at.is_some());
        assert_eq!(
            recovery.validation_strategies,
            vec![
                project::ValidationRetryStrategy::DeterministicNormalization,
                project::ValidationRetryStrategy::ProtocolRepair,
            ]
        );

        proj.workflow_state.recovery_state = None;
        proj.milestones[0].mid_stages[0].subtasks[0]
            .test_result
            .as_mut()
            .unwrap()
            .review_failure_kind = Some(project::ReviewFailureKind::Authentication);
        assert!(!crate::recovery::ensure_quality_recovery(
            &mut proj,
            "review authentication failed"
        )?);
        let blocked = proj.workflow_state.recovery_state.as_ref().unwrap();
        assert_eq!(
            blocked.error_kind,
            project::RecoveryErrorKind::ReviewServiceBlocked
        );
        assert_eq!(blocked.phase, project::RecoveryPhase::WaitingHuman);
        assert_eq!(blocked.validation_retry_count, 0);
        Ok(())
    }

    #[test]
    fn review_retry_failure_never_spends_code_repair_attempts() -> Result<(), String> {
        let session = execution_session("recovering", "review-retry", "abc123");
        let mut proj = execution_project(
            "review-retry",
            Path::new(""),
            project::SubtaskStatus::Executing,
            Some(session.clone()),
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            run_status: project::AutopilotRunStatus::Running,
            recovery_action: project::AutopilotRecoveryAction::RunAutomaticRecovery,
            ..Default::default()
        });
        let pending_execution = project::ExecutionResult {
            success: true,
            output: "pending repair result".to_string(),
            file_changes: vec!["tracked.txt".to_string()],
            ..Default::default()
        };
        proj.workflow_state.recovery_state = Some(project::RecoveryState {
            error_kind: project::RecoveryErrorKind::ReviewTransientFailure,
            phase: project::RecoveryPhase::Retesting,
            attempt: 1,
            max_attempts: 2,
            subtask_id: "subtask-1".to_string(),
            execution_id: "review-retry".to_string(),
            validation_retry_count: 1,
            max_validation_retries: 3,
            pending_execution_result: Some(pending_execution.clone()),
            ..Default::default()
        });
        let failed_review = project::TestResult {
            automated_test_status: project::AutomatedTestStatus::Passed,
            review_status: project::ReviewStatus::Failed,
            review_failure_kind: Some(project::ReviewFailureKind::Network),
            review_diagnostic_summary: "network unavailable".to_string(),
            ..Default::default()
        };

        crate::recovery::finish_retest(&mut proj, &session, "review-retry", failed_review.clone())?;
        let recovery = proj.workflow_state.recovery_state.as_ref().unwrap();
        assert_eq!(recovery.phase, project::RecoveryPhase::Retesting);
        assert_eq!(recovery.attempt, 1);
        assert_eq!(recovery.validation_retry_count, 1);
        assert!(recovery.attempt_history.is_empty());
        let preserved_execution = recovery.pending_execution_result.as_ref().unwrap();
        assert_eq!(preserved_execution.output, pending_execution.output);
        assert_eq!(
            preserved_execution.file_changes,
            pending_execution.file_changes
        );
        assert!(recovery.next_validation_retry_at.is_some());

        proj.workflow_state
            .recovery_state
            .as_mut()
            .unwrap()
            .validation_retry_count = 3;
        crate::recovery::finish_retest(&mut proj, &session, "review-retry", failed_review)?;
        let recovery = proj.workflow_state.recovery_state.as_ref().unwrap();
        assert_eq!(recovery.phase, project::RecoveryPhase::WaitingHuman);
        assert_eq!(recovery.attempt, 1);
        assert!(!recovery.replan_attempted);
        assert!(recovery.attempt_history.is_empty());
        assert!(recovery.next_validation_retry_at.is_none());
        Ok(())
    }

    #[test]
    fn failed_retest_keeps_pending_execution_out_of_task_facts() -> Result<(), String> {
        let session = execution_session("recovering", "recovery-pending", "abc123");
        let mut proj = execution_project(
            "recovery-pending",
            Path::new(""),
            project::SubtaskStatus::Executing,
            Some(session.clone()),
        );
        let original_execution = project::ExecutionResult {
            success: true,
            output: "original execution".to_string(),
            ..Default::default()
        };
        let pending_execution = project::ExecutionResult {
            success: true,
            output: "repair execution pending retest".to_string(),
            ..Default::default()
        };
        let subtask = &mut proj.milestones[0].mid_stages[0].subtasks[0];
        subtask.execution_result = Some(original_execution.clone());
        proj.workflow_state.recovery_state = Some(project::RecoveryState {
            error_kind: project::RecoveryErrorKind::TestFailure,
            phase: project::RecoveryPhase::Retesting,
            attempt: 1,
            max_attempts: 2,
            subtask_id: "subtask-1".to_string(),
            execution_id: "recovery-pending".to_string(),
            pending_execution_result: Some(pending_execution.clone()),
            ..Default::default()
        });

        let rolled_back = crate::recovery::finish_retest(
            &mut proj,
            &session,
            "recovery-pending",
            project::TestResult {
                automated_test_status: project::AutomatedTestStatus::Failed,
                issues: vec!["retest failed".to_string()],
                ..Default::default()
            },
        )?;
        assert!(!rolled_back);
        let subtask = &proj.milestones[0].mid_stages[0].subtasks[0];
        assert_eq!(
            subtask
                .execution_result
                .as_ref()
                .map(|result| &result.output),
            Some(&original_execution.output)
        );
        assert!(subtask.test_result.is_some());
        let recovery = proj.workflow_state.recovery_state.as_ref().unwrap();
        assert_eq!(
            recovery
                .pending_execution_result
                .as_ref()
                .map(|result| &result.output),
            Some(&pending_execution.output)
        );
        Ok(())
    }

    #[test]
    fn phase1_runtime_contract_successful_retest_clears_recovery_immediately() -> Result<(), String>
    {
        let session = execution_session("recovering", "recovery-success", "abc123");
        let mut proj = execution_project(
            "recovery-success",
            Path::new(""),
            project::SubtaskStatus::Executing,
            Some(session.clone()),
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: "milestone-1".to_string(),
            run_status: project::AutopilotRunStatus::Running,
            last_action: "retesting".to_string(),
            last_action_at: String::new(),
            error_message: String::new(),
            recovery_action: project::AutopilotRecoveryAction::RunAutomaticRecovery,
            ..Default::default()
        });
        proj.workflow_state.recovery_state = Some(project::RecoveryState {
            error_kind: project::RecoveryErrorKind::TestFailure,
            phase: project::RecoveryPhase::Retesting,
            attempt: 1,
            max_attempts: 2,
            subtask_id: "subtask-1".to_string(),
            execution_id: "recovery-success".to_string(),
            baseline_commit: "abc123".to_string(),
            ..Default::default()
        });
        proj.milestones[0].mid_stages[0].subtasks[0].execution_result =
            Some(project::ExecutionResult {
                success: true,
                output: "fixed".to_string(),
                error_log: String::new(),
                file_changes: vec!["tracked.txt".to_string()],
                ..Default::default()
            });
        let test = project::TestResult {
            passed: true,
            review_passed: true,
            automated_test_status: project::AutomatedTestStatus::Passed,
            verification_kind: project::VerificationKind::AutomatedTestAndReview,
            ..Default::default()
        };

        crate::recovery::finish_retest(&mut proj, &session, "recovery-success", test)?;
        assert!(proj.workflow_state.recovery_state.is_none());
        assert_eq!(
            proj.milestones[0].mid_stages[0].subtasks[0].status,
            project::SubtaskStatus::AwaitingConfirmation
        );
        assert_eq!(
            proj.workflow_state
                .autopilot_state
                .as_ref()
                .map(|state| &state.recovery_action),
            Some(&project::AutopilotRecoveryAction::None)
        );
        Ok(())
    }

    #[test]
    fn successful_review_retry_preserves_execution_without_code_repair() -> Result<(), String> {
        let session = execution_session("recovering", "review-recovery-success", "abc123");
        let mut proj = execution_project(
            "review-recovery-success",
            Path::new(""),
            project::SubtaskStatus::Executing,
            Some(session.clone()),
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            run_status: project::AutopilotRunStatus::Running,
            recovery_action: project::AutopilotRecoveryAction::RunAutomaticRecovery,
            ..Default::default()
        });
        let original_execution = project::ExecutionResult {
            success: true,
            output: "original successful execution".to_string(),
            file_changes: vec!["tracked.txt".to_string()],
            ..Default::default()
        };
        proj.workflow_state.recovery_state = Some(project::RecoveryState {
            error_kind: project::RecoveryErrorKind::ReviewTransientFailure,
            phase: project::RecoveryPhase::Retesting,
            attempt: 0,
            max_attempts: 2,
            subtask_id: "subtask-1".to_string(),
            execution_id: "review-recovery-success".to_string(),
            validation_retry_count: 2,
            max_validation_retries: 3,
            pending_execution_result: Some(original_execution.clone()),
            ..Default::default()
        });
        let test = project::TestResult {
            passed: true,
            review_passed: true,
            review_status: project::ReviewStatus::Completed,
            automated_test_status: project::AutomatedTestStatus::Passed,
            verification_kind: project::VerificationKind::AutomatedTestAndReview,
            ..Default::default()
        };

        crate::recovery::finish_retest(&mut proj, &session, "review-recovery-success", test)?;

        assert!(proj.workflow_state.recovery_state.is_none());
        let subtask = &proj.milestones[0].mid_stages[0].subtasks[0];
        assert_eq!(subtask.status, project::SubtaskStatus::AwaitingConfirmation);
        let preserved_execution = subtask.execution_result.as_ref().unwrap();
        assert_eq!(preserved_execution.output, original_execution.output);
        assert_eq!(
            preserved_execution.file_changes,
            original_execution.file_changes
        );
        assert!(preserved_execution.success);
        let current_session = proj.execution_session.as_ref().unwrap();
        assert_eq!(current_session.status, "awaiting_confirmation");
        assert!(current_session.active);
        assert!(!proj.execution_history.iter().any(|entry| matches!(
            entry.event_type,
            project::ExecutionEventType::RepairAttemptStarted
                | project::ExecutionEventType::RepairAttemptCompleted
        )));
        assert_eq!(
            proj.workflow_state
                .autopilot_state
                .as_ref()
                .map(|state| &state.recovery_action),
            Some(&project::AutopilotRecoveryAction::None)
        );
        Ok(())
    }

    #[test]
    fn regression_rollback_restores_files_and_rebuilds_evidence() -> Result<(), String> {
        let repo = TempGitRepo::new("recovery-regression-rollback")?;
        let session = execution_session("recovering", "recovery-regression", "abc123");
        let mut proj = execution_project(
            "recovery-regression",
            &repo.path,
            project::SubtaskStatus::Executing,
            Some(session.clone()),
        );
        let original_execution = project::ExecutionResult {
            success: true,
            output: "original execution".to_string(),
            file_changes: vec!["tracked.txt".to_string()],
            ..Default::default()
        };
        let original_issue = project::RecoveryIssue {
            id: "unstructured:original failure".to_string(),
            actual: "original failure".to_string(),
            ..Default::default()
        };
        let checkpoint_id =
            crate::recovery_checkpoint::create(&repo.path_string(), &["tracked.txt".to_string()])?;
        std::fs::write(repo.path.join("tracked.txt"), "regressing repair\n")
            .map_err(|error| error.to_string())?;

        let subtask = &mut proj.milestones[0].mid_stages[0].subtasks[0];
        subtask.execution_result = Some(original_execution.clone());
        subtask.test_result = Some(project::TestResult {
            passed: false,
            issues: vec!["original failure".to_string()],
            ..Default::default()
        });
        subtask.acceptance_ledger = vec![project::AcceptanceLedgerItem::default()];
        subtask.human_verification = Some(project::HumanVerification {
            verification_kind: project::VerificationKind::HumanOverride,
            verification_reason: "stale evidence".to_string(),
            verified_at: "now".to_string(),
            original_test_failure: "original failure".to_string(),
            resolution: project::HumanResolution::ConfirmActualPass,
            accepted_criteria: vec![],
            dependency_check: String::new(),
            action_source: String::new(),
            execution_result_fingerprint: String::new(),
            task_tree_revision: 0,
            project_revision: 0,
        });
        proj.workflow_state.recovery_state = Some(project::RecoveryState {
            error_kind: project::RecoveryErrorKind::TestFailure,
            phase: project::RecoveryPhase::Retesting,
            attempt: 1,
            max_attempts: 2,
            subtask_id: "subtask-1".to_string(),
            execution_id: "recovery-regression".to_string(),
            checkpoint_id,
            original_test_failure: "original failure".to_string(),
            active_issues: vec![original_issue.clone()],
            pending_execution_result: Some(project::ExecutionResult {
                success: true,
                output: "regressing repair".to_string(),
                file_changes: vec!["tracked.txt".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        });

        let rolled_back = crate::recovery::finish_retest(
            &mut proj,
            &session,
            "recovery-regression",
            project::TestResult {
                passed: false,
                issues: vec!["new regression".to_string()],
                automated_test_status: project::AutomatedTestStatus::Failed,
                ..Default::default()
            },
        )?;
        assert!(rolled_back);
        assert_eq!(
            std::fs::read_to_string(repo.path.join("tracked.txt"))
                .map_err(|error| error.to_string())?,
            "baseline\n"
        );
        let subtask = &proj.milestones[0].mid_stages[0].subtasks[0];
        assert_eq!(
            subtask
                .execution_result
                .as_ref()
                .map(|result| &result.output),
            Some(&original_execution.output)
        );
        assert!(subtask.test_result.is_none());
        assert!(subtask.acceptance_ledger.is_empty());
        assert!(subtask.human_verification.is_none());
        let recovery = proj
            .workflow_state
            .recovery_state
            .as_ref()
            .ok_or_else(|| "回归回滚恢复状态意外丢失".to_string())?;
        assert!(recovery.rollback_retest_pending);
        assert!(recovery.pending_execution_result.is_none());
        assert_eq!(recovery.active_issues, vec![original_issue]);

        let rolled_back_again = crate::recovery::finish_retest(
            &mut proj,
            &session,
            "recovery-regression",
            project::TestResult {
                passed: false,
                issues: vec!["original failure".to_string()],
                automated_test_status: project::AutomatedTestStatus::Failed,
                ..Default::default()
            },
        )?;
        assert!(!rolled_back_again);
        let subtask = &proj.milestones[0].mid_stages[0].subtasks[0];
        let execution_result = subtask.execution_result.as_ref().unwrap();
        assert!(execution_result.success);
        assert_eq!(execution_result.output, original_execution.output);
        assert_eq!(
            execution_result.file_changes,
            original_execution.file_changes
        );
        assert_eq!(
            subtask.test_result.as_ref().map(|result| &result.issues),
            Some(&vec!["original failure".to_string()])
        );
        Ok(())
    }

    #[test]
    fn evidence_retest_does_not_spend_repair_or_replan_attempts() -> Result<(), String> {
        let session = execution_session("recovering", "recovery-evidence", "abc123");
        let mut proj = execution_project(
            "recovery-evidence",
            Path::new(""),
            project::SubtaskStatus::Executing,
            Some(session.clone()),
        );
        let subtask = &mut proj.milestones[0].mid_stages[0].subtasks[0];
        subtask.acceptance_criteria = vec!["criterion".to_string()];
        subtask.execution_result = Some(project::ExecutionResult {
            success: true,
            file_changes: vec!["tracked.txt".to_string()],
            ..Default::default()
        });
        proj.workflow_state.recovery_state = Some(project::RecoveryState {
            error_kind: project::RecoveryErrorKind::EvidenceInsufficient,
            phase: project::RecoveryPhase::Retesting,
            attempt: 1,
            max_attempts: 2,
            subtask_id: "subtask-1".to_string(),
            execution_id: "recovery-evidence".to_string(),
            ..Default::default()
        });
        let insufficient = project::TestResult {
            automated_test_status: project::AutomatedTestStatus::Passed,
            criterion_reviews: vec![project::CriterionReviewResult {
                criterion_index: 1,
                criterion: "criterion".to_string(),
                conclusion: project::CriterionReviewConclusion::EvidenceInsufficient,
                ..Default::default()
            }],
            ..Default::default()
        };

        crate::recovery::finish_retest(
            &mut proj,
            &session,
            "recovery-evidence",
            insufficient.clone(),
        )?;
        let recovery = proj
            .workflow_state
            .recovery_state
            .as_ref()
            .ok_or_else(|| "补证恢复状态意外丢失".to_string())?;
        assert_eq!(recovery.phase, project::RecoveryPhase::Retesting);
        assert_eq!(
            recovery.error_kind,
            project::RecoveryErrorKind::EvidenceInsufficient
        );
        assert_eq!(recovery.attempt, 1);
        assert!(!recovery.replan_attempted);
        assert_eq!(recovery.pending_evidence_criteria, vec![1]);

        proj.workflow_state
            .recovery_state
            .as_mut()
            .ok_or_else(|| "补证恢复状态意外丢失".to_string())?
            .evidence_rebuild_attempts = 2;
        crate::recovery::finish_retest(&mut proj, &session, "recovery-evidence", insufficient)?;
        let recovery = proj
            .workflow_state
            .recovery_state
            .as_ref()
            .ok_or_else(|| "补证恢复状态意外丢失".to_string())?;
        assert_eq!(recovery.phase, project::RecoveryPhase::WaitingHuman);
        assert_eq!(recovery.attempt, 1);
        assert!(!recovery.replan_attempted);
        Ok(())
    }

    #[test]
    fn exhausted_regular_repair_replans_once_then_waits_for_human() -> Result<(), String> {
        let session = execution_session("recovering", "recovery-exhausted", "abc123");
        let mut proj = execution_project(
            "recovery-exhausted",
            Path::new(""),
            project::SubtaskStatus::Executing,
            Some(session.clone()),
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: "milestone-1".to_string(),
            run_status: project::AutopilotRunStatus::Running,
            last_action: String::new(),
            last_action_at: String::new(),
            error_message: String::new(),
            recovery_action: project::AutopilotRecoveryAction::RunAutomaticRecovery,
            ..Default::default()
        });
        proj.workflow_state.recovery_state = Some(project::RecoveryState {
            error_kind: project::RecoveryErrorKind::TestFailure,
            phase: project::RecoveryPhase::Retesting,
            attempt: 2,
            max_attempts: 2,
            subtask_id: "subtask-1".to_string(),
            execution_id: "recovery-exhausted".to_string(),
            baseline_commit: "abc123".to_string(),
            ..Default::default()
        });
        proj.milestones[0].mid_stages[0].subtasks[0].execution_result =
            Some(project::ExecutionResult {
                success: true,
                file_changes: vec!["tracked.txt".to_string()],
                ..Default::default()
            });
        let failed = project::TestResult {
            passed: false,
            issues: vec!["仍未满足验收标准".to_string()],
            automated_test_status: project::AutomatedTestStatus::Failed,
            ..Default::default()
        };

        crate::recovery::finish_retest(&mut proj, &session, "recovery-exhausted", failed.clone())?;
        assert_eq!(
            proj.workflow_state
                .recovery_state
                .as_ref()
                .map(|state| &state.phase),
            Some(&project::RecoveryPhase::Replanning)
        );

        let recovery = proj.workflow_state.recovery_state.as_mut().unwrap();
        recovery.phase = project::RecoveryPhase::Retesting;
        recovery.replan_attempted = true;
        recovery.replan_execution_attempted = true;
        proj.execution_session.as_mut().unwrap().status = "recovering".to_string();
        crate::recovery::finish_retest(&mut proj, &session, "recovery-exhausted", failed)?;
        assert_eq!(
            proj.workflow_state
                .recovery_state
                .as_ref()
                .map(|state| &state.phase),
            Some(&project::RecoveryPhase::WaitingHuman)
        );
        Ok(())
    }

    #[test]
    fn stale_retest_cannot_overwrite_current_recovery_session() -> Result<(), String> {
        let session = execution_session("recovering", "recovery-current", "abc123");
        let mut proj = execution_project(
            "recovery-stale",
            Path::new(""),
            project::SubtaskStatus::Executing,
            Some(session.clone()),
        );
        proj.workflow_state.recovery_state = Some(project::RecoveryState {
            error_kind: project::RecoveryErrorKind::TestFailure,
            phase: project::RecoveryPhase::Retesting,
            subtask_id: "subtask-1".to_string(),
            execution_id: "recovery-current".to_string(),
            ..Default::default()
        });
        let original = proj.clone();

        let result = crate::recovery::finish_retest(
            &mut proj,
            &session,
            "recovery-stale",
            project::TestResult {
                passed: true,
                ..Default::default()
            },
        );

        assert!(result.is_err());
        assert_eq!(
            serde_json::to_value(&proj).map_err(|error| error.to_string())?,
            serde_json::to_value(&original).map_err(|error| error.to_string())?
        );
        Ok(())
    }

    #[test]
    fn phase1_human_action_safety_audited_override_preserves_failed_test() {
        let mut proj = execution_project(
            "human-override",
            Path::new(""),
            project::SubtaskStatus::AwaitingConfirmation,
            Some(execution_session(
                "awaiting_confirmation",
                "execution-human",
                "abc123",
            )),
        );
        proj.workflow_state.data_revision = 1;
        let subtask = &mut proj.milestones[0].mid_stages[0].subtasks[0];
        subtask.execution_result = Some(project::ExecutionResult {
            success: true,
            output: String::new(),
            error_log: String::new(),
            file_changes: vec!["tracked.txt".to_string()],
            ..Default::default()
        });
        subtask.test_result = Some(project::TestResult {
            passed: false,
            automated_test_status: project::AutomatedTestStatus::Unavailable,
            ..Default::default()
        });
        let execution_result_fingerprint =
            crate::human_action_policy::execution_result_fingerprint(subtask).unwrap();
        subtask.human_verification = Some(project::HumanVerification {
            verification_kind: project::VerificationKind::HumanOverride,
            verification_reason: "manual smoke test".to_string(),
            verified_at: "2026-07-21T00:00:00Z".to_string(),
            original_test_failure: "runner unavailable".to_string(),
            resolution: project::HumanResolution::ConfirmActualPass,
            accepted_criteria: vec![],
            dependency_check: String::new(),
            action_source: "recovery".to_string(),
            execution_result_fingerprint,
            task_tree_revision: 0,
            project_revision: 1,
        });

        assert!(validate_subtask_quality_gate(&proj).is_ok());
        assert_eq!(
            proj.milestones[0].mid_stages[0].subtasks[0]
                .test_result
                .as_ref()
                .map(|test| test.passed),
            Some(false)
        );

        proj.milestones[0].mid_stages[0].subtasks[0]
            .human_verification
            .as_mut()
            .unwrap()
            .execution_result_fingerprint = "sha256:forged".to_string();
        assert!(validate_subtask_quality_gate(&proj)
            .unwrap_err()
            .contains("执行结果已变化"));
    }

    #[test]
    fn quality_gate_rejects_unknown_acceptance_evidence() {
        let mut proj = execution_project(
            "unknown-acceptance",
            Path::new(""),
            project::SubtaskStatus::AwaitingConfirmation,
            Some(execution_session(
                "awaiting_confirmation",
                "execution-unknown-acceptance",
                "abc123",
            )),
        );
        let subtask = &mut proj.milestones[0].mid_stages[0].subtasks[0];
        subtask.acceptance_criteria = vec!["criterion".to_string()];
        subtask.execution_result = Some(project::ExecutionResult {
            success: true,
            ..Default::default()
        });
        subtask.test_result = Some(project::TestResult {
            passed: true,
            ..Default::default()
        });
        subtask.acceptance_ledger = vec![project::AcceptanceLedgerItem {
            criterion_index: 1,
            criterion: "criterion".to_string(),
            status: project::AcceptanceStatus::Unknown,
            ..Default::default()
        }];

        assert!(validate_subtask_quality_gate(&proj)
            .unwrap_err()
            .contains("未证明项"));
        proj.milestones[0].mid_stages[0].subtasks[0].acceptance_ledger[0].status =
            project::AcceptanceStatus::Satisfied;
        assert!(validate_subtask_quality_gate(&proj).is_ok());
    }

    #[test]
    fn adaptive_execution_contract_skipped_last_task_completes_stage_and_milestone() {
        let mut proj = execution_project(
            "skip-terminal",
            Path::new(""),
            project::SubtaskStatus::Skipped,
            None,
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState::default());
        let (mid_completed, milestone_completed) =
            reconcile_terminal_stage(&mut proj, "milestone-1", "mid-1")
                .expect("terminal stage reconciliation");
        assert!(mid_completed);
        assert!(milestone_completed);
        assert_eq!(
            proj.milestones[0].mid_stages[0].status,
            project::MidStageStatus::Completed
        );
        assert_eq!(
            proj.milestones[0].status,
            project::MilestoneStatus::Completed
        );
        assert_eq!(
            proj.workflow_state.current_step,
            project::WorkflowStep::MilestoneReview
        );
    }

    #[test]
    fn adaptive_execution_contract_terminal_reconcile_propagates_profile_error_atomically() {
        let mut proj = execution_project(
            "terminal-profile-error",
            Path::new(""),
            project::SubtaskStatus::Passed,
            None,
        );
        proj.workload_profile = None;
        proj.workflow_state.data_revision = 7;

        let error = reconcile_terminal_stage(&mut proj, "milestone-1", "mid-1")
            .expect_err("missing profile must block Review convergence");
        assert!(error.contains("画像缺失"));
        assert_eq!(proj.workflow_state.data_revision, 7);
        assert_eq!(
            proj.workflow_state.current_step,
            project::WorkflowStep::Execution
        );
        assert_eq!(
            proj.milestones[0].mid_stages[0].status,
            project::MidStageStatus::InProgress
        );
        assert_eq!(
            proj.milestones[0].status,
            project::MilestoneStatus::InProgress
        );
    }

    #[test]
    fn adaptive_execution_contract_startup_closure_builds_complete_quick_review_boundary() {
        let mut proj = quick_execution_project(project::SubtaskStatus::Passed, None);
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
        proj.workflow_state.data_revision = 12;
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState::default());
        proj.milestones[0].status = project::MilestoneStatus::Completed;
        proj.milestones[0].review_status = Some("approved".to_string());
        proj.milestones[0].review_conclusion = Some("A".to_string());

        assert!(
            crate::commands::workflow::reconcile_workflow_closure_state(&mut proj)
                .expect("startup workflow closure")
        );
        assert_eq!(proj.workflow_state.data_revision, 13);
        assert_eq!(
            proj.workflow_state.current_step,
            project::WorkflowStep::MilestoneReview
        );
        assert_eq!(proj.workflow_state.review_node_id, "milestone-1");
        assert_eq!(
            proj.milestones[0].review_status.as_deref(),
            Some("pending_review")
        );
        assert!(proj.milestones[0].review_conclusion.is_none());
        assert_eq!(
            proj.workflow_state
                .autopilot_state
                .as_ref()
                .expect("startup autopilot boundary")
                .run_status,
            project::AutopilotRunStatus::WaitingMilestoneReview
        );
    }

    #[test]
    fn adaptive_execution_contract_startup_closure_profile_error_is_atomic() {
        let mut proj = quick_execution_project(project::SubtaskStatus::Skipped, None);
        proj.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
        proj.workflow_state.data_revision = 14;
        proj.workload_profile = None;
        proj.milestones[0].status = project::MilestoneStatus::Completed;
        proj.milestones[0].review_status = Some("approved".to_string());

        let error = crate::commands::workflow::reconcile_workflow_closure_state(&mut proj)
            .expect_err("startup must propagate missing profile");
        assert!(error.contains("画像缺失"));
        assert_eq!(proj.workflow_state.data_revision, 14);
        assert_eq!(
            proj.workflow_state.current_step,
            project::WorkflowStep::MilestoneSelection
        );
        assert!(proj.workflow_state.review_node_id.is_empty());
        assert_eq!(
            proj.milestones[0].review_status.as_deref(),
            Some("approved")
        );
    }

    #[test]
    fn adaptive_execution_contract_quick_terminal_task_completes_milestone() {
        let mut proj = project::Project::new("quick-terminal");
        proj.workload_profile = Some(crate::workload_policy::test_profile(
            project::WorkloadScale::Small,
        ));
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id.clear();
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        proj.milestones.push(project::Milestone {
            id: "milestone-1".to_string(),
            title: "Quick".to_string(),
            status: project::MilestoneStatus::InProgress,
            mode: project::StageMode::Quick,
            subtasks: vec![test_subtask(project::SubtaskStatus::Skipped)],
            ..Default::default()
        });

        let (target_completed, milestone_completed) =
            reconcile_terminal_stage(&mut proj, "milestone-1", "")
                .expect("terminal milestone reconciliation");
        assert!(target_completed);
        assert!(milestone_completed);
        assert!(proj.milestones[0].mid_stages.is_empty());
        assert_eq!(
            proj.workflow_state.current_step,
            project::WorkflowStep::MilestoneReview
        );
    }

    #[test]
    fn quick_refresh_recovery_resets_direct_task_and_uses_milestone_boundary() {
        let mut lost =
            quick_execution_project(project::SubtaskStatus::Executing, Some("executing"));
        assert_eq!(
            reconcile_execution_state(&lost, None),
            ExecutionReconciliation::SessionLost
        );
        assert!(apply_execution_reconciliation(
            &mut lost,
            &ExecutionReconciliation::SessionLost
        ));
        assert_eq!(
            lost.milestones[0].subtasks[0].status,
            project::SubtaskStatus::Pending
        );
        assert!(lost.milestones[0].mid_stages.is_empty());

        let mut invalid = quick_execution_project(project::SubtaskStatus::Pending, None);
        invalid.execution_session = Some(project::ExecutionSession::default());
        assert!(apply_execution_reconciliation(
            &mut invalid,
            &ExecutionReconciliation::SessionInvalid
        ));
        assert_eq!(
            invalid.workflow_state.current_step,
            project::WorkflowStep::MilestoneSelection
        );
        assert!(invalid.current_mid_stage_id.is_empty());
    }

    #[test]
    fn validation_retry_and_running_recovery_survive_startup_reconciliation() {
        let proj = execution_project(
            "recovering-session",
            Path::new(""),
            project::SubtaskStatus::Executing,
            Some(execution_session(
                "recovering",
                "recovery-current",
                "abc123",
            )),
        );
        let pipeline = pipeline_state("recovery-current", PipelineStatus::Running);
        assert!(matches!(
            reconcile_execution_state(&proj, Some(&pipeline)),
            ExecutionReconciliation::Executing
        ));

        let mut queued_retry = proj.clone();
        queued_retry.workflow_state.recovery_state = Some(project::RecoveryState {
            error_kind: project::RecoveryErrorKind::ReviewTransientFailure,
            phase: project::RecoveryPhase::Retesting,
            subtask_id: "subtask-1".to_string(),
            execution_id: "recovery-current".to_string(),
            validation_retry_count: 1,
            max_validation_retries: 3,
            next_validation_retry_at: Some("2099-01-01T00:00:00Z".to_string()),
            ..Default::default()
        });
        let reconciliation = reconcile_execution_state(&queued_retry, None);
        assert!(matches!(
            reconciliation,
            ExecutionReconciliation::AwaitingConfirmation
        ));
        assert!(!apply_execution_reconciliation(
            &mut queued_retry,
            &reconciliation
        ));
        assert_eq!(
            queued_retry
                .workflow_state
                .recovery_state
                .as_ref()
                .map(|recovery| recovery.error_kind.clone()),
            Some(project::RecoveryErrorKind::ReviewTransientFailure)
        );
    }

    #[test]
    fn validation_progress_rejects_stale_execution_and_updates_heartbeat() -> Result<(), String> {
        let project_name = unique_project_name("verification-progress");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let session = execution_session("executing", "verification-current", "abc123");
        let mut proj = execution_project(
            &project_name,
            Path::new(""),
            project::SubtaskStatus::Executing,
            Some(session),
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            run_status: project::AutopilotRunStatus::Running,
            ..Default::default()
        });
        proj.workflow_state.recovery_state = Some(project::RecoveryState {
            error_kind: project::RecoveryErrorKind::ReviewProtocolFailure,
            phase: project::RecoveryPhase::Retesting,
            validation_retry_count: 1,
            max_validation_retries: 2,
            ..Default::default()
        });
        crate::save_project(&proj)?;

        assert!(!persist_verification_progress(
            &project_name,
            "verification-stale",
            project::VerificationStage::ProtocolRepair,
        )?);
        assert!(persist_verification_progress(
            &project_name,
            "verification-current",
            project::VerificationStage::ProtocolRepair,
        )?);

        let persisted = crate::load_project(&project_name)?;
        let session = persisted.execution_session.as_ref().unwrap();
        assert_eq!(
            session.verification_stage,
            project::VerificationStage::ProtocolRepair
        );
        let autopilot = persisted.workflow_state.autopilot_state.as_ref().unwrap();
        assert!(!autopilot.heartbeat_at.is_empty());
        assert!(autopilot.last_action.contains("Schema"));
        assert!(autopilot.last_action.contains("1/2"));
        Ok(())
    }

    #[tokio::test]
    async fn restore_failure_keeps_session_and_evidence() -> Result<(), String> {
        let project_name = unique_project_name("restore-fail");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut session = execution_session(
            "execution_failed",
            "execution-bad-base",
            "not-a-real-commit-hash",
        );
        session.active = false;
        session.failure_message = "original failure".to_string();
        // 无真实 git 仓库：恢复必然失败
        let proj = execution_project(
            &project_name,
            Path::new("/tmp/metheus-nonexistent-git-repo-for-test"),
            project::SubtaskStatus::Pending,
            Some(session),
        );
        crate::save_project(&proj)?;

        let result = retry_current_subtask(project_name.clone()).await;
        assert!(result.is_err());
        let err = result.err().unwrap_or_default();
        assert!(err.contains("失败证据已保留"));

        let persisted = crate::load_project(&project_name)?;
        assert_eq!(
            persisted
                .execution_session
                .as_ref()
                .map(|s| s.status.as_str()),
            Some("execution_failed")
        );
        assert_eq!(
            persisted
                .execution_session
                .as_ref()
                .map(|s| s.failure_message.as_str()),
            Some("original failure")
        );
        assert_eq!(
            persisted
                .execution_session
                .as_ref()
                .map(|s| s.base_commit.as_str()),
            Some("not-a-real-commit-hash")
        );
        assert_eq!(
            persisted.milestones[0].mid_stages[0].subtasks[0].retry_count,
            0
        );
        Ok(())
    }

    #[tokio::test]
    async fn ed_stop_completed_pipeline_rejects_without_overwrite() -> Result<(), String> {
        let mut executing = execution_project(
            "ed-stop-done",
            Path::new(""),
            project::SubtaskStatus::AwaitingConfirmation,
            Some(execution_session(
                "awaiting_confirmation",
                "execution-done",
                "HEAD",
            )),
        );
        // 会话仍标记 executing 模拟竞态边界；流水线已完成
        if let Some(ref mut session) = executing.execution_session {
            session.status = "executing".to_string();
            session.active = true;
        }
        let mut done_pipeline = pipeline_state("execution-done", PipelineStatus::Paused);
        done_pipeline.awaiting_confirmation = true;
        let pipeline = Arc::new(Mutex::new(Some(done_pipeline)));

        let result = request_ed_stop_with_pipeline_state(pipeline, &mut executing).await;
        assert!(result.is_err());
        let err = result.err().unwrap_or_default();
        assert!(err.contains("任务已经完成"));
        assert!(executing.pause_context.is_none());
        assert!(executing.execution_history.is_empty());
        Ok(())
    }

    #[test]
    fn finalize_execution_failure_sets_recoverable_session() {
        let mut proj = execution_project(
            "finalize-fail",
            Path::new(""),
            project::SubtaskStatus::Executing,
            Some(execution_session("executing", "execution-x", "abc123")),
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: "milestone-1".to_string(),
            run_status: project::AutopilotRunStatus::Running,
            last_action: String::new(),
            last_action_at: String::new(),
            error_message: String::new(),
            recovery_action: project::AutopilotRecoveryAction::None,
            ..Default::default()
        });
        let mut pipeline = Some(pipeline_state("execution-x", PipelineStatus::Running));
        finalize_execution_failure(&mut proj, &mut pipeline, "subtask-1", "timeout", None, true);

        let session = proj.execution_session.as_ref().expect("session kept");
        assert_eq!(session.status, "execution_failed");
        assert!(!session.active);
        assert_eq!(session.base_commit, "abc123");
        assert!(session.failure_message.contains("timeout"));
        assert_eq!(
            session.parsed_status(),
            project::ExecutionSessionStatus::ExecutionFailed
        );
        assert_eq!(
            proj.milestones[0].mid_stages[0].subtasks[0].status,
            project::SubtaskStatus::Pending
        );
        assert_eq!(
            proj.workflow_state
                .autopilot_state
                .as_ref()
                .map(|ap| &ap.recovery_action),
            Some(&project::AutopilotRecoveryAction::RestoreExecutionBaseline)
        );
        // 首次失败 retry_count 仍为 0，但会话可定位恢复
        assert_eq!(proj.milestones[0].mid_stages[0].subtasks[0].retry_count, 0);
    }

    #[test]
    fn execution_failure_updates_only_nested_leaf() {
        let mut proj = execution_project(
            "nested-failure",
            Path::new("/tmp"),
            project::SubtaskStatus::Pending,
            Some(execution_session("executing", "nested-execution", "base")),
        );
        let mut leaf = test_subtask(project::SubtaskStatus::Executing);
        leaf.id = "nested-leaf".to_string();
        let parent = &mut proj.milestones[0].mid_stages[0].subtasks[0];
        parent.id = "parent".to_string();
        parent.child_tasks = vec![leaf];
        let session = proj.execution_session.as_mut().unwrap();
        session.subtask_id = "nested-leaf".to_string();
        session.task_path = vec!["parent".to_string(), "nested-leaf".to_string()];
        session.parent_task_id = "parent".to_string();
        session.top_level_task_id = "parent".to_string();
        session.node_depth = 1;

        let mut pipeline = None;
        finalize_execution_failure(
            &mut proj,
            &mut pipeline,
            "nested-leaf",
            "timeout",
            None,
            true,
        );

        let parent = &proj.milestones[0].mid_stages[0].subtasks[0];
        assert_eq!(parent.status, project::SubtaskStatus::Pending);
        assert_eq!(
            parent.child_tasks[0].status,
            project::SubtaskStatus::Pending
        );
        assert_eq!(
            proj.execution_session.as_ref().unwrap().subtask_id,
            "nested-leaf"
        );
    }

    #[tokio::test]
    async fn adaptive_execution_contract_engine_block_restores_baseline() -> Result<(), String> {
        let repo = TempGitRepo::new("initial-engine-block")?;
        let baseline = repo.head()?;
        std::fs::write(repo.path.join("tracked.txt"), "partial provider change\n")
            .map_err(|error| error.to_string())?;
        std::fs::write(repo.path.join("untracked.txt"), "partial output\n")
            .map_err(|error| error.to_string())?;

        let project_name = unique_project_name("initial-engine-block");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = execution_project(
            &project_name,
            &repo.path,
            project::SubtaskStatus::Executing,
            Some(execution_session("executing", "execution-quota", &baseline)),
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: "milestone-1".to_string(),
            run_status: project::AutopilotRunStatus::Running,
            ..Default::default()
        });
        crate::save_project(&proj)?;

        let pipeline = Arc::new(Mutex::new(Some(pipeline_state(
            "execution-quota",
            PipelineStatus::Running,
        ))));
        let failure = BackgroundExecutionFailure::engine(
            project::RecoveryErrorKind::ExecutionError,
            project::EngineFailureKind::QuotaExceeded,
            "API Error: 402 Insufficient Balance".to_string(),
            Some(project::ExecutionResult {
                success: false,
                stdout: "API Error: 402 Insufficient Balance".to_string(),
                engine_failure_kind: Some(project::EngineFailureKind::QuotaExceeded),
                ..Default::default()
            }),
        );
        finalize_background_execution_failure(
            &project_name,
            "milestone-1",
            "mid-1",
            "subtask-1",
            "测试小阶段",
            0,
            1,
            "execution-quota",
            &failure,
            pipeline,
            project::OperationSource::Autopilot,
        )
        .await?;

        assert_eq!(
            std::fs::read_to_string(repo.path.join("tracked.txt")).unwrap(),
            "baseline\n"
        );
        assert!(!repo.path.join("untracked.txt").exists());
        let persisted = crate::load_project(&project_name)?;
        let recovery = persisted.workflow_state.recovery_state.as_ref().unwrap();
        assert_eq!(recovery.phase, project::RecoveryPhase::WaitingEngine);
        assert_eq!(recovery.attempt, 0);
        assert_eq!(
            recovery.engine_failure_kind,
            Some(project::EngineFailureKind::QuotaExceeded)
        );
        assert!(persisted.milestones[0].mid_stages[0].subtasks[0]
            .execution_result
            .as_ref()
            .is_some_and(|result| result.stdout.contains("402 Insufficient Balance")));
        assert!(persisted
            .execution_session
            .as_ref()
            .is_some_and(|session| session.failure_message.contains("已恢复任务基线")));
        assert_eq!(
            recovery.baseline_status,
            project::RecoveryBaselineStatus::Restored
        );
        assert!(recovery.baseline_stash_created);
        assert!(!recovery.baseline_target_summary.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn baseline_restore_outcome_is_persisted() -> Result<(), String> {
        // Old RecoveryState JSON without baseline fields deserializes as Unknown.
        let legacy = serde_json::json!({
            "error_kind": "ExecutionError",
            "phase": "Diagnosing",
            "attempt": 0,
            "max_attempts": 2,
            "error_signature": "sig",
            "subtask_id": "subtask-1",
            "execution_id": "exec-1",
            "started_at": "2026-08-11T00:00:00Z",
            "updated_at": "2026-08-11T00:00:00Z"
        });
        let legacy_state: project::RecoveryState =
            serde_json::from_value(legacy).map_err(|e| e.to_string())?;
        assert_eq!(
            legacy_state.baseline_status,
            project::RecoveryBaselineStatus::Unknown
        );
        assert!(legacy_state.baseline_target_summary.is_empty());
        assert!(!legacy_state.baseline_stash_created);

        // Clean tree → Restored, stash_created=false
        let clean_repo = TempGitRepo::new("baseline-clean")?;
        let clean_head = clean_repo.head()?;
        let clean_outcome = restore_git_execution_baseline(&clean_repo.path_string(), &clean_head)
            .map_err(|o| o.error_message())?;
        assert_eq!(
            clean_outcome.status,
            project::RecoveryBaselineStatus::Restored
        );
        assert!(!clean_outcome.stash_created);
        assert_eq!(
            clean_outcome.target_summary,
            summarize_baseline_target(&clean_head)
        );

        // Dirty tree success → Restored, stash_created=true
        let dirty_repo = TempGitRepo::new("baseline-dirty")?;
        let dirty_head = dirty_repo.head()?;
        std::fs::write(dirty_repo.path.join("tracked.txt"), "dirty work\n")
            .map_err(|e| e.to_string())?;
        std::fs::write(dirty_repo.path.join("untracked-new.txt"), "new\n")
            .map_err(|e| e.to_string())?;
        let dirty_outcome = restore_git_execution_baseline(&dirty_repo.path_string(), &dirty_head)
            .map_err(|o| o.error_message())?;
        assert_eq!(
            dirty_outcome.status,
            project::RecoveryBaselineStatus::Restored
        );
        assert!(dirty_outcome.stash_created);
        assert_eq!(
            std::fs::read_to_string(dirty_repo.path.join("tracked.txt")).unwrap(),
            "baseline\n"
        );
        assert!(!dirty_repo.path.join("untracked-new.txt").exists());

        // Engine-blocked path persists structured facts on RecoveryState
        let project_name = unique_project_name("baseline-persisted");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let persist_repo = TempGitRepo::new("baseline-persisted")?;
        let persist_head = persist_repo.head()?;
        std::fs::write(persist_repo.path.join("tracked.txt"), "partial\n")
            .map_err(|e| e.to_string())?;
        let mut proj = execution_project(
            &project_name,
            &persist_repo.path,
            project::SubtaskStatus::Executing,
            Some(execution_session(
                "executing",
                "execution-baseline-fact",
                &persist_head,
            )),
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: "milestone-1".to_string(),
            run_status: project::AutopilotRunStatus::Running,
            ..Default::default()
        });
        crate::save_project(&proj)?;
        let pipeline = Arc::new(Mutex::new(Some(pipeline_state(
            "execution-baseline-fact",
            PipelineStatus::Running,
        ))));
        let failure = BackgroundExecutionFailure::engine(
            project::RecoveryErrorKind::ExecutionError,
            project::EngineFailureKind::OutputTruncated,
            "truncated".to_string(),
            Some(project::ExecutionResult {
                success: false,
                engine_failure_kind: Some(project::EngineFailureKind::OutputTruncated),
                ..Default::default()
            }),
        );
        finalize_background_execution_failure(
            &project_name,
            "milestone-1",
            "mid-1",
            "subtask-1",
            "测试小阶段",
            0,
            1,
            "execution-baseline-fact",
            &failure,
            pipeline,
            project::OperationSource::Autopilot,
        )
        .await?;
        let persisted = crate::load_project(&project_name)?;
        let recovery = persisted
            .workflow_state
            .recovery_state
            .as_ref()
            .expect("recovery");
        assert_eq!(
            recovery.baseline_status,
            project::RecoveryBaselineStatus::Restored
        );
        assert!(recovery.baseline_stash_created);
        assert_eq!(
            recovery.baseline_target_summary,
            summarize_baseline_target(&persist_head)
        );
        assert_eq!(
            recovery.engine_failure_kind,
            Some(project::EngineFailureKind::OutputTruncated)
        );

        // Failed restore → RestoreFailed facts (invalid target ref)
        let fail_repo = TempGitRepo::new("baseline-fail")?;
        let fail_outcome = restore_git_execution_baseline(
            &fail_repo.path_string(),
            "definitely-not-a-valid-ref-zzzz",
        );
        assert!(fail_outcome.is_err());
        let failed = fail_outcome.err().unwrap();
        assert_eq!(
            failed.status,
            project::RecoveryBaselineStatus::RestoreFailed
        );
        assert!(failed.error.is_some());
        assert!(!failed.target_summary.is_empty());
        // Round-trip apply to recovery state
        let mut applied = project::RecoveryState::default();
        apply_baseline_restore_outcome(&mut applied, &failed);
        assert_eq!(
            applied.baseline_status,
            project::RecoveryBaselineStatus::RestoreFailed
        );
        assert_eq!(applied.baseline_target_summary, failed.target_summary);
        Ok(())
    }

    #[tokio::test]
    async fn output_truncation_uses_single_recovery_entry() -> Result<(), String> {
        let repo = TempGitRepo::new("output-truncation-single-entry")?;
        let baseline = repo.head()?;
        std::fs::write(repo.path.join("tracked.txt"), "partial truncated output\n")
            .map_err(|error| error.to_string())?;

        let project_name = unique_project_name("output-truncation-single-entry");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = execution_project(
            &project_name,
            &repo.path,
            project::SubtaskStatus::Executing,
            Some(execution_session(
                "executing",
                "execution-truncated",
                &baseline,
            )),
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: "milestone-1".to_string(),
            run_status: project::AutopilotRunStatus::Running,
            transient_retry_count: 0,
            ..Default::default()
        });
        crate::save_project(&proj)?;

        let pipeline = Arc::new(Mutex::new(Some(pipeline_state(
            "execution-truncated",
            PipelineStatus::Running,
        ))));
        let failure = BackgroundExecutionFailure::engine(
            project::RecoveryErrorKind::ExecutionError,
            project::EngineFailureKind::OutputTruncated,
            "model output truncated after continuation".to_string(),
            Some(project::ExecutionResult {
                success: false,
                stdout: "model output truncated after continuation".to_string(),
                engine_failure_kind: Some(project::EngineFailureKind::OutputTruncated),
                ..Default::default()
            }),
        );
        finalize_background_execution_failure(
            &project_name,
            "milestone-1",
            "mid-1",
            "subtask-1",
            "测试小阶段",
            0,
            1,
            "execution-truncated",
            &failure,
            pipeline,
            project::OperationSource::Autopilot,
        )
        .await?;

        let updated = crate::load_project(&project_name)?;
        let recovery = updated
            .workflow_state
            .recovery_state
            .as_ref()
            .expect("recovery started");
        assert_eq!(recovery.error_kind, project::RecoveryErrorKind::PlanFailure);
        assert_eq!(recovery.phase, project::RecoveryPhase::Replanning);
        assert_eq!(
            recovery.engine_failure_kind,
            Some(project::EngineFailureKind::OutputTruncated)
        );

        let autopilot = updated
            .workflow_state
            .autopilot_state
            .as_ref()
            .expect("autopilot present");
        assert_eq!(autopilot.run_status, project::AutopilotRunStatus::Running);
        assert_eq!(
            autopilot.recovery_action,
            project::AutopilotRecoveryAction::RunAutomaticRecovery
        );
        assert_ne!(
            autopilot.recovery_action,
            project::AutopilotRecoveryAction::RegenerateExecutionPlan
        );
        assert_eq!(
            autopilot.last_failure_kind,
            project::AutopilotFailureKind::Permanent
        );
        assert_eq!(autopilot.transient_retry_count, 0);
        assert!(autopilot.next_retry_at.is_none());
        assert!(
            autopilot.last_action.contains("受限重规划") || autopilot.last_action.contains("诊断")
        );

        let decision = crate::autopilot_policy::decide_next_step(
            &updated,
            &project_name,
            &crate::autopilot_policy::AutopilotPolicyFacts {
                precondition_block: None,
                quality_gate: crate::autopilot_policy::QualityGateFact::NotApplicable,
                needs_calibration: false,
            },
        );
        assert_eq!(decision.next.command, "run_error_recovery");
        assert_ne!(decision.next.command, "regenerate_execution_plan");
        Ok(())
    }

    #[tokio::test]
    async fn background_execution_failure_starts_automatic_recovery() -> Result<(), String> {
        let project_name = unique_project_name("background-auto-recovery");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = execution_project(
            &project_name,
            Path::new(""),
            project::SubtaskStatus::Executing,
            Some(execution_session("executing", "execution-auto", "abc123")),
        );
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            target_milestone_id: "milestone-1".to_string(),
            run_status: project::AutopilotRunStatus::Running,
            last_action: String::new(),
            last_action_at: String::new(),
            error_message: String::new(),
            recovery_action: project::AutopilotRecoveryAction::None,
            ..Default::default()
        });
        crate::save_project(&proj)?;
        let pipeline = Arc::new(Mutex::new(Some(pipeline_state(
            "execution-auto",
            PipelineStatus::Running,
        ))));
        let failure = BackgroundExecutionFailure::new(
            project::RecoveryErrorKind::ExecutionError,
            "process lost".to_string(),
        );

        finalize_background_execution_failure(
            &project_name,
            "milestone-1",
            "mid-1",
            "subtask-1",
            "测试小阶段",
            0,
            1,
            "execution-auto",
            &failure,
            pipeline,
            project::OperationSource::Autopilot,
        )
        .await?;

        let updated = crate::load_project(&project_name)?;
        let recovery = updated.workflow_state.recovery_state.as_ref().unwrap();
        assert_eq!(
            recovery.error_kind,
            project::RecoveryErrorKind::ExecutionError
        );
        assert_eq!(recovery.phase, project::RecoveryPhase::Diagnosing);
        assert_eq!(recovery.baseline_commit, "abc123");
        let autopilot = updated.workflow_state.autopilot_state.as_ref().unwrap();
        assert_eq!(autopilot.run_status, project::AutopilotRunStatus::Running);
        assert_eq!(
            autopilot.recovery_action,
            project::AutopilotRecoveryAction::RunAutomaticRecovery
        );
        Ok(())
    }

    #[test]
    fn failed_session_survives_reconcile_without_clearing() {
        let mut session = execution_session("execution_failed", "execution-keep", "HEAD");
        session.active = false;
        session.failure_message = "kept".to_string();
        let proj = execution_project(
            "keep-failed",
            Path::new(""),
            project::SubtaskStatus::Pending,
            Some(session),
        );
        let result = reconcile_execution_state(&proj, None);
        // keep 路径：不得清理失败会话
        assert!(matches!(
            result,
            ExecutionReconciliation::AwaitingConfirmation
        ));
        let mut copy = proj.clone();
        assert!(!apply_execution_reconciliation(&mut copy, &result));
        assert_eq!(
            copy.execution_session
                .as_ref()
                .map(|s| s.failure_message.as_str()),
            Some("kept")
        );
    }

    /// 模拟“取锁后再 load”的正确对账：后台已写入 awaiting_confirmation 时，
    /// 不得用启动前缓存的 executing 旧快照判 SessionLost 并覆盖。
    #[tokio::test]
    async fn reconcile_under_lock_after_completion_keeps_awaiting_results() -> Result<(), String> {
        let project_name = unique_project_name("reconcile-race");
        let _guard = ProjectDataGuard::new(&project_name)?;

        // 磁盘初始为 executing（模拟启动对账若过早 load 会拿到的旧快照）
        let executing = execution_project(
            &project_name,
            Path::new(""),
            project::SubtaskStatus::Executing,
            Some(execution_session("executing", "execution-race", "HEAD")),
        );
        crate::save_project(&executing)?;

        let pipeline = Arc::new(Mutex::new(Some(pipeline_state(
            "execution-race",
            PipelineStatus::Running,
        ))));

        // 后台完成：持锁写 awaiting_confirmation + 测试结果，流水线改 Paused
        {
            let mut guard = pipeline.lock().await;
            let mut done = crate::load_project(&project_name)?;
            done.milestones[0].mid_stages[0].subtasks[0].status =
                project::SubtaskStatus::AwaitingConfirmation;
            done.milestones[0].mid_stages[0].subtasks[0].execution_result =
                Some(project::ExecutionResult {
                    success: true,
                    output: "ok".to_string(),
                    error_log: String::new(),
                    file_changes: vec!["tracked.txt".to_string()],
                    ..Default::default()
                });
            done.milestones[0].mid_stages[0].subtasks[0].test_result = Some(project::TestResult {
                passed: true,
                issues: vec![],
                suggestion: String::new(),
                warnings: vec![],
                ..Default::default()
            });
            if let Some(ref mut session) = done.execution_session {
                session.status = "awaiting_confirmation".to_string();
                session.state_entered_at = chrono::Utc::now().to_rfc3339();
            }
            crate::save_project(&done)?;
            if let Some(ref mut ps) = *guard {
                ps.status = PipelineStatus::Paused;
                ps.awaiting_confirmation = true;
            }
        }

        // 错误路径反例：若仍用启动前的旧 executing 快照 + 完成后的 Paused 内存态，会误判 SessionLost
        let stale = executing.clone();
        let paused = pipeline_state("execution-race", PipelineStatus::Paused);
        assert!(matches!(
            reconcile_execution_state(&stale, Some(&paused)),
            ExecutionReconciliation::SessionLost
        ));

        // 正确路径：持锁后重新 load，再对账 → 保留待确认与执行证据
        {
            let guard = pipeline.lock().await;
            let mut fresh = crate::load_project(&project_name)?;
            let modified = reconcile_loaded_project_under_pipeline_lock(&mut fresh, guard.as_ref());
            assert!(!modified, "待确认事实不得被对账改写");
            assert_eq!(
                fresh.execution_session.as_ref().map(|s| s.status.as_str()),
                Some("awaiting_confirmation")
            );
            assert_eq!(
                fresh.milestones[0].mid_stages[0].subtasks[0].status,
                project::SubtaskStatus::AwaitingConfirmation
            );
            assert!(fresh.milestones[0].mid_stages[0].subtasks[0]
                .execution_result
                .as_ref()
                .is_some_and(|r| r.success));
            assert!(fresh.milestones[0].mid_stages[0].subtasks[0]
                .test_result
                .as_ref()
                .is_some_and(|r| r.passed));
            if modified {
                crate::save_project(&fresh)?;
            }
        }

        let final_proj = crate::load_project(&project_name)?;
        assert_eq!(
            final_proj
                .execution_session
                .as_ref()
                .map(|s| s.status.as_str()),
            Some("awaiting_confirmation")
        );
        Ok(())
    }

    #[test]
    fn phase1_runtime_contract_git_confirmation_claim_reconciles_without_reexecution() {
        let proj = execution_project(
            "claim-crash",
            Path::new(""),
            project::SubtaskStatus::AwaitingConfirmation,
            Some(execution_session("confirming", "execution-claim", "HEAD")),
        );
        assert!(matches!(
            reconcile_execution_state(&proj, None),
            ExecutionReconciliation::AwaitingConfirmation
        ));
    }

    #[test]
    fn git_confirmation_interrupted_transaction_becomes_retryable_block() {
        let mut session = execution_session("confirming", "execution-claim", "HEAD");
        session.confirmation_transaction_id = "transaction-interrupted".to_string();
        session.confirmation_phase = project::ConfirmationPhase::CommitCreated;
        session.confirmation_commit = "commit-interrupted".to_string();
        let mut proj = execution_project(
            "claim-crash",
            Path::new(""),
            project::SubtaskStatus::AwaitingConfirmation,
            Some(session),
        );
        proj.workflow_state.autopilot_state = Some(project::AutopilotState::default());

        assert!(reconcile_loaded_project_under_pipeline_lock(
            &mut proj, None
        ));
        let session = proj.execution_session.as_ref().unwrap();
        assert_eq!(session.status, "confirmation_blocked");
        assert_eq!(
            session.confirmation_failure_kind,
            Some(project::GitConfirmationFailureKind::ProjectFinalizationFailed)
        );
        assert_eq!(
            proj.workflow_state
                .autopilot_state
                .as_ref()
                .map(|autopilot| &autopilot.recovery_action),
            Some(&project::AutopilotRecoveryAction::RetryGitConfirmation)
        );
    }

    #[test]
    fn runtime_fault_stale_lock_reconciliation_preserves_git_transaction_facts() {
        let mut session = execution_session("confirming", "execution-claim", "HEAD");
        session.confirmation_transaction_id = "transaction-interrupted".to_string();
        session.confirmation_phase = project::ConfirmationPhase::CommitCreated;
        session.confirmation_commit = "commit-interrupted".to_string();
        let mut proj = execution_project(
            "claim-crash-with-lock",
            Path::new(""),
            project::SubtaskStatus::AwaitingConfirmation,
            Some(session),
        );
        proj.workflow_state.autopilot_state = Some(project::AutopilotState::default());
        let now = chrono::Utc::now();
        let lease = crate::task_control::ControlActionLease {
            action_id: "git-confirm-action".to_string(),
            owner_process_start_id: "old-process".to_string(),
            action_kind: crate::control_action::ControlActionKind::GitConfirm
                .as_str()
                .to_string(),
            task_id: "subtask-1".to_string(),
            started_at: (now - chrono::Duration::seconds(40)).to_rfc3339(),
            heartbeat_at: (now - chrono::Duration::seconds(20)).to_rfc3339(),
            expected_max_duration_secs: 900,
        };
        proj.task_control.active_action_id = lease.action_id.clone();
        proj.task_control.active_action_kind = lease.action_kind.clone();
        proj.task_control.active_action_task_id = lease.task_id.clone();
        proj.task_control.active_action_lease = Some(lease);

        assert!(reconcile_loaded_project_under_pipeline_lock(
            &mut proj, None
        ));
        let reconciled = proj.execution_session.as_ref().unwrap();
        assert_eq!(reconciled.status, "confirmation_blocked");
        assert_eq!(
            reconciled.confirmation_phase,
            project::ConfirmationPhase::CommitCreated
        );
        assert_eq!(reconciled.confirmation_commit, "commit-interrupted");
        assert_eq!(
            reconciled.confirmation_transaction_id,
            "transaction-interrupted"
        );
        assert!(proj.task_control.active_action_lease.is_none());
        assert_eq!(
            proj.execution_history.last().unwrap().event_type,
            project::ExecutionEventType::StaleControlLockCleared
        );
        assert_eq!(
            proj.execution_history
                .iter()
                .filter(|entry| {
                    entry.event_type == project::ExecutionEventType::GitConfirmationCommitCreated
                })
                .count(),
            0,
            "锁对账不得创建或重复记录 Git 提交"
        );
    }

    #[test]
    fn git_confirmation_claim_is_exclusive_and_reuses_transaction() -> Result<(), String> {
        let project_name = unique_project_name("claim-excl");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut proj = execution_project(
            &project_name,
            Path::new(""),
            project::SubtaskStatus::AwaitingConfirmation,
            Some(execution_session(
                "awaiting_confirmation",
                "execution-claim",
                "HEAD",
            )),
        );
        crate::save_project(&proj)?;

        claim_awaiting_confirmation_under_lock(&mut proj, "confirming")?;
        assert_eq!(
            proj.execution_session.as_ref().map(|s| s.status.as_str()),
            Some("confirming")
        );
        let transaction_id = proj
            .execution_session
            .as_ref()
            .map(|session| session.confirmation_transaction_id.clone())
            .ok_or("缺少确认会话".to_string())?;
        let candidate_tag = proj
            .execution_session
            .as_ref()
            .map(|session| session.confirmation_candidate_tag.clone())
            .ok_or("缺少确认会话".to_string())?;
        assert!(!transaction_id.is_empty());
        assert!(candidate_tag.contains(&transaction_id));

        let mut second = crate::load_project(&project_name)?;
        let err = claim_awaiting_confirmation_under_lock(&mut second, "confirming")
            .err()
            .ok_or("第二次认领应失败".to_string())?;
        assert!(err.contains("正在进行中") || err.contains("重复"));

        if let Some(session) = second.execution_session.as_mut() {
            session.status = "confirmation_blocked".to_string();
            session.confirmation_failure_kind =
                Some(project::GitConfirmationFailureKind::TagFailed);
        }
        crate::save_project(&second)?;
        let mut retry = crate::load_project(&project_name)?;
        claim_awaiting_confirmation_under_lock(&mut retry, "confirming")?;
        let retry_session = retry
            .execution_session
            .as_ref()
            .ok_or("缺少重试确认会话".to_string())?;
        assert_eq!(retry_session.confirmation_transaction_id, transaction_id);
        assert_eq!(retry_session.confirmation_candidate_tag, candidate_tag);
        Ok(())
    }

    #[test]
    fn git_confirmation_normalizes_persisted_generic_conflicts_by_transaction_facts() {
        let mut legacy = execution_project(
            "legacy-generic-conflict",
            Path::new(""),
            project::SubtaskStatus::AwaitingConfirmation,
            Some(execution_session(
                "confirmation_blocked",
                "execution-legacy-generic",
                "HEAD",
            )),
        );
        legacy.workflow_state.autopilot_state = Some(project::AutopilotState::default());
        legacy
            .execution_session
            .as_mut()
            .unwrap()
            .confirmation_failure_kind =
            Some(project::GitConfirmationFailureKind::TagIdentityConflict);

        assert!(normalize_legacy_confirmation_conflict_kind(&mut legacy));
        assert_eq!(
            legacy
                .execution_session
                .as_ref()
                .and_then(|session| session.confirmation_failure_kind.as_ref()),
            Some(&project::GitConfirmationFailureKind::LegacyV1TagConflict)
        );
        assert_eq!(
            legacy
                .workflow_state
                .autopilot_state
                .as_ref()
                .map(|autopilot| &autopilot.recovery_action),
            Some(&project::AutopilotRecoveryAction::RetryGitConfirmation)
        );

        let mut v2 = legacy.clone();
        let v2_session = v2.execution_session.as_mut().unwrap();
        v2_session.confirmation_failure_kind =
            Some(project::GitConfirmationFailureKind::TagIdentityConflict);
        v2_session.confirmation_transaction_id = "persisted-transaction".to_string();
        v2_session.confirmation_phase = project::ConfirmationPhase::Preparing;
        v2.workflow_state
            .autopilot_state
            .as_mut()
            .unwrap()
            .recovery_action = project::AutopilotRecoveryAction::RetryGitConfirmation;

        assert!(normalize_legacy_confirmation_conflict_kind(&mut v2));
        assert_eq!(
            v2.execution_session
                .as_ref()
                .and_then(|session| session.confirmation_failure_kind.as_ref()),
            Some(&project::GitConfirmationFailureKind::V2TagIntegrityConflict)
        );
        assert_eq!(
            v2.workflow_state
                .autopilot_state
                .as_ref()
                .map(|autopilot| &autopilot.recovery_action),
            Some(&project::AutopilotRecoveryAction::WaitHumanDecision)
        );
    }

    #[tokio::test]
    async fn git_confirmation_v2_integrity_conflict_stops_for_human_without_committing(
    ) -> Result<(), String> {
        let repo = TempGitRepo::new("v2-integrity-conflict")?;
        let project_name = unique_project_name("v2-integrity-conflict");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let transaction_id = "transaction-owned-elsewhere";
        let candidate_tag =
            crate::git_ops::subtask_v2_tag("milestone-1", "mid-1", "subtask-1", transaction_id);
        let unrelated_commit = repo.head()?;
        repo.git(&["tag", &candidate_tag, &unrelated_commit])?;
        std::fs::write(repo.path.join("tracked.txt"), "approved task change\n")
            .map_err(|error| format!("写入待确认变更失败：{}", error))?;

        let mut session = execution_session(
            "awaiting_confirmation",
            "v2-integrity-execution",
            &unrelated_commit,
        );
        session.confirmation_transaction_id = transaction_id.to_string();
        session.confirmation_phase = project::ConfirmationPhase::Preparing;
        session.confirmation_candidate_tag = candidate_tag.clone();
        let mut proj = execution_project(
            &project_name,
            &repo.path,
            project::SubtaskStatus::AwaitingConfirmation,
            Some(session),
        );
        let subtask = &mut proj.milestones[0].mid_stages[0].subtasks[0];
        subtask.execution_result = Some(project::ExecutionResult {
            success: true,
            output: "approved execution".to_string(),
            ..Default::default()
        });
        subtask.test_result = Some(project::TestResult {
            passed: true,
            review_passed: true,
            ..Default::default()
        });
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            run_status: project::AutopilotRunStatus::Running,
            ..Default::default()
        });
        crate::save_project(&proj)?;
        let pipeline = Arc::new(Mutex::new(Some(pipeline_state(
            "v2-integrity-execution",
            PipelineStatus::Paused,
        ))));
        let commit_count = repo.git(&["rev-list", "--count", "HEAD"])?;

        let error = confirm_subtask_result_with_pipeline(&pipeline, project_name.clone())
            .await
            .expect_err("V2 标签完整性冲突必须阻断确认");
        assert!(!error.contains("代码与质量结果已保留"));

        let blocked = crate::load_project(&project_name)?;
        let blocked_session = blocked
            .execution_session
            .as_ref()
            .ok_or("V2 冲突后缺少确认会话".to_string())?;
        assert_eq!(blocked_session.status, "confirmation_blocked");
        assert_eq!(
            blocked_session.confirmation_failure_kind,
            Some(project::GitConfirmationFailureKind::V2TagIntegrityConflict)
        );
        assert_eq!(
            blocked
                .workflow_state
                .autopilot_state
                .as_ref()
                .map(|autopilot| &autopilot.recovery_action),
            Some(&project::AutopilotRecoveryAction::WaitHumanDecision)
        );
        assert_eq!(
            blocked.milestones[0].mid_stages[0].subtasks[0].retry_count,
            0
        );
        assert!(blocked.milestones[0].mid_stages[0].subtasks[0]
            .execution_result
            .as_ref()
            .is_some_and(|result| result.success));
        assert!(blocked.milestones[0].mid_stages[0].subtasks[0]
            .test_result
            .as_ref()
            .is_some_and(|result| result.passed));
        assert_eq!(
            crate::git_ops::tag_target(&repo.path_string(), &candidate_tag)?,
            Some(unrelated_commit)
        );
        assert_eq!(repo.git(&["rev-list", "--count", "HEAD"])?, commit_count);
        assert!(repo.git(&["status", "--short"])?.contains("tracked.txt"));

        let retry_error = confirm_subtask_result_with_pipeline(&pipeline, project_name.clone())
            .await
            .expect_err("不可重试的 V2 完整性冲突必须拒绝再次认领");
        assert!(retry_error.contains("禁止机械重试"));
        assert_eq!(repo.git(&["rev-list", "--count", "HEAD"])?, commit_count);
        Ok(())
    }

    #[tokio::test]
    async fn git_confirmation_migrates_legacy_v1_collision_and_completes_v2_confirmation(
    ) -> Result<(), String> {
        let repo = TempGitRepo::new("legacy-tag-collision")?;
        let legacy_target = repo.head()?;
        repo.git(&["tag", "metheus/auto/v0.1.1/task-1", &legacy_target])?;
        repo.git(&["commit", "--allow-empty", "-m", "other milestone task"])?;
        std::fs::write(repo.path.join("tracked.txt"), "approved task change\n")
            .map_err(|error| format!("写入待确认变更失败：{}", error))?;

        let project_name = unique_project_name("legacy-confirmation");
        let _guard = ProjectDataGuard::new(&project_name)?;
        let mut session = execution_session("execution_failed", "legacy-execution", &repo.head()?);
        session.active = false;
        session.failure_message = "legacy opaque failure".to_string();
        let mut proj = execution_project(
            &project_name,
            &repo.path,
            project::SubtaskStatus::AwaitingConfirmation,
            Some(session),
        );
        let subtask = &mut proj.milestones[0].mid_stages[0].subtasks[0];
        subtask.execution_result = Some(project::ExecutionResult {
            success: true,
            output: "approved execution result".to_string(),
            ..Default::default()
        });
        subtask.test_result = Some(project::TestResult {
            passed: true,
            review_passed: true,
            suggestion: "approved quality result".to_string(),
            ..Default::default()
        });
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState {
            active: true,
            recovery_action: project::AutopilotRecoveryAction::RestoreExecutionBaseline,
            ..Default::default()
        });

        assert!(reconcile_loaded_project_under_pipeline_lock(
            &mut proj, None
        ));
        let migrated = proj
            .execution_session
            .as_ref()
            .ok_or("迁移后缺少确认会话".to_string())?;
        assert_eq!(migrated.status, "confirmation_blocked");
        assert_eq!(
            migrated.confirmation_failure_kind,
            Some(project::GitConfirmationFailureKind::LegacyV1TagConflict)
        );
        assert!(migrated.confirmation_transaction_id.is_empty());
        assert_eq!(
            proj.milestones[0].mid_stages[0].subtasks[0].status,
            project::SubtaskStatus::AwaitingConfirmation
        );
        assert!(proj.milestones[0].mid_stages[0].subtasks[0]
            .execution_result
            .as_ref()
            .is_some_and(|result| result.success));
        assert!(proj.milestones[0].mid_stages[0].subtasks[0]
            .test_result
            .as_ref()
            .is_some_and(|result| result.passed));
        assert_eq!(
            proj.workflow_state
                .autopilot_state
                .as_ref()
                .map(|autopilot| &autopilot.recovery_action),
            Some(&project::AutopilotRecoveryAction::RetryGitConfirmation)
        );
        assert_eq!(
            crate::git_ops::tag_target(&repo.path_string(), "metheus/auto/v0.1.1/task-1")?,
            Some(legacy_target.clone())
        );
        assert!(repo.git(&["status", "--short"])?.contains("tracked.txt"));

        let retry_count = proj.milestones[0].mid_stages[0].subtasks[0].retry_count;
        let plan_regeneration_count = proj.milestones[0].mid_stages[0].plan_regeneration_count;
        crate::save_project(&proj)?;
        let pipeline = Arc::new(Mutex::new(Some(pipeline_state(
            "legacy-execution",
            PipelineStatus::Paused,
        ))));

        let confirmed =
            confirm_subtask_result_with_pipeline(&pipeline, project_name.clone()).await?;
        let confirmed_subtask = &confirmed.milestones[0].mid_stages[0].subtasks[0];
        assert_eq!(confirmed_subtask.status, project::SubtaskStatus::Passed);
        assert_eq!(confirmed_subtask.retry_count, retry_count);
        assert_eq!(
            confirmed.milestones[0].mid_stages[0].plan_regeneration_count,
            plan_regeneration_count
        );
        assert_eq!(
            confirmed_subtask
                .execution_result
                .as_ref()
                .map(|result| result.output.as_str()),
            Some("approved execution result")
        );
        assert_eq!(
            confirmed_subtask
                .test_result
                .as_ref()
                .map(|result| result.suggestion.as_str()),
            Some("approved quality result")
        );
        let v2_tag = confirmed_subtask
            .auto_tag
            .as_ref()
            .ok_or("确认完成后缺少 V2 标签".to_string())?;
        assert!(v2_tag.starts_with("metheus/v2/subtask/milestone-1/mid-1/subtask-1/"));
        assert!(crate::git_ops::tag_target(&repo.path_string(), v2_tag)?.is_some());
        assert_eq!(
            crate::git_ops::tag_target(&repo.path_string(), "metheus/auto/v0.1.1/task-1")?,
            Some(legacy_target)
        );
        assert!(confirmed.execution_session.is_none());
        let autopilot = confirmed
            .workflow_state
            .autopilot_state
            .as_ref()
            .ok_or("确认完成后缺少自动驾驶状态".to_string())?;
        assert_eq!(
            autopilot.recovery_action,
            project::AutopilotRecoveryAction::None
        );
        assert_eq!(
            autopilot.run_status,
            project::AutopilotRunStatus::WaitingMilestoneReview
        );
        let pipeline = pipeline.lock().await;
        assert_eq!(
            pipeline.as_ref().map(|state| &state.status),
            Some(&PipelineStatus::Idle)
        );
        assert_eq!(
            pipeline.as_ref().map(|state| state.awaiting_confirmation),
            Some(false)
        );
        Ok(())
    }
}
