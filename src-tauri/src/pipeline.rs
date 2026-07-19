use serde::{Deserialize, Serialize};
use crate::project;
use crate::AppState;


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
fn append_log(state: &mut PipelineState, level: &str, text: String) {
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

/// 写入持久化执行历史到 Project 磁盘文件。
/// 返回 Result 确保写入失败能被上层感知和处理。
fn write_execution_history(
    project_name: &str,
    level: &str,
    event_type: project::ExecutionEventType,
    text: String,
    milestone_id: Option<&str>,
    mid_stage_id: Option<&str>,
    subtask_id: Option<&str>,
) -> Result<(), String> {
    let mut proj = crate::load_project(project_name)?;
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
    crate::save_project(&proj)
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
    let proj = crate::load_project(&project_name)?;
    let project_path = proj.project_path.clone();

    let milestone_id = &proj.current_milestone_id;
    let mid_stage_id = &proj.current_mid_stage_id;
    if milestone_id.is_empty() || mid_stage_id.is_empty() {
        return Err("请先选择大阶段和中阶段。".to_string());
    }

    let ms = proj.milestones.iter().find(|m| m.id == *milestone_id).ok_or("大阶段不存在。")?;
    let mid = ms.mid_stages.iter().find(|m| m.id == *mid_stage_id).ok_or("中阶段不存在。")?;

    // Verify plan is approved
    if mid.plan_approved_at.is_none() || mid.plan_revision == 0 {
        return Err("执行计划尚未批准，请先在 Console 中批准执行计划。".to_string());
    }

    // Verify Git workspace is ready
    let ws = get_execution_workspace_status_inner(&project_path)?;
    if !ws.ready {
        return Err(ws.status_message);
    }

    // Find the next pending subtask
    let next_idx = mid.subtasks.iter().position(|st| {
        st.status == project::SubtaskStatus::Pending
    }).ok_or("没有待执行的小阶段。所有小阶段已执行完成。".to_string())?;

    let subtask = &mid.subtasks[next_idx];
    let subtask_id = subtask.id.clone();
    let subtask_title = subtask.title.clone();
    let approved_prompt = if subtask.execution_prompt.is_empty() {
        subtask.prompt.clone()
    } else {
        subtask.execution_prompt.clone()
    };

    let total = mid.subtasks.len();
    let now = chrono::Utc::now().to_rfc3339();

    // Write execution history: user clicked execute
    write_execution_history(
        &project_name, "info", project::ExecutionEventType::UserExecute,
        format!("👆 用户点击执行 ({}/{})：{}", next_idx + 1, total, subtask_title),
        Some(milestone_id), Some(mid_stage_id), Some(&subtask_id),
    )?;

    // === 阶段一关键修复：执行前先持久化 "Executing" 到磁盘 ===
    // 这样刷新后前端能从磁盘 Project 中知道当前正在执行，
    // 而不是错误地显示"点击执行"。
    {
        let mut proj = crate::load_project(&project_name)?;
        let ms = proj.milestones.iter_mut().find(|m| m.id == *milestone_id).ok_or("大阶段不存在。")?;
        let mid = ms.mid_stages.iter_mut().find(|m| m.id == *mid_stage_id).ok_or("中阶段不存在。")?;
        if let Some(st) = mid.subtasks.get_mut(next_idx) {
            st.status = project::SubtaskStatus::Executing;
        }
        proj.execution_session = Some(project::ExecutionSession {
            active: true,
            milestone_id: milestone_id.clone(),
            mid_stage_id: mid_stage_id.clone(),
            subtask_id: subtask_id.clone(),
            subtask_title: subtask_title.clone(),
            status: "executing".to_string(),
            started_at: now.clone(),
            state_entered_at: now.clone(),
            plan_revision: mid.plan_revision,
            subtask_index: next_idx,
            total_subtasks: total,
        });
        crate::save_project(&proj)?;
    }

    // Write execution history: execution started
    write_execution_history(
        &project_name, "info", project::ExecutionEventType::SubtaskExecuting,
        format!("▶ 开始执行 ({}/{})：{}", next_idx + 1, total, subtask_title),
        Some(milestone_id), Some(mid_stage_id), Some(&subtask_id),
    )?;

    // Initialize pipeline state (V1: single subtask context)
    let pipeline_state = state.pipeline_state.clone();
    {
        let mut guard = pipeline_state.lock().await;
        *guard = Some(PipelineState {
            mid_stage_id: mid_stage_id.clone(),
            status: PipelineStatus::Running,
            current_subtask_index: next_idx,
            total_subtasks: total,
            subtask_statuses: mid.subtasks.iter().map(|s| SubtaskStatusItem {
                subtask_id: s.id.clone(),
                title: s.title.clone(),
                status: if s.id == subtask_id { "executing".to_string() } else { "waiting".to_string() },
                test_result: None,
                retry_count: 0,
            }).collect(),
            current_log: format!("▶ 执行中 ({}/{})：{}", next_idx + 1, total, subtask_title),
            last_error: None,
            child_pid: None,
            project_name: project_name.clone(),
            milestone_id: milestone_id.clone(),
            plan_revision: mid.plan_revision,
            current_subtask_id: subtask_id.clone(),
            awaiting_confirmation: false,
            log_history: vec![LogEntry {
                timestamp: chrono::Utc::now().to_rfc3339(),
                level: "info".to_string(),
                text: format!("▶ 执行中 ({}/{})：{}", next_idx + 1, total, subtask_title),
            }],
        });
    }

    // Execute the approved prompt
    let exec_result = crate::executor::execute_subtask_inner(
        &project_path,
        &approved_prompt,
        &subtask_id,
        pipeline_state.clone(),
    ).await.map_err(|e| match e {
        project::SubTaskError::UserPaused => "用户暂停".to_string(),
        project::SubTaskError::ExecutionFailed { message } => message,
        project::SubTaskError::Timeout => "执行超时".to_string(),
    })?;

    // Write execution history: executor complete
    write_execution_history(
        &project_name, "info", project::ExecutionEventType::ExecutorComplete,
        format!("✅ 执行完成 ({}/{})：{}", next_idx + 1, total, subtask_title),
        Some(milestone_id), Some(mid_stage_id), Some(&subtask_id),
    )?;

    // Run test
    let test = crate::test_runner::check_subtask(
        &project_path,
        &approved_prompt,
        &subtask_id,
        &subtask_title,
        mid_stage_id,
    ).await.unwrap_or(project::TestResult {
        passed: false,
        issues: vec!["测试服务不可用".to_string()],
        suggestion: "请手动检查".to_string(),
        warnings: vec![],
    });

    // Write execution history: test complete
    write_execution_history(
        &project_name,
        if test.passed { "success" } else { "error" },
        project::ExecutionEventType::TestComplete,
        if test.passed {
            format!("🔍 测试通过 ({}/{})：{}", next_idx + 1, total, subtask_title)
        } else {
            format!("🔍 测试未通过 ({}/{})：{} — {}", next_idx + 1, total, subtask_title, test.suggestion)
        },
        Some(milestone_id), Some(mid_stage_id), Some(&subtask_id),
    )?;

    // Write results back to disk (with execution session update)
    let mut proj = crate::load_project(&project_name)?;
    let ms = proj.milestones.iter_mut().find(|m| m.id == *milestone_id).ok_or("大阶段不存在。")?;
    let mid = ms.mid_stages.iter_mut().find(|m| m.id == *mid_stage_id).ok_or("中阶段不存在。")?;
    if let Some(st) = mid.subtasks.get_mut(next_idx) {
        st.execution_result = Some(exec_result.clone());
        st.test_result = Some(test.clone());
        st.status = project::SubtaskStatus::AwaitingConfirmation;
    }
    // Update execution session to awaiting_confirmation
    let now_await = chrono::Utc::now().to_rfc3339();
    proj.execution_session = Some(project::ExecutionSession {
        active: true,
        milestone_id: milestone_id.clone(),
        mid_stage_id: mid_stage_id.clone(),
        subtask_id: subtask_id.clone(),
        subtask_title: subtask_title.clone(),
        status: "awaiting_confirmation".to_string(),
        started_at: proj.execution_session.as_ref().map(|s| s.started_at.clone()).unwrap_or_else(|| now_await.clone()),
        state_entered_at: now_await.clone(),
        plan_revision: mid.plan_revision,
        subtask_index: next_idx,
        total_subtasks: total,
    });
    crate::save_project(&proj)?;

    // Write execution history: awaiting confirmation
    write_execution_history(
        &project_name, "info", project::ExecutionEventType::AwaitingConfirmation,
        format!("⏳ 待确认 ({}/{})：{}", next_idx + 1, total, subtask_title),
        Some(milestone_id), Some(mid_stage_id), Some(&subtask_id),
    )?;

    // Update pipeline state to awaiting confirmation
    let result_state;
    {
        let mut guard = pipeline_state.lock().await;
        if let Some(s) = guard.as_mut() {
            s.status = PipelineStatus::Paused;
            append_log(s, "info", format!("⏳ 待确认 ({}/{})：{}", next_idx + 1, total, subtask_title));
            s.awaiting_confirmation = true;
            if let Some(stat) = s.subtask_statuses.get_mut(next_idx) {
                stat.status = "testing".to_string();
                stat.test_result = Some(test);
            }
            result_state = s.clone();
        } else {
            return Err("流水线状态丢失".to_string());
        }
    }

    Ok(result_state)
}

/// V1 确认小阶段执行结果（用户点击"确认通过"）
#[tauri::command]
pub(crate) async fn confirm_subtask_result(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;
    let project_path = proj.project_path.clone();

    let milestone_id = proj.current_milestone_id.clone();
    let mid_stage_id = proj.current_mid_stage_id.clone();
    if milestone_id.is_empty() || mid_stage_id.is_empty() {
        return Err("请先选择大阶段和中阶段。".to_string());
    }

    // 在获取可变借用前，收集当前大阶段其他中阶段的完成状态
    let other_mid_stages_all_completed = {
        let ms_for_check = proj.milestones.iter().find(|m| m.id == milestone_id);
        ms_for_check.map(|ms| {
            ms.mid_stages.iter()
                .filter(|m| m.id != mid_stage_id)
                .all(|m| m.status == project::MidStageStatus::Completed)
        }).unwrap_or(false)
    };

    let ms = proj.milestones.iter_mut().find(|m| m.id == milestone_id).ok_or("大阶段不存在。")?;
    let mid = ms.mid_stages.iter_mut().find(|m| m.id == mid_stage_id).ok_or("中阶段不存在。")?;

    // Verify Git workspace is still available before tagging
    let ws = get_execution_workspace_status_inner(&project_path)?;
    if !ws.ready {
        return Err(format!("Git 工作区不可用，无法标记确认：{}", ws.status_message));
    }

    // Collect subtask data before mutation
    let subtask_id = {
        let st = mid.subtasks.iter()
            .find(|s| s.status == project::SubtaskStatus::AwaitingConfirmation)
            .ok_or("没有待确认的小阶段。".to_string())?;
        st.id.clone()
    };
    let subtask_idx = mid.subtasks.iter().position(|s| s.id == subtask_id).unwrap_or(0);
    let subtask_title = mid.subtasks[subtask_idx].title.clone();
    let mid_version = mid.version.clone();

    let now = chrono::Utc::now().to_rfc3339();

    // Create git tag first (before mutating status)
    let tag_result = crate::git_ops::git_save_subtask(
        project_path.clone(),
        (subtask_idx + 1) as u32,
        mid_version.clone(),
        subtask_title.clone(),
    ).await;

    match tag_result {
        Ok(tag_name) => {
            let st = &mut mid.subtasks[subtask_idx];
            st.status = project::SubtaskStatus::Passed;
            st.confirmed_by_user = Some(true);
            st.confirmed_at = Some(now.clone());
            st.auto_tag = Some(tag_name);
        }
        Err(e) => {
            return Err(format!("Git 标签创建失败：{}。任务未标记为通过。", e));
        }
    }

    // === 记录代码变更历史（在标记 Passed 后立即捕获 diff） ===
    {
        let diff_text = capture_diff_snapshot(&project_path);
        if !diff_text.is_empty() {
            let files = extract_changed_files(&diff_text);
            let max_diff_len = 8000usize;
            let (truncated_diff, was_truncated) = if diff_text.len() > max_diff_len {
                (diff_text.chars().take(max_diff_len).collect::<String>() + "\n…（diff 已截断）", true)
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

    // === 中阶段完成检测与工作流推进 ===
    // 检查当前中阶段是否所有小阶段均已通过
    let all_subtasks_passed = mid.subtasks.iter().all(|s| s.status == project::SubtaskStatus::Passed);

    if all_subtasks_passed {
        // 标记中阶段完成
        mid.status = project::MidStageStatus::Completed;
        mid.completed_at = Some(now.clone());

        // Write execution history: mid_stage complete
        write_execution_history(
            &project_name, "success", project::ExecutionEventType::MidStageComplete,
            format!("✅ 中阶段完成：{} (v{})", mid.title, mid.version),
            Some(&milestone_id), Some(&mid_stage_id), None,
        )?;

        // 使用预先收集的状态：当前大阶段其他中阶段是否均已完成
        let all_mid_stages_done = other_mid_stages_all_completed;

        if all_mid_stages_done {
            // 大阶段全部中阶段完成 → 进入大阶段审阅
            proj.workflow_state.current_step = project::WorkflowStep::MilestoneReview;
            proj.workflow_state.review_node_id = milestone_id.clone();

            // Autopilot awareness: signal milestone review boundary
            if proj.workflow_state.autopilot_active {
                if let Some(ref mut ap) = proj.workflow_state.autopilot_state {
                    ap.run_status = project::AutopilotRunStatus::WaitingMilestoneReview;
                    ap.last_action = format!("到达大阶段边界：{}，等待人工 A/B/C", ms.title);
                    ap.last_action_at = now.clone();
                }
            }

            write_execution_history(
                &project_name, "success", project::ExecutionEventType::AdvanceMilestoneReview,
                format!("📋 推进到大阶段审阅：{}", ms.title),
                Some(&milestone_id), None, None,
            )?;
        } else {
            // 大阶段仍有未完成中阶段 → 进入中阶段选择
            proj.workflow_state.current_step = project::WorkflowStep::MidStageSelection;
            proj.current_mid_stage_id = String::new();
            write_execution_history(
                &project_name, "success", project::ExecutionEventType::AdvanceNextMidStage,
                "➡ 推进到下一中阶段选择".to_string(),
                Some(&milestone_id), None, None,
            )?;
        }

        proj.workflow_state.data_revision += 1;
        proj.workflow_state.last_transition_at = now.clone();
    }

    // 提取中阶段节点标签所需数据（在 save_project 释放可变借用前克隆）
    let mid_version_for_node_tag = mid.version.clone();
    let mid_title_for_node_tag = mid.title.clone();
    let mid_stage_id_for_node_tag = mid_stage_id.clone();

    // Update constitution part 2 + record change history entry
    if !project_path.is_empty() {
        let constitution_path = std::path::Path::new(&project_path).join("CONSTITUTION.md");
        if constitution_path.exists() {
            if let Ok(diff_output) = std::process::Command::new("git")
                .args(["diff", "HEAD~1"])
                .current_dir(&project_path)
                .output()
            {
                let diff_str = String::from_utf8_lossy(&diff_output.stdout).to_string();
                let diff_summary = crate::diff::extract_diff_summary(&diff_str);
                let old_constitution = std::fs::read_to_string(&constitution_path).unwrap_or_default();
                let diff_summary_for_history = diff_summary.clone();
                if let Ok(updated) = crate::constitution::update_constitution(old_constitution.clone(), diff_summary).await {
                    let _ = std::fs::write(&constitution_path, &updated);
                    // 仅在宪法实际变更时记录历史
                    if updated != old_constitution {
                        let part2 = extract_constitution_part2(&updated);
                        let token_est = crate::constitution::estimate_tokens(&part2);
                        let summary = build_constitution_change_summary(&diff_summary_for_history);
                        proj.constitution_change_history.push(project::ConstitutionChangeEntry {
                            timestamp: now.clone(),
                            subtask_id: subtask_id.clone(),
                            subtask_title: subtask_title.clone(),
                            change_summary: summary,
                            token_estimate: token_est,
                        });
                        const MAX_CONSTITUTION_HISTORY: usize = 50;
                        if proj.constitution_change_history.len() > MAX_CONSTITUTION_HISTORY {
                            let excess = proj.constitution_change_history.len() - MAX_CONSTITUTION_HISTORY;
                            proj.constitution_change_history.drain(0..excess);
                        }
                    }
                }
            }
        }
    }

    // Write execution history: user confirmed
    write_execution_history(
        &project_name, "success", project::ExecutionEventType::UserConfirm,
        format!("✅ 用户确认通过：{}", subtask_title),
        Some(&milestone_id), Some(&mid_stage_id), Some(&subtask_id),
    )?;

    // Clear execution session before saving (小阶段已确认)
    proj.execution_session = None;

    let proj = crate::save_and_reload_project(&proj)?;

    // === 中阶段节点 Git 标签（项目状态已持久化，标签为补充元数据） ===
    if all_subtasks_passed {
        match crate::git_ops::git_save_node(
            project_path.clone(),
            mid_version_for_node_tag,
            mid_title_for_node_tag,
        ).await {
            Ok(node_tag) => {
                // 更新中阶段的 git_tag 字段
                if let Err(e) = crate::git_ops::save_tag_to_mid_stage(
                    &project_name,
                    &mid_stage_id_for_node_tag,
                    &node_tag,
                ) {
                    eprintln!("[execution] 中阶段 git_tag 写入失败（项目状态已推进）：{}", e);
                }
            }
            Err(e) => {
                eprintln!("[execution] 中阶段节点标签创建失败（项目状态已推进）：{}", e);
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
    let mut proj = crate::load_project(&project_name)?;

    let milestone_id = &proj.current_milestone_id;
    let mid_stage_id = &proj.current_mid_stage_id;

    let ms = proj.milestones.iter_mut().find(|m| m.id == *milestone_id).ok_or("大阶段不存在。")?;
    let mid = ms.mid_stages.iter_mut().find(|m| m.id == *mid_stage_id).ok_or("中阶段不存在。")?;

    let subtask_idx = mid.subtasks.iter()
        .position(|s| s.status == project::SubtaskStatus::AwaitingConfirmation)
        .ok_or("没有待确认的小阶段。".to_string())?;

    // Capture data before mutation
    let subtask_id = mid.subtasks[subtask_idx].id.clone();
    let subtask_title = mid.subtasks[subtask_idx].title.clone();

    let now = chrono::Utc::now().to_rfc3339();
    let st = &mut mid.subtasks[subtask_idx];
    st.status = project::SubtaskStatus::Rejected;
    st.confirmed_by_user = Some(false);
    st.confirmed_at = Some(now.clone());
    st.confirmation_notes = Some(reason.clone());

    // Write execution history: user rejected
    write_execution_history(
        &project_name, "error", project::ExecutionEventType::UserReject,
        format!("❌ 用户驳回：{} — {}", subtask_title, reason),
        Some(milestone_id), Some(mid_stage_id), Some(&subtask_id),
    )?;

    // Clear execution session
    proj.execution_session = None;

    let proj = crate::save_and_reload_project(&proj)?;

    // Clear pipeline state
    {
        let mut guard = state.pipeline_state.lock().await;
        if let Some(s) = guard.as_mut() {
            s.status = PipelineStatus::Idle;
            s.awaiting_confirmation = false;
            append_log(s, "error", format!("❌ 已驳回: {}", reason));
        }
    }

    Ok(proj)
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
            ready: false,
            status_message: "项目路径未设置。".to_string(),
        });
    }

    let path_std = std::path::Path::new(path);
    let path_exists = path_std.exists();
    let is_directory = path_std.is_dir();

    if !path_exists {
        return Ok(project::ExecutionWorkspaceStatus {
            path_exists: false,
            is_directory: false,
            is_git_repo: false,
            has_commits: false,
            git_user_available: false,
            git_email_available: false,
            ready: false,
            status_message: format!("项目路径 {} 不存在。", path),
        });
    }
    if !is_directory {
        return Ok(project::ExecutionWorkspaceStatus {
            path_exists: true,
            is_directory: false,
            is_git_repo: false,
            has_commits: false,
            git_user_available: false,
            git_email_available: false,
            ready: false,
            status_message: format!("项目路径 {} 不是目录。", path),
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
        .map(|o| {
            String::from_utf8_lossy(&o.stdout).trim().len() > 0
        })
        .unwrap_or(false);

    let git_email_available = std::process::Command::new("git")
        .args(["config", "user.email"])
        .current_dir(path)
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout).trim().len() > 0
        })
        .unwrap_or(false);

    let ready = is_git_repo && has_commits && git_user_available && git_email_available;

    let status_message = if ready {
        "Git 工作区已就绪，可以执行小阶段。".to_string()
    } else {
        let mut missing = Vec::new();
        if !is_git_repo { missing.push("Git 仓库未初始化"); }
        if is_git_repo && !has_commits { missing.push("尚无首次提交"); }
        if !git_user_available { missing.push("Git user.name 未配置"); }
        if !git_email_available { missing.push("Git user.email 未配置"); }
        format!("Git 工作区未就绪：{}。", missing.join("、"))
    };

    Ok(project::ExecutionWorkspaceStatus {
        path_exists,
        is_directory,
        is_git_repo,
        has_commits,
        git_user_available,
        git_email_available,
        ready,
        status_message,
    })
}

/// 准备执行工作区：初始化 Git 仓库、创建首次提交（仅在 Execution 步骤可调用）
#[tauri::command]
pub(crate) async fn prepare_execution_workspace(
    project_name: String,
) -> Result<project::ExecutionWorkspaceStatus, String> {
    let proj = crate::load_project(&project_name)?;

    if proj.workflow_state.current_step != project::WorkflowStep::Execution {
        return Err(format!(
            "当前步骤为 {:?}，只有 Execution 步骤可以准备执行工作区",
            proj.workflow_state.current_step
        ));
    }

    // Write execution history: user requested workspace preparation
    write_execution_history(
        &project_name, "info", project::ExecutionEventType::WorkspacePrepare,
        "🔧 用户点击准备执行环境".to_string(),
        None, None, None,
    )?;

    let path = &proj.project_path;
    if path.is_empty() {
        return Err("项目路径未设置。".to_string());
    }

    let path_std = std::path::Path::new(path);
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
            .current_dir(path)
            .output()
            .map_err(|e| format!("git init 失败：{}", e))?;
        if !init.status.success() {
            let stderr = String::from_utf8_lossy(&init.stderr);
            return Err(format!("git init 失败：{}", stderr.chars().take(200).collect::<String>()));
        }
    }

    // Check git identity
    let user_name = std::process::Command::new("git")
        .args(["config", "user.name"])
        .current_dir(path)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let user_email = std::process::Command::new("git")
        .args(["config", "user.email"])
        .current_dir(path)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    if user_name.is_empty() || user_email.is_empty() {
        write_execution_history(
            &project_name, "error", project::ExecutionEventType::WorkspacePrepareFailed,
            format!("Git 身份未配置（user.name={:?}, user.email={:?}）", user_name, user_email),
            None, None, None,
        )?;
        return Err(format!(
            "Git 身份未配置（user.name={:?}, user.email={:?}）。请在项目目录下执行 git config user.name 和 git config user.email。",
            user_name, user_email
        ));
    }

    // Create initial commit if no commits exist
    let has_commits = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !has_commits {
        std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(path)
            .output()
            .map_err(|e| format!("git add 失败：{}", e))?;
        let commit = std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "初始提交（由 Metheus 自动创建）"])
            .current_dir(path)
            .output()
            .map_err(|e| format!("git commit 失败：{}", e))?;
        if !commit.status.success() {
            let stderr = String::from_utf8_lossy(&commit.stderr);
            if !stderr.contains("nothing to commit") {
                return Err(format!("git commit 失败：{}", stderr.chars().take(200).collect::<String>()));
            }
        }
    }

    // Write execution history: workspace ready
    write_execution_history(
        &project_name, "success", project::ExecutionEventType::WorkspaceReady,
        "Git 工作区已就绪，可以执行小阶段。".to_string(),
        None, None, None,
    )?;

    // Re-probe and return status
    get_execution_workspace_status_inner(path)
}

/// Internal helper: probe workspace status from path
fn get_execution_workspace_status_inner(path: &str) -> Result<project::ExecutionWorkspaceStatus, String> {
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
            ready: false,
            status_message: if !path_exists {
                format!("项目路径 {} 不存在。", path)
            } else {
                format!("项目路径 {} 不是目录。", path)
            },
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

    let ready = is_git_repo && has_commits && git_user_available && git_email_available;

    let status_message = if ready {
        "Git 工作区已就绪，可以执行小阶段。".to_string()
    } else {
        let mut missing = Vec::new();
        if !is_git_repo { missing.push("Git 仓库未初始化"); }
        if is_git_repo && !has_commits { missing.push("尚无首次提交"); }
        if !git_user_available { missing.push("Git user.name 未配置"); }
        if !git_email_available { missing.push("Git user.email 未配置"); }
        format!("Git 工作区未就绪：{}。", missing.join("、"))
    };

    Ok(project::ExecutionWorkspaceStatus {
        path_exists,
        is_directory,
        is_git_repo,
        has_commits,
        git_user_available,
        git_email_available,
        ready,
        status_message,
    })
}

// ===================================================================
// V1 暂停与回退命令
// ===================================================================

/// V1 In Stop：立即终止当前子进程，回到上一个稳定检查点
#[tauri::command]
pub(crate) async fn request_in_stop(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    // Kill child process
    let child_pid = {
        let mut guard = state.pipeline_state.lock().await;
        if let Some(s) = guard.as_mut() {
            s.status = PipelineStatus::Failed;
            append_log(s, "pause", "⏹ In Stop：立即暂停".to_string());
            let pid = s.child_pid.take();
            s.child_pid = None;
            pid
        } else {
            None
        }
    };

    if let Some(pid) = child_pid {
        #[cfg(unix)]
        { let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).output(); }
        #[cfg(not(unix))]
        { let _ = std::process::Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output(); }
    }

    // Find last passed (stable) subtask
    let last_passed = find_last_passed_subtask(&proj);
    let current_attempt = find_current_subtask(&proj);

    // Revert code to last stable tag if available
    if let Some(ref last) = last_passed {
        if let Some(ref tag) = last.auto_tag {
            let _ = crate::git_ops::git_stash_and_reset_to_tag(&proj.project_path, tag);
        }
    }

    // Save PauseContext
    let now = chrono::Utc::now().to_rfc3339();
    proj.pause_context = Some(project::PauseContext {
        pause_type: "in_stop".to_string(),
        current_subtask_id: current_attempt.as_ref().map(|s| s.id.clone()).unwrap_or_default(),
        last_passed_subtask_id: last_passed.as_ref().map(|s| s.id.clone()).unwrap_or_default(),
        stable_tag: last_passed.as_ref().and_then(|s| s.auto_tag.clone()).unwrap_or_default(),
        paused_at: now.clone(),
        discussion_start_revision: proj.discussion_revision,
        pending_action: String::new(),
    });

    // Write execution history: user requested In Stop
    write_execution_history(
        &project_name, "pause", project::ExecutionEventType::UserInStop,
        "⏹ 用户请求立即暂停 (In Stop)".to_string(),
        current_attempt.as_ref().and_then(|_| {
            proj.milestones.iter().find(|m| m.id == proj.current_milestone_id).map(|m| m.id.as_str())
        }),
        Some(&proj.current_mid_stage_id),
        current_attempt.as_ref().map(|s| s.id.as_str()),
    )?;

    // Clear execution session (execution is being aborted)
    proj.execution_session = None;

    proj.workflow_state.current_step = project::WorkflowStep::PauseDecision;
    proj.workflow_state.data_revision += 1;
    proj.workflow_state.last_transition_at = now;

    crate::save_and_reload_project(&proj)
}

