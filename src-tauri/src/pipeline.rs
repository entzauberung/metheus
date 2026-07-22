use crate::project;
use crate::AppState;
use serde::{Deserialize, Serialize};

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

/// 追加日志条目到 PipelineState，同时更新 current_log 并限制历史上限
pub(crate) fn append_log(state: &mut PipelineState, level: &str, text: String) {
    let entry = LogEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        level: level.to_string(),
        text: text.clone(),
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
    append_log(state, level, text);
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
    let entry = project::ExecutionHistoryEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        level: level.to_string(),
        event_type,
        text,
        milestone_id: milestone_id.map(|s| s.to_string()),
        mid_stage_id: mid_stage_id.map(|s| s.to_string()),
        subtask_id: subtask_id.map(|s| s.to_string()),
    };
    proj.execution_history.push(entry);
    // 限制历史上限
    if proj.execution_history.len() > project::MAX_EXECUTION_HISTORY {
        let excess = proj.execution_history.len() - project::MAX_EXECUTION_HISTORY;
        proj.execution_history.drain(0..excess);
    }
}

/// Acquire the pipeline lock for a new execution while rejecting an existing
/// running session. Keeping this check and the subsequent state reservation
/// under one guard prevents two callers from launching the same subtask.
async fn acquire_pipeline_start<'a>(
    pipeline_state: &'a std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
) -> Result<tokio::sync::MutexGuard<'a, Option<PipelineState>>, String> {
    let guard = pipeline_state.lock().await;
    if guard
        .as_ref()
        .map(|pipeline| pipeline.status == PipelineStatus::Running)
        .unwrap_or(false)
    {
        return Err("已有小阶段正在执行，请等待当前任务结束。".to_string());
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
/// 本命令是唯一修改 `Project` 但返回 `PipelineState` 而非 `Project` 的命令。
/// 原因：
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
/// - 调用方应立即使用返回的 `PipelineState` 更新 `executionStatus`
/// - 调用方应启动执行轮询（`isExecuting = true`）持续获取最新状态
/// - 轮询检测到终态后应调用 `get_project` 刷新完整 Project
#[tauri::command]
pub(crate) async fn execute_current_subtask(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<PipelineState, String> {
    // 以全局流水线锁串行化“校验 + Running 落盘 + 内存状态建立”，阻止重复启动。
    let mut pipeline_guard = acquire_pipeline_start(&state.pipeline_state).await?;

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
    if milestone_id.is_empty() || mid_stage_id.is_empty() {
        return Err("请先选择大阶段和中阶段。".to_string());
    }

    let ms = proj
        .milestones
        .iter()
        .find(|m| m.id == milestone_id)
        .ok_or("大阶段不存在。")?;
    let mid = ms
        .mid_stages
        .iter()
        .find(|m| m.id == mid_stage_id)
        .ok_or("中阶段不存在。")?;

    // Verify plan is approved
    if mid.plan_approved_at.is_none() || mid.plan_revision == 0 {
        return Err("执行计划尚未批准，请先在 Console 中批准执行计划。".to_string());
    }
    crate::plan_contract::validate_subtasks(&mid.subtasks)
        .map_err(|error| format!("执行计划契约无效：{}", error))?;

    // Verify Git workspace is ready
    let ws = get_execution_workspace_status_inner(&project_path)?;
    if !ws.ready {
        return Err(ws.status_message);
    }
    crate::plan_contract::validate_subtasks_in_project(&mid.subtasks, &project_path)
        .map_err(|error| format!("执行计划契约无效：{}", error))?;

    crate::engine::validate_profile(&proj.execution_profile)?;
    let engine_health = crate::engine::check_engine_health(&proj.execution_profile).await;
    if engine_health.status.blocks_execution() {
        return Err(format!("执行引擎不可用：{}", engine_health.message));
    }
    let execution_profile = proj.execution_profile.clone();

    // Find the next pending subtask
    let next_idx = mid
        .subtasks
        .iter()
        .position(|st| st.status == project::SubtaskStatus::Pending)
        .ok_or("没有待执行的小阶段。所有小阶段已执行完成。".to_string())?;

    let subtask = &mid.subtasks[next_idx];
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
    let approved_prompt = if subtask.execution_prompt.is_empty() {
        subtask.prompt.clone()
    } else {
        subtask.execution_prompt.clone()
    };

    let total = mid.subtasks.len();
    let plan_revision = mid.plan_revision;
    let execution_id = format!(
        "execution-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let subtask_statuses = mid
        .subtasks
        .iter()
        .map(|s| SubtaskStatusItem {
            subtask_id: s.id.clone(),
            title: s.title.clone(),
            status: if s.id == subtask_id {
                "executing".to_string()
            } else {
                "waiting".to_string()
            },
            test_result: None,
            retry_count: 0,
        })
        .collect();
    let now = chrono::Utc::now().to_rfc3339();

    // 执行事实和启动历史使用同一个项目对象并在同一事务边界保存。
    write_execution_history(
        &mut proj,
        "info",
        project::ExecutionEventType::UserExecute,
        format!(
            "👆 用户点击执行 ({}/{})：{}（{}）",
            next_idx + 1,
            total,
            subtask_title,
            execution_profile.provider.display_name(),
        ),
        Some(&milestone_id),
        Some(&mid_stage_id),
        Some(&subtask_id),
    );

    // === 阶段一关键修复：执行前先持久化 "Executing" 到磁盘 ===
    // 这样刷新后前端能从磁盘 Project 中知道当前正在执行，
    // 而不是错误地显示"点击执行"。
    {
        let ms = proj
            .milestones
            .iter_mut()
            .find(|m| m.id == milestone_id)
            .ok_or("大阶段不存在。")?;
        let mid = ms
            .mid_stages
            .iter_mut()
            .find(|m| m.id == mid_stage_id)
            .ok_or("中阶段不存在。")?;
        if let Some(st) = mid.subtasks.get_mut(next_idx) {
            st.status = project::SubtaskStatus::Executing;
        }
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
            started_at: now.clone(),
            state_entered_at: now.clone(),
            plan_revision,
            subtask_index: next_idx,
            total_subtasks: total,
            engine_snapshot: execution_profile.clone(),
        });
    }

    write_execution_history(
        &mut proj,
        "info",
        project::ExecutionEventType::SubtaskExecuting,
        format!("▶ 开始执行 ({}/{})：{}", next_idx + 1, total, subtask_title),
        Some(&milestone_id),
        Some(&mid_stage_id),
        Some(&subtask_id),
    );
    crate::save_project(&proj)?;

    // Initialize pipeline state, then return immediately after scheduling the background task.
    let pipeline_state = state.pipeline_state.clone();
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
    tauri::async_runtime::spawn(async move {
        let result = execute_current_subtask_background(
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
            background_pipeline_state.clone(),
        )
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
    pipeline_state: std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
) -> Result<(), BackgroundExecutionFailure> {
    let exec_result = match crate::engine::execute(
        &execution_profile,
        crate::engine::ExecutionRequest {
            project_path: project_path.clone(),
            prompt: approved_prompt.clone(),
            authorized_paths: authorized_paths.clone(),
            subtask_id: subtask_id.clone(),
            execution_id: execution_id.clone(),
        },
        pipeline_state.clone(),
    )
    .await
    {
        Ok(result) => result,
        Err(crate::engine::EngineError::Cancelled) => return Ok(()),
        Err(crate::engine::EngineError::Timeout) => {
            return Err(BackgroundExecutionFailure::new(
                project::RecoveryErrorKind::ExecutionError,
                "执行超时".to_string(),
            ))
        }
        Err(error) => {
            return Err(BackgroundExecutionFailure::new(
                project::RecoveryErrorKind::ExecutionError,
                error.to_string(),
            ))
        }
    };

    if !exec_result.success {
        return Err(BackgroundExecutionFailure::new(
            project::RecoveryErrorKind::ExecutionError,
            if exec_result.error_log.is_empty() {
                format!("{} 非零退出", execution_profile.provider.display_name())
            } else {
                exec_result.error_log.clone()
            },
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

    let test = crate::test_runner::check_subtask_with_context(
        &project_path,
        &subtask_goal,
        &subtask_id,
        &subtask_title,
        &mid_stage_id,
        Some(acceptance_criteria),
        Some(authorized_paths),
        Some(approved_prompt),
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

    // 与暂停命令共用流水线锁，保证 execution_id 校验到项目保存之间不被旧任务穿透。
    let mut pipeline_guard = pipeline_state.lock().await;
    let pipeline_matches = pipeline_guard
        .as_ref()
        .map(|pipeline| {
            pipeline.execution_id == execution_id && pipeline.status == PipelineStatus::Running
        })
        .unwrap_or(false);
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

    {
        let ms = proj
            .milestones
            .iter_mut()
            .find(|milestone| milestone.id == milestone_id)
            .ok_or_else(|| BackgroundExecutionFailure::state_conflict("大阶段不存在。"))?;
        let mid = ms
            .mid_stages
            .iter_mut()
            .find(|mid_stage| mid_stage.id == mid_stage_id)
            .ok_or_else(|| BackgroundExecutionFailure::state_conflict("中阶段不存在。"))?;
        let subtask = mid
            .subtasks
            .get_mut(subtask_idx)
            .ok_or_else(|| BackgroundExecutionFailure::state_conflict("小阶段索引已失效。"))?;
        if subtask.id != subtask_id || subtask.status != project::SubtaskStatus::Executing {
            return Ok(());
        }
        subtask.execution_result = Some(exec_result);
        subtask.test_result = Some(test.clone());
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
        started_at: session.started_at,
        state_entered_at: now_await,
        plan_revision: session.plan_revision,
        subtask_index: subtask_idx,
        total_subtasks: total,
        engine_snapshot: session.engine_snapshot,
    });
    write_execution_history(
        &mut proj,
        "info",
        project::ExecutionEventType::ExecutorComplete,
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
    write_execution_history(
        &mut proj,
        if test.passed { "success" } else { "error" },
        project::ExecutionEventType::TestComplete,
        if test.passed {
            format!(
                "🔍 测试通过 ({}/{})：{}",
                subtask_idx + 1,
                total,
                subtask_title
            )
        } else {
            format!(
                "🔍 测试未通过 ({}/{})：{} — {}",
                subtask_idx + 1,
                total,
                subtask_title,
                test.suggestion
            )
        },
        Some(&milestone_id),
        Some(&mid_stage_id),
        Some(&subtask_id),
    );
    write_execution_history(
        &mut proj,
        "info",
        project::ExecutionEventType::AwaitingConfirmation,
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
}

impl BackgroundExecutionFailure {
    fn new(kind: project::RecoveryErrorKind, message: String) -> Self {
        Self { kind, message }
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
) -> Result<(), String> {
    let mut pipeline_guard = pipeline_state.lock().await;
    let pipeline_matches = pipeline_guard
        .as_ref()
        .map(|pipeline| {
            pipeline.execution_id == execution_id && pipeline.status == PipelineStatus::Running
        })
        .unwrap_or(false);
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

    finalize_execution_failure(
        &mut proj,
        &mut *pipeline_guard,
        subtask_idx,
        &failure.message,
    );
    crate::recovery::begin_execution_recovery(
        &mut proj,
        failure.kind.clone(),
        execution_id,
        &failure.message,
    );
    write_execution_history(
        &mut proj,
        "error",
        project::ExecutionEventType::ExecutionFailed,
        format!(
            "❌ 执行失败 ({}/{}): {} - {}",
            subtask_idx + 1,
            total,
            subtask_title,
            failure.message
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

    let ms = proj
        .milestones
        .iter()
        .find(|m| m.id == session.milestone_id)
        .ok_or("执行会话中的大阶段不存在。".to_string())?;
    let mid = ms
        .mid_stages
        .iter()
        .find(|m| m.id == session.mid_stage_id)
        .ok_or("执行会话中的中阶段不存在。".to_string())?;
    let subtask = mid
        .subtasks
        .get(session.subtask_index)
        .ok_or("执行会话中的小阶段索引越界。".to_string())?;

    // 校验执行结果存在
    let exec_result = subtask
        .execution_result
        .as_ref()
        .ok_or("缺少执行结果，无法确认。".to_string())?;

    // 校验执行结果成功
    if !exec_result.success {
        return Err(format!(
            "执行未成功：{}。请先处理失败后再确认。",
            if exec_result.error_log.is_empty() {
                "无详细说明"
            } else {
                &exec_result.error_log
            }
        ));
    }

    let human_override = subtask
        .human_verification
        .as_ref()
        .is_some_and(|verification| {
            verification.verification_kind == project::VerificationKind::HumanOverride
                && !verification.verification_reason.trim().is_empty()
        });

    // 人工核验是独立的通过通道；真实测试结果保持原值。
    if human_override {
        return Ok(());
    }

    // 校验测试结果存在
    let test_result = subtask
        .test_result
        .as_ref()
        .ok_or("缺少测试结果，无法确认。测试服务可能不可用。".to_string())?;

    // 校验测试结果通过
    if !test_result.passed {
        return Err(format!(
            "测试未通过：{}",
            if test_result.suggestion.is_empty() {
                "无详细说明"
            } else {
                &test_result.suggestion
            }
        ));
    }

    Ok(())
}

/// 执行器失败时修正磁盘任务、会话、流水线和自动驾驶状态
fn finalize_execution_failure(
    proj: &mut project::Project,
    pipeline_state: &mut Option<PipelineState>,
    subtask_idx: usize,
    error_message: &str,
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
    if let Some(ms) = proj
        .milestones
        .iter_mut()
        .find(|m| m.id == proj.current_milestone_id)
    {
        if let Some(mid) = ms
            .mid_stages
            .iter_mut()
            .find(|m| m.id == proj.current_mid_stage_id)
        {
            if let Some(st) = mid.subtasks.get_mut(subtask_idx) {
                st.status = project::SubtaskStatus::Pending;
                st.execution_result = None;
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
    let has_awaiting = proj.milestones.iter().any(|ms| {
        ms.mid_stages.iter().any(|mid| {
            mid.subtasks
                .iter()
                .any(|st| st.status == project::SubtaskStatus::AwaitingConfirmation)
        })
    });
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
    // 仅允许从待确认或质量阻断进入认领
    let allowed = status.eq_ignore_ascii_case("awaiting_confirmation")
        || status.eq_ignore_ascii_case("quality_blocked");
    if !allowed {
        return Err(format!(
            "任务未处于可确认状态（当前：{}），无法提交。",
            status
        ));
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

/// V1 确认小阶段执行结果（用户点击"确认通过"）
#[tauri::command]
pub(crate) async fn confirm_subtask_result(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    // 与后台完成/启动对账共用流水线锁做 CAS 认领，关闭自动确认与人工确认的并发窗口。
    {
        let _guard = state.pipeline_state.lock().await;
        let mut claim_proj = crate::load_project(&project_name)?;
        claim_awaiting_confirmation_under_lock(&mut claim_proj, "confirming")?;
    }

    let mut proj = crate::load_project(&project_name)?;
    let project_path = proj.project_path.clone();

    let milestone_id = proj.current_milestone_id.clone();
    let mid_stage_id = proj.current_mid_stage_id.clone();
    if milestone_id.is_empty() || mid_stage_id.is_empty() {
        release_confirmation_claim(&mut proj, "awaiting_confirmation");
        let _ = crate::save_project(&proj);
        return Err("请先选择大阶段和中阶段。".to_string());
    }

    // 在获取可变借用前，收集当前大阶段其他中阶段的完成状态
    let other_mid_stages_all_completed = {
        let ms_for_check = proj.milestones.iter().find(|m| m.id == milestone_id);
        ms_for_check
            .map(|ms| {
                ms.mid_stages
                    .iter()
                    .filter(|m| m.id != mid_stage_id)
                    .all(|m| m.status == project::MidStageStatus::Completed)
            })
            .unwrap_or(false)
    };

    // 质量门禁：在创建 Git 标签之前校验执行/测试/证据完整性
    // 认领后 session 为 confirming，质量门禁仍按子任务状态判定
    if let Err(gate_reason) = validate_subtask_quality_gate_allowing_claim(&proj) {
        write_execution_history(
            &mut proj,
            "error",
            project::ExecutionEventType::QualityGateBlocked,
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
        let ms = proj
            .milestones
            .iter()
            .find(|m| m.id == milestone_id)
            .ok_or_else(|| "大阶段不存在。".to_string())?;
        let mid = ms
            .mid_stages
            .iter()
            .find(|m| m.id == mid_stage_id)
            .ok_or_else(|| "中阶段不存在。".to_string())?;
        let milestone_title = ms.title.clone();
        let mid_version = mid.version.clone();
        let subtask_idx = mid
            .subtasks
            .iter()
            .position(|s| s.status == project::SubtaskStatus::AwaitingConfirmation)
            .ok_or_else(|| "没有待确认的小阶段。".to_string())?;
        let subtask_id = mid.subtasks[subtask_idx].id.clone();
        let subtask_title = mid.subtasks[subtask_idx].title.clone();
        let authorized_paths = crate::plan_contract::validate_subtask(
            &mid.subtasks[subtask_idx],
            &format!("第 {} 个小阶段", subtask_idx + 1),
        )?;
        Ok::<_, String>((
            milestone_title,
            mid_version,
            subtask_idx,
            subtask_id,
            subtask_title,
            authorized_paths,
        ))
    })();

    let (milestone_title, mid_version, subtask_idx, subtask_id, subtask_title, authorized_paths) =
        match precheck {
            Ok(v) => v,
            Err(msg) => {
                release_confirmation_claim(&mut proj, "awaiting_confirmation");
                let _ = crate::save_project(&proj);
                return Err(msg);
            }
        };

    // Verify Git workspace is still available before tagging
    let ws = match get_execution_workspace_status_inner(&project_path) {
        Ok(ws) => ws,
        Err(e) => {
            release_confirmation_claim(&mut proj, "awaiting_confirmation");
            let _ = crate::save_project(&proj);
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
        release_confirmation_claim(&mut proj, "awaiting_confirmation");
        let _ = crate::save_project(&proj);
        return Err(format!(
            "Git 工作区不可用，无法标记确认：{}",
            ws.status_message
        ));
    }

    let now = chrono::Utc::now().to_rfc3339();

    // 在真实 index 之外捕获任务 diff，先在内存中完成宪法更新，再统一提交。
    let task_diff_result =
        crate::git_ops::capture_authorized_diff(&project_path, &authorized_paths);
    let mut task_diff_text = String::new();
    let mut pending_constitution_entry: Option<project::ConstitutionChangeEntry> = None;

    let generated_file_result = match task_diff_result {
        Ok(diff_text) => {
            task_diff_text = diff_text;
            let diff_summary = crate::diff::extract_diff_summary(&task_diff_text);
            let constitution_path = std::path::Path::new(&project_path).join("CONSTITUTION.md");
            if constitution_path.exists() {
                let old_constitution = std::fs::read_to_string(&constitution_path)
                    .map_err(|error| format!("读取 CONSTITUTION.md 失败：{}", error));
                match old_constitution {
                    Ok(old_constitution) => {
                        match crate::constitution::update_constitution(
                            old_constitution.clone(),
                            diff_summary.clone(),
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

    // 任务文件和 Metheus 生成的宪法更新必须进入同一提交和标签。
    let tag_result = match generated_file_result {
        Ok(generated_file) => {
            crate::git_ops::git_save_subtask(
                project_path.clone(),
                (subtask_idx + 1) as u32,
                mid_version.clone(),
                subtask_title.clone(),
                authorized_paths,
                generated_file,
            )
            .await
        }
        Err(error) => Err(error),
    };

    match tag_result {
        Ok(tag_name) => {
            if let Some(ms) = proj.milestones.iter_mut().find(|m| m.id == milestone_id) {
                if let Some(mid) = ms.mid_stages.iter_mut().find(|m| m.id == mid_stage_id) {
                    if let Some(st) = mid.subtasks.get_mut(subtask_idx) {
                        st.status = project::SubtaskStatus::Passed;
                        st.confirmed_by_user = Some(true);
                        st.confirmed_at = Some(now.clone());
                        st.auto_tag = Some(tag_name);
                    }
                }
            }
        }
        Err(e) => {
            // Git 失败：范围外/残留变更必须恢复基线；纯标签冲突留给人工处理。
            release_confirmation_claim(&mut proj, "awaiting_confirmation");
            let workspace_dirty = get_execution_workspace_status_inner(&project_path)
                .map(|workspace| !workspace.working_tree_clean)
                .unwrap_or(true);
            let recovery = if workspace_dirty {
                if let Some(session) = proj.execution_session.as_mut() {
                    session.active = false;
                    session.status = "execution_failed".to_string();
                    session.failure_message = e.clone();
                    session.state_entered_at = chrono::Utc::now().to_rfc3339();
                }
                project::AutopilotRecoveryAction::RestoreExecutionBaseline
            } else {
                project::AutopilotRecoveryAction::WaitHumanDecision
            };
            if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
                autopilot.run_status = project::AutopilotRunStatus::ErrorStopped;
                autopilot.error_message = e.clone();
                autopilot.last_action = "Git 确认失败".to_string();
                autopilot.last_action_at = chrono::Utc::now().to_rfc3339();
                autopilot.recovery_action = recovery;
            }
            crate::save_project(&proj)?;
            return Err(format!("确认提交失败：{}。任务未标记为通过。", e));
        }
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
    let all_subtasks_passed = proj
        .milestones
        .iter()
        .find(|m| m.id == milestone_id)
        .and_then(|ms| ms.mid_stages.iter().find(|m| m.id == mid_stage_id))
        .map(|mid| {
            mid.subtasks
                .iter()
                .all(|s| s.status == project::SubtaskStatus::Passed)
        })
        .unwrap_or(false);

    let mid_title_for_node_tag = proj
        .milestones
        .iter()
        .find(|m| m.id == milestone_id)
        .and_then(|ms| ms.mid_stages.iter().find(|m| m.id == mid_stage_id))
        .map(|mid| mid.title.clone())
        .unwrap_or_default();
    let mid_version_for_node_tag = mid_version.clone();
    let mid_stage_id_for_node_tag = mid_stage_id.clone();

    if all_subtasks_passed {
        if let Some(ms) = proj.milestones.iter_mut().find(|m| m.id == milestone_id) {
            if let Some(mid) = ms.mid_stages.iter_mut().find(|m| m.id == mid_stage_id) {
                mid.status = project::MidStageStatus::Completed;
                mid.completed_at = Some(now.clone());
            }
            if other_mid_stages_all_completed {
                ms.status = project::MilestoneStatus::Completed;
                ms.review_status = Some("pending_review".to_string());
                ms.review_conclusion = None;
            }
        }
        if other_mid_stages_all_completed {
            proj.workflow_state.current_step = project::WorkflowStep::MilestoneReview;
            proj.workflow_state.review_node_id = milestone_id.clone();
            if proj.workflow_state.autopilot_active {
                let ap = proj
                    .workflow_state
                    .autopilot_state
                    .get_or_insert_with(project::AutopilotState::default);
                ap.active = true;
                ap.target_milestone_id = milestone_id.clone();
                ap.run_status = project::AutopilotRunStatus::WaitingMilestoneReview;
                ap.last_action = format!("到达大阶段边界：{}，等待人工 A/B/C", milestone_title);
                ap.last_action_at = now.clone();
                ap.error_message.clear();
            }
        } else {
            proj.workflow_state.current_step = project::WorkflowStep::MidStageSelection;
            proj.current_mid_stage_id = String::new();
        }
        proj.workflow_state.data_revision += 1;
        proj.workflow_state.last_transition_at = now.clone();
    }

    if all_subtasks_passed {
        write_execution_history(
            &mut proj,
            "success",
            project::ExecutionEventType::MidStageComplete,
            format!(
                "✅ 中阶段完成：{} (v{})",
                mid_title_for_node_tag, mid_version_for_node_tag
            ),
            Some(&milestone_id),
            Some(&mid_stage_id),
            None,
        );
        if other_mid_stages_all_completed {
            write_execution_history(
                &mut proj,
                "success",
                project::ExecutionEventType::AdvanceMilestoneReview,
                format!("📋 推进到大阶段审阅：{}", milestone_title),
                Some(&milestone_id),
                None,
                None,
            );
        } else {
            write_execution_history(
                &mut proj,
                "success",
                project::ExecutionEventType::AdvanceNextMidStage,
                "➡ 推进到下一中阶段选择".to_string(),
                Some(&milestone_id),
                None,
                None,
            );
        }
    }

    // Write execution history: user confirmed
    write_execution_history(
        &mut proj,
        "success",
        project::ExecutionEventType::UserConfirm,
        format!("✅ 用户确认通过：{}", subtask_title),
        Some(&milestone_id),
        Some(&mid_stage_id),
        Some(&subtask_id),
    );

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

    let proj = crate::save_and_reload_project(&proj)?;

    // === 中阶段节点 Git 标签（项目状态已持久化，标签为补充元数据） ===
    if all_subtasks_passed {
        match crate::git_ops::git_save_node(
            project_path.clone(),
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
        let mut guard = state.pipeline_state.lock().await;
        if let Some(s) = guard.as_mut() {
            s.status = PipelineStatus::Idle;
            s.awaiting_confirmation = false;
            append_log(s, "success", format!("✅ 已确认: {}", subtask_title));
        }
    }

    Ok(proj)
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

    let milestone_id = proj.current_milestone_id.clone();
    let mid_stage_id = proj.current_mid_stage_id.clone();

    let locate = (|| {
        let ms = proj
            .milestones
            .iter()
            .find(|m| m.id == milestone_id)
            .ok_or("大阶段不存在。")?;
        let mid = ms
            .mid_stages
            .iter()
            .find(|m| m.id == mid_stage_id)
            .ok_or("中阶段不存在。")?;
        let subtask_idx = mid
            .subtasks
            .iter()
            .position(|s| s.status == project::SubtaskStatus::AwaitingConfirmation)
            .ok_or("没有待确认的小阶段。")?;
        let subtask_id = mid.subtasks[subtask_idx].id.clone();
        let subtask_title = mid.subtasks[subtask_idx].title.clone();
        Ok::<_, &str>((subtask_idx, subtask_id, subtask_title))
    })();

    let (subtask_idx, subtask_id, subtask_title) = match locate {
        Ok(v) => v,
        Err(msg) => {
            release_confirmation_claim(&mut proj, "awaiting_confirmation");
            let _ = crate::save_project(&proj);
            return Err(msg.to_string());
        }
    };

    let now = chrono::Utc::now().to_rfc3339();
    if let Some(ms) = proj.milestones.iter_mut().find(|m| m.id == milestone_id) {
        if let Some(mid) = ms.mid_stages.iter_mut().find(|m| m.id == mid_stage_id) {
            if let Some(st) = mid.subtasks.get_mut(subtask_idx) {
                st.status = project::SubtaskStatus::Rejected;
                st.confirmed_by_user = Some(false);
                st.confirmed_at = Some(now.clone());
                st.confirmation_notes = Some(reason.clone());
            }
        }
    }

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

    let milestone_id = proj.current_milestone_id.clone();
    let mid_stage_id = proj.current_mid_stage_id.clone();
    if milestone_id.is_empty() || mid_stage_id.is_empty() {
        return Err("请先选择大阶段和中阶段。".to_string());
    }

    // 可由会话状态直接定位可恢复任务，不得依赖 retry_count > 0
    let recoverable_subtask_id = proj
        .execution_session
        .as_ref()
        .filter(|session| session.is_recoverable_failure())
        .map(|session| session.subtask_id.clone());

    let ms = proj
        .milestones
        .iter()
        .find(|m| m.id == milestone_id)
        .ok_or("大阶段不存在。")?;
    let mid = ms
        .mid_stages
        .iter()
        .find(|m| m.id == mid_stage_id)
        .ok_or("中阶段不存在。")?;

    let subtask_idx = mid
        .subtasks
        .iter()
        .position(|st| {
            matches!(
                st.status,
                project::SubtaskStatus::Rejected | project::SubtaskStatus::AwaitingConfirmation
            ) || (st.status == project::SubtaskStatus::Pending
                && recoverable_subtask_id
                    .as_ref()
                    .is_some_and(|id| id == &st.id))
                || (st.status == project::SubtaskStatus::Pending && st.retry_count > 0)
        })
        .ok_or(
            "没有可重试的小阶段。只有测试失败、执行失败、人工驳回或恢复中断的任务可以重试。"
                .to_string(),
        )?;

    let subtask = &mid.subtasks[subtask_idx];

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
    restore_git_execution_baseline(&project_path, &restore_target)
        .map_err(|error| format!("Git 基线恢复失败：{}。失败证据已保留。", error))?;

    let now = chrono::Utc::now().to_rfc3339();

    // 基线恢复成功后才清理旧结果并递增重试次数（每次人工确认只 +1）
    let ms = proj
        .milestones
        .iter_mut()
        .find(|m| m.id == milestone_id)
        .ok_or("大阶段不存在。")?;
    let mid = ms
        .mid_stages
        .iter_mut()
        .find(|m| m.id == mid_stage_id)
        .ok_or("中阶段不存在。")?;
    let st = &mut mid.subtasks[subtask_idx];
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
        if !session.active || session.base_commit.is_empty() {
            return None;
        }
        let current_head = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&proj.project_path)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())?;
        if current_head != session.base_commit {
            return None;
        }
        let subtask = proj
            .milestones
            .iter()
            .find(|milestone| milestone.id == session.milestone_id)
            .and_then(|milestone| {
                milestone
                    .mid_stages
                    .iter()
                    .find(|mid_stage| mid_stage.id == session.mid_stage_id)
            })
            .and_then(|mid_stage| {
                mid_stage
                    .subtasks
                    .iter()
                    .find(|subtask| subtask.id == session.subtask_id)
            })?;
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

pub(crate) fn restore_git_execution_baseline(
    project_path: &str,
    target: &str,
) -> Result<(), String> {
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(project_path)
        .output()
        .map_err(|error| format!("git status 失败：{}", error))?;
    if !status_output.status.success() {
        return Err(format!(
            "git status 失败：{}",
            String::from_utf8_lossy(&status_output.stderr).trim()
        ));
    }
    let has_changes = !String::from_utf8_lossy(&status_output.stdout)
        .trim()
        .is_empty();
    if has_changes {
        let stash_output = std::process::Command::new("git")
            .args([
                "stash",
                "push",
                "--include-untracked",
                "-m",
                "metheus_execution_safety_stash",
            ])
            .current_dir(project_path)
            .output()
            .map_err(|error| format!("git stash 失败：{}", error))?;
        if !stash_output.status.success() {
            return Err(format!(
                "git stash 失败：{}",
                String::from_utf8_lossy(&stash_output.stderr).trim()
            ));
        }
    }

    let reset_output = std::process::Command::new("git")
        .args(["reset", "--hard", target])
        .current_dir(project_path)
        .output()
        .map_err(|error| format!("git reset --hard {} 失败：{}", target, error))?;
    if !reset_output.status.success() {
        let reset_error = String::from_utf8_lossy(&reset_output.stderr)
            .trim()
            .to_string();
        if has_changes {
            let pop_output = std::process::Command::new("git")
                .args(["stash", "pop"])
                .current_dir(project_path)
                .output()
                .map_err(|error| {
                    format!(
                        "回退到 {} 失败：{}；恢复安全暂存也失败：{}",
                        target, reset_error, error
                    )
                })?;
            if !pop_output.status.success() {
                return Err(format!(
                    "回退到 {} 失败：{}；恢复安全暂存也失败：{}",
                    target,
                    reset_error,
                    String::from_utf8_lossy(&pop_output.stderr).trim()
                ));
            }
        }
        return Err(format!("回退到 {} 失败：{}", target, reset_error));
    }

    let workspace = get_execution_workspace_status_inner(project_path)?;
    if !workspace.working_tree_clean {
        return Err("Git 回退后工作区仍有残留修改，安全基线验证失败。".to_string());
    }
    Ok(())
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
    if let Err(error) = restore_git_execution_baseline(&proj.project_path, &restore_target) {
        let persisted_error = persist_in_stop_failure(&pipeline_state, proj, &error).await;
        return Err(persisted_error);
    }

    // 4. 只有进程退出且 Git 基线验证通过后，才进入暂停决策。
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(ms) = proj
        .milestones
        .iter_mut()
        .find(|m| m.id == proj.current_milestone_id)
    {
        if let Some(mid) = ms
            .mid_stages
            .iter_mut()
            .find(|m| m.id == proj.current_mid_stage_id)
        {
            for st in &mut mid.subtasks {
                if st.status == project::SubtaskStatus::Executing
                    || st.status == project::SubtaskStatus::AwaitingConfirmation
                {
                    st.status = project::SubtaskStatus::Pending;
                    st.execution_result = None;
                    st.test_result = None;
                }
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
    let deleted_tags: Vec<String> = all_subtasks[cp_idx + 1..]
        .iter()
        .filter_map(|(_, _, st)| st.auto_tag.clone())
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
        deleted_tags,
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

    // Find checkpoint subtask and collect every later tag in global execution order.
    let mut checkpoint_tag: Option<String> = None;
    let mut checkpoint_found = false;
    let mut discarded_tags = Vec::new();

    for ms in &proj.milestones {
        for mid in &ms.mid_stages {
            let mut mid_has_discarded_task = false;
            for st in &mid.subtasks {
                if st.id == checkpoint_subtask_id {
                    checkpoint_tag = st.auto_tag.clone();
                    checkpoint_found = true;
                } else if checkpoint_found {
                    mid_has_discarded_task = true;
                    if let Some(tag) = st.auto_tag.clone() {
                        discarded_tags.push(tag);
                    }
                }
            }
            if mid_has_discarded_task && !mid.git_tag.is_empty() {
                discarded_tags.push(mid.git_tag.clone());
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
    crate::git_ops::delete_tags(&project_path, &discarded_tags)
        .map_err(|error| format!("清理废弃 Git 标签失败：{}", error))?;

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
        for mid in &ms.mid_stages {
            for st in &mid.subtasks {
                if st.status == project::SubtaskStatus::Passed {
                    last = Some(st.clone());
                }
            }
        }
    }
    last
}

/// 执行状态对账结果
#[derive(Debug, Clone)]
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
            "quality_blocked" | "QualityBlocked"
        )
    {
        if session.subtask_id.is_empty() {
            return ExecutionReconciliation::SessionInvalid;
        }
        return ExecutionReconciliation::AwaitingConfirmation;
    }

    // Check session validity
    if !session.active || session.subtask_id.is_empty() {
        return ExecutionReconciliation::SessionInvalid;
    }

    // Check data consistency: session milestone/mid_stage match current
    if proj.current_milestone_id != session.milestone_id
        || proj.current_mid_stage_id != session.mid_stage_id
    {
        return ExecutionReconciliation::DataConflict;
    }

    // Check if referenced subtask still exists
    let subtask_exists = proj
        .milestones
        .iter()
        .filter(|ms| ms.id == session.milestone_id)
        .flat_map(|ms| ms.mid_stages.iter())
        .filter(|mid| mid.id == session.mid_stage_id)
        .flat_map(|mid| mid.subtasks.iter())
        .any(|st| st.id == session.subtask_id);

    if !subtask_exists {
        return ExecutionReconciliation::DataConflict;
    }

    match session.status.as_str() {
        "executing" | "recovering" => {
            match pipeline_status {
                // 内存 PipelineState 存在且正在运行 → 真执行中
                Some(ps)
                    if ps.status == PipelineStatus::Running
                        && (session.execution_id.is_empty()
                            || ps.execution_id == session.execution_id) =>
                {
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
                // Reset the Executing/Awaiting subtask to Pending
                if let Some(ms) = proj
                    .milestones
                    .iter_mut()
                    .find(|m| m.id == session.milestone_id)
                {
                    if let Some(mid) = ms
                        .mid_stages
                        .iter_mut()
                        .find(|m| m.id == session.mid_stage_id)
                    {
                        for st in &mut mid.subtasks {
                            if st.status == project::SubtaskStatus::Executing
                                || (st.status == project::SubtaskStatus::AwaitingConfirmation
                                    && st
                                        .execution_result
                                        .as_ref()
                                        .map(|r| !r.success)
                                        .unwrap_or(false))
                            {
                                st.status = project::SubtaskStatus::Pending;
                                st.execution_result = None;
                                st.test_result = None;
                            }
                        }
                    }
                }
            }
            // 自动驾驶显式标记恢复动作，不得靠错误文本猜测
            if proj.workflow_state.autopilot_active {
                if let Some(ref mut ap) = proj.workflow_state.autopilot_state {
                    let interrupted_recovery = proj.workflow_state.recovery_state.is_some();
                    ap.run_status = if interrupted_recovery {
                        project::AutopilotRunStatus::Running
                    } else {
                        project::AutopilotRunStatus::ErrorStopped
                    };
                    ap.last_action = if interrupted_recovery {
                        "自动修复进程失联，准备从基线重新执行".to_string()
                    } else {
                        "执行会话失联，需要恢复执行基线".to_string()
                    };
                    ap.last_action_at = now;
                    if ap.error_message.is_empty() {
                        ap.error_message = "执行进程失联，请先恢复执行基线后再继续。".to_string();
                    }
                    ap.recovery_action = if interrupted_recovery {
                        project::AutopilotRecoveryAction::RunAutomaticRecovery
                    } else {
                        project::AutopilotRecoveryAction::RestoreExecutionBaseline
                    };
                }
            }
            if let Some(recovery) = proj.workflow_state.recovery_state.as_mut() {
                recovery.error_kind = project::RecoveryErrorKind::ExecutionError;
                recovery.phase = project::RecoveryPhase::Diagnosing;
                recovery.last_repair_summary = "恢复进程中断；下次尝试将先恢复执行基线".to_string();
                recovery.updated_at = chrono::Utc::now().to_rfc3339();
            }
            proj.workflow_state.data_revision += 1;
            true
        }
        ExecutionReconciliation::SessionInvalid => {
            proj.execution_session = None;
            if proj.workflow_state.current_step == project::WorkflowStep::Execution {
                // No valid session in Execution step → go back
                proj.workflow_state.current_step = project::WorkflowStep::MidStageSelection;
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
                proj.workflow_state.current_step = project::WorkflowStep::MidStageSelection;
                proj.workflow_state.data_revision += 1;
                proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
            }
            true
        }
    }
}

/// 在调用方已持有流水线互斥权时，对最新项目快照做执行对账并可选写盘。
///
/// 必须在持有 `pipeline_state` 锁期间调用：先取锁、再 load、再对账/保存，
/// 避免“先读旧盘 → 后台完成写盘 → 用旧快照覆盖”的窗口（与 ED Stop 同构）。
pub(crate) fn reconcile_loaded_project_under_pipeline_lock(
    proj: &mut project::Project,
    pipeline_status: Option<&PipelineState>,
) -> bool {
    let reconciliation = reconcile_execution_state(proj, pipeline_status);
    apply_execution_reconciliation(proj, &reconciliation)
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
    let mut proj = crate::load_project(&project_name)?;
    let modified = reconcile_loaded_project_under_pipeline_lock(&mut proj, guard.as_ref());

    if modified {
        crate::save_project(&proj)?;
        // 仍在锁内重读，保证返回值与磁盘最终事实一致
        let reloaded = crate::load_project(&project_name)?;
        drop(guard);
        Ok(reloaded)
    } else {
        drop(guard);
        Ok(proj)
    }
}

/// 应用启动恢复确认：实际恢复 Git 基线；失败时保留会话与证据，禁止谎称已恢复
#[tauri::command]
pub(crate) async fn acknowledge_execution_recovery(
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;
    let project_path = proj.project_path.clone();

    let session = proj
        .execution_session
        .as_ref()
        .filter(|session| {
            matches!(
                session.parsed_status(),
                project::ExecutionSessionStatus::SessionLost
                    | project::ExecutionSessionStatus::StopFailed
                    | project::ExecutionSessionStatus::ExecutionFailed
            )
        })
        .ok_or("当前没有需要恢复的执行失败会话。".to_string())?;

    let base_commit = session.base_commit.clone();
    let subtask_id = session.subtask_id.clone();
    let subtask_title = session.subtask_title.clone();
    let milestone_id = session.milestone_id.clone();
    let mid_stage_id = session.mid_stage_id.clone();

    let restore_target = if base_commit.is_empty() {
        find_last_passed_subtask(&proj)
            .and_then(|st| st.auto_tag)
            .unwrap_or_else(|| "HEAD".to_string())
    } else {
        base_commit
    };

    // Git 恢复失败：保留失败会话、基线和错误证据，自动驾驶保持 ErrorStopped
    restore_git_execution_baseline(&project_path, &restore_target).map_err(|error| {
        format!(
            "Git 基线恢复失败：{}。失败证据已保留，请勿认为已恢复到安全状态。",
            error
        )
    })?;

    // 基线恢复成功后才清除会话
    proj.execution_session = None;
    proj.workflow_state.recovery_state = None;

    // 确保受影响任务为 Pending，可再次执行
    if let Some(ms) = proj.milestones.iter_mut().find(|m| m.id == milestone_id) {
        if let Some(mid) = ms.mid_stages.iter_mut().find(|m| m.id == mid_stage_id) {
            if let Some(st) = mid.subtasks.iter_mut().find(|st| st.id == subtask_id) {
                if st.status != project::SubtaskStatus::Passed {
                    st.status = project::SubtaskStatus::Pending;
                    st.execution_result = None;
                    st.test_result = None;
                }
            }
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
        if ap.recovery_action == project::AutopilotRecoveryAction::RestoreExecutionBaseline {
            ap.recovery_action = project::AutopilotRecoveryAction::None;
            ap.error_message = String::new();
            ap.last_action = format!("已恢复执行基线：{}", subtask_title);
            ap.last_action_at = now.clone();
            // 基线恢复是完整恢复命令，成功后直接回到自动推进。
            if ap.run_status == project::AutopilotRunStatus::ErrorStopped {
                ap.run_status = project::AutopilotRunStatus::Running;
            }
        }
    }

    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    crate::save_and_reload_project(&proj)
}

fn find_current_subtask(proj: &project::Project) -> Option<project::Subtask> {
    for ms in &proj.milestones {
        for mid in &ms.mid_stages {
            for st in &mid.subtasks {
                if st.status == project::SubtaskStatus::Executing
                    || st.status == project::SubtaskStatus::AwaitingConfirmation
                {
                    return Some(st.clone());
                }
            }
        }
    }
    // Fallback: find first Pending
    for ms in &proj.milestones {
        for mid in &ms.mid_stages {
            for st in &mid.subtasks {
                if st.status == project::SubtaskStatus::Pending {
                    return Some(st.clone());
                }
            }
        }
    }
    None
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
            started_at: "2026-07-20T00:00:00Z".to_string(),
            state_entered_at: "2026-07-20T00:00:00Z".to_string(),
            plan_revision: 1,
            subtask_index: 0,
            total_subtasks: 1,
            engine_snapshot: project::ExecutionProfile::default(),
        }
    }

    fn execution_project(
        project_name: &str,
        project_path: &Path,
        subtask_status: project::SubtaskStatus,
        session: Option<project::ExecutionSession>,
    ) -> project::Project {
        let mut proj = project::Project::new(project_name);
        proj.project_path = project_path.to_string_lossy().to_string();
        proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        proj.current_milestone_id = "milestone-1".to_string();
        proj.current_mid_stage_id = "mid-1".to_string();
        proj.milestones = vec![test_milestone(subtask_status)];
        proj.execution_session = session;
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
    fn workspace_classifies_active_task_changes_separately() -> Result<(), String> {
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

        restore_git_execution_baseline(&repo.path_string(), &baseline)?;
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
        });
        crate::save_project(&proj)?;

        let updated = acknowledge_execution_recovery(project_name).await?;
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
        });
        crate::save_project(&proj)?;

        let status = refresh_execution_workspace(project_name.clone()).await?;
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

        let status = refresh_execution_workspace(project_name).await?;
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
        let recovery = proj.workflow_state.recovery_state.as_ref().unwrap();
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
    fn successful_retest_clears_recovery_and_returns_to_autopilot() -> Result<(), String> {
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
    fn human_override_passes_gate_without_mutating_failed_test() {
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
        subtask.human_verification = Some(project::HumanVerification {
            verification_kind: project::VerificationKind::HumanOverride,
            verification_reason: "manual smoke test".to_string(),
            verified_at: "2026-07-21T00:00:00Z".to_string(),
            original_test_failure: "runner unavailable".to_string(),
        });

        assert!(validate_subtask_quality_gate(&proj).is_ok());
        assert_eq!(
            proj.milestones[0].mid_stages[0].subtasks[0]
                .test_result
                .as_ref()
                .map(|test| test.passed),
            Some(false)
        );
    }

    #[test]
    fn running_recovery_session_survives_startup_reconciliation() {
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
        });
        let mut pipeline = Some(pipeline_state("execution-x", PipelineStatus::Running));
        finalize_execution_failure(&mut proj, &mut pipeline, 0, "timeout");

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
    fn incomplete_confirmation_claim_reconciles_as_awaiting() {
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
    fn claim_confirmation_is_exclusive() -> Result<(), String> {
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

        let mut second = crate::load_project(&project_name)?;
        let err = claim_awaiting_confirmation_under_lock(&mut second, "confirming")
            .err()
            .ok_or("第二次认领应失败".to_string())?;
        assert!(err.contains("正在进行中") || err.contains("重复"));
        Ok(())
    }
}