/// V1 ED Stop：当前任务完成后暂停（标记请求，执行器完成后检查）
#[tauri::command]
pub(crate) async fn request_ed_stop(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    // Set ED Stop flag in pipeline state
    {
        let mut guard = state.pipeline_state.lock().await;
        if let Some(s) = guard.as_mut() {
            append_log(s, "pause", "⏸ ED Stop：当前任务完成后将暂停".to_string());
        }
    }

    // Write execution history
    write_execution_history(
        &project_name, "pause", project::ExecutionEventType::UserEdStop,
        "⏸ 用户请求完成后暂停 (ED Stop)".to_string(),
        Some(&proj.current_milestone_id), Some(&proj.current_mid_stage_id), None,
    )?;

    // Save pending action in PauseContext
    let now = chrono::Utc::now().to_rfc3339();
    let current = find_current_subtask(&proj);
    proj.pause_context = Some(project::PauseContext {
        pause_type: "ed_stop".to_string(),
        current_subtask_id: current.as_ref().map(|s| s.id.clone()).unwrap_or_default(),
        last_passed_subtask_id: String::new(),
        stable_tag: String::new(),
        paused_at: now.clone(),
        discussion_start_revision: proj.discussion_revision,
        pending_action: "ed_stop_requested".to_string(),
    });

    proj.workflow_state.pause_reason = project::PauseReason::EDStop;

    crate::save_and_reload_project(&proj)
}

/// V1 暂停决策：继续 / 调整 / 回退
#[tauri::command]
pub(crate) async fn resolve_pause_decision(
    project_name: String,
    action: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    if proj.workflow_state.current_step != project::WorkflowStep::PauseDecision {
        return Err(format!("当前步骤 {:?} 不是 PauseDecision", proj.workflow_state.current_step));
    }

    match action.as_str() {
        "continue" => {
            // Go back to Execution step
            proj.workflow_state.current_step = project::WorkflowStep::Execution;
            proj.workflow_state.pause_reason = project::PauseReason::None;
            proj.pause_context = None;
            write_execution_history(
                &project_name, "info", project::ExecutionEventType::UserContinue,
                "▶ 用户选择继续执行".to_string(), None, None, None,
            )?;
        }
        "adjust" => {
            // Enter Discussion with PauseAdjustment scope
            proj.workflow_state.current_step = project::WorkflowStep::Discussion;
            proj.workflow_state.discussion_scope = project::DiscussionScope::PauseAdjustment;
            // Keep pause_context for reference
            write_execution_history(
                &project_name, "info", project::ExecutionEventType::UserAdjust,
                "🔧 用户选择调整后续方案".to_string(), None, None, None,
            )?;
        }
        "rollback" => {
            // Enter RollbackPreview
            proj.workflow_state.current_step = project::WorkflowStep::RollbackPreview;
            write_execution_history(
                &project_name, "pause", project::ExecutionEventType::UserRollback,
                "↩ 用户选择回退到更早稳定点".to_string(), None, None, None,
            )?;
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
    let cp_idx = all_subtasks.iter()
        .position(|(_, _, st)| st.id == checkpoint_subtask_id)
        .ok_or("未找到检查点小阶段".to_string())?;

    let retained: Vec<String> = all_subtasks[..=cp_idx].iter()
        .map(|(_, _, st)| st.title.clone()).collect();
    let discarded: Vec<String> = all_subtasks[cp_idx + 1..].iter()
        .map(|(_, _, st)| st.title.clone()).collect();
    let deleted_tags: Vec<String> = all_subtasks[cp_idx + 1..].iter()
        .filter_map(|(_, _, st)| st.auto_tag.clone()).collect();

    let target_tag = all_subtasks[cp_idx].2.auto_tag.clone()
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

    // Find checkpoint subtask and its tag
    let mut checkpoint_tag: Option<String> = None;
    let mut checkpoint_idx: Option<(usize, String, String)> = None; // (subtask_idx, mid_stage_id, milestone_id)

    for ms in &proj.milestones {
        for mid in &ms.mid_stages {
            for (idx, st) in mid.subtasks.iter().enumerate() {
                if st.id == checkpoint_subtask_id {
                    checkpoint_tag = st.auto_tag.clone();
                    checkpoint_idx = Some((idx, mid.id.clone(), ms.id.clone()));
                    break;
                }
            }
        }
    }

    let (cp_idx, mid_stage_id, milestone_id) =
        checkpoint_idx.ok_or("未找到检查点小阶段".to_string())?;

    // Execute git rollback
    if let Some(ref tag) = checkpoint_tag {
        crate::git_ops::git_stash_and_reset_to_tag(&project_path, tag)
            .map_err(|e| format!("Git 回退失败：{}", e))?;
    }

    // Update project data: discard subtasks after checkpoint, clear their tags
    for ms in &mut proj.milestones {
        for mid in &mut ms.mid_stages {
            for (idx, st) in mid.subtasks.iter_mut().enumerate() {
                if mid.id == mid_stage_id && ms.id == milestone_id {
                    if idx > cp_idx {
                        st.status = project::SubtaskStatus::RolledBack;
                        st.auto_tag = None;
                        st.execution_result = None;
                        st.test_result = None;
                    }
                }
            }
        }
    }

    // Clear mid-stage tags for rolled-back mid-stages
    for ms in &mut proj.milestones {
        for mid in &mut ms.mid_stages {
            if mid.id == mid_stage_id && ms.id == milestone_id {
                // Keep the mid_stage but clear its completion markers
                if mid.subtasks.iter().all(|s| s.status == project::SubtaskStatus::RolledBack
                    || s.status == project::SubtaskStatus::Pending) {
                    mid.status = project::MidStageStatus::Pending;
                    mid.git_tag.clear();
                    mid.completed_at = None;
                }
            }
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

/// 捕获最近一次提交的 diff（git diff HEAD~1），失败时静默返回空字符串
fn capture_diff_snapshot(project_path: &str) -> String {
    std::process::Command::new("git")
        .args(["diff", "HEAD~1"])
        .current_dir(project_path)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

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
        parts.push(format!("依赖变更：{}", diff.changed_dependencies.join("、")));
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
    /// 启动时可恢复：磁盘 session 存在但内存 PipelineState 尚未建立（应用刚启动 / 迁移中）
    /// 不应立即清理或判为丢失，应保留会话等待后续恢复
    StartupRecoverable,
}

/// 对账执行状态（启动恢复时调用）
///
/// 区分六种情况：
/// - Executing: 磁盘 session=executing + 内存 Running → 恢复轮询
/// - AwaitingConfirmation: 磁盘 session=awaiting_confirmation → 恢复确认界面
/// - SessionLost: 磁盘 session=executing 且内存有状态但非 Running → 进程已死
/// - SessionInvalid: active=false 或字段缺失 → 清理 session
/// - DataConflict: 与当前 milestone/mid_stage 不匹配 → cleanup
/// - StartupRecoverable: 磁盘 session 存在但内存 PipelineState 尚未建立 → 保留等待恢复
pub fn reconcile_execution_state(
    proj: &project::Project,
    pipeline_status: Option<&PipelineState>,
) -> ExecutionReconciliation {
    let session = match proj.execution_session.as_ref() {
        Some(s) => s,
        None => {
            // No session at all — check if we're still in Execution step
            if proj.workflow_state.current_step == project::WorkflowStep::Execution {
                return ExecutionReconciliation::SessionInvalid;
            }
            return ExecutionReconciliation::SessionInvalid;
        }
    };

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
    let subtask_exists = proj.milestones.iter()
        .filter(|ms| ms.id == session.milestone_id)
        .flat_map(|ms| ms.mid_stages.iter())
        .filter(|mid| mid.id == session.mid_stage_id)
        .flat_map(|mid| mid.subtasks.iter())
        .any(|st| st.id == session.subtask_id);

    if !subtask_exists {
        return ExecutionReconciliation::DataConflict;
    }

    match session.status.as_str() {
        "executing" => {
            match pipeline_status {
                // 内存 PipelineState 存在且正在运行 → 真执行中
                Some(ps) if ps.status == PipelineStatus::Running => {
                    ExecutionReconciliation::Executing
                }
                // 内存 PipelineState 存在但不在运行 → 进程已死
                Some(_) => {
                    ExecutionReconciliation::SessionLost
                }
                // 内存 PipelineState 尚未建立（启动迁移中 / 刚启动）
                // → 不判丢失，保留会话等待后续恢复流程处理
                None => {
                    ExecutionReconciliation::StartupRecoverable
                }
            }
        }
        "awaiting_confirmation" => {
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
        ExecutionReconciliation::Executing
        | ExecutionReconciliation::AwaitingConfirmation
        | ExecutionReconciliation::StartupRecoverable => {
            // Valid or recoverable states — keep session, don't modify
            // StartupRecoverable: 内存 PipelineState 尚未建立，保留会话等待后续恢复
            false
        }
        ExecutionReconciliation::SessionLost => {
            // Process died — preserve session with "session_lost" marker
            // so frontend can detect interrupted execution and display recovery info.
            // session.active stays true so the frontend recovery effect triggers.
            if let Some(ref mut session) = proj.execution_session {
                session.status = "session_lost".to_string();
                // Keep active=true — frontend will check status to show "interrupted" state
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

/// 启动时对账执行状态：加载项目 → reconcile → apply → 保存 → 返回磁盘最终 Project。
///
/// 与独立函数 `reconcile_execution_state` + `apply_execution_reconciliation` 的区别：
/// 本命令是一个完整的持久化流程，返回对账并保存后的磁盘事实，供前端启动恢复使用。
#[tauri::command]
pub(crate) async fn reconcile_on_startup(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    let mut proj = crate::load_project(&project_name)?;

    // 获取当前内存中的 PipelineState（可能为 None）
    let pipeline_status = {
        let guard = state.pipeline_state.lock().await;
        guard.clone()
    };

    let reconciliation = reconcile_execution_state(&proj, pipeline_status.as_ref());
    let modified = apply_execution_reconciliation(&mut proj, &reconciliation);

    if modified {
        crate::save_and_reload_project(&proj)
    } else {
        Ok(proj)
    }
}

fn find_current_subtask(proj: &project::Project) -> Option<project::Subtask> {
    for ms in &proj.milestones {
        for mid in &ms.mid_stages {
            for st in &mid.subtasks {
                if st.status == project::SubtaskStatus::Executing
                    || st.status == project::SubtaskStatus::AwaitingConfirmation {
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