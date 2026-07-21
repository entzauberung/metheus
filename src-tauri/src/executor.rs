use crate::pipeline::{PipelineState, PipelineStatus, append_runtime_log};
use crate::project;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// 单条实时日志字符上限（界面展示）
const MAX_RUNTIME_LOG_CHARS: usize = 2_000;
/// 单个输出流累计字节上限（最终 stdout / stderr）
const MAX_STREAM_BYTES: usize = 256 * 1024;

async fn clear_child_pid(state: &Arc<Mutex<Option<PipelineState>>>, execution_id: &str) {
    let mut guard = state.lock().await;
    if let Some(pipeline) = guard.as_mut() {
        if execution_id.is_empty() || pipeline.execution_id == execution_id {
            pipeline.child_pid = None;
        }
    }
}

async fn collect_pipe(reader: JoinHandle<Vec<u8>>, name: &str) -> Vec<u8> {
    match reader.await {
        Ok(bytes) => bytes,
        Err(error) => format!("[读取 Claude Code {} 任务失败: {}]", name, error).into_bytes(),
    }
}

/// 并行分段读取子进程输出流，校验 execution_id 后追加到 PipelineState。
/// 超限时截断并追加明确标记；读取失败记录可见错误但不堵塞管道。
async fn stream_process_pipe(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    stream_name: &str,
    execution_id: String,
    state: Arc<Mutex<Option<PipelineState>>>,
) -> Vec<u8> {
    let mut collected = Vec::new();
    let mut truncated = false;
    let mut buf = [0u8; 4_096];

    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let chunk = &buf[..n];
                if !truncated && collected.len() < MAX_STREAM_BYTES {
                    let remaining = MAX_STREAM_BYTES.saturating_sub(collected.len());
                    if chunk.len() > remaining {
                        collected.extend_from_slice(&chunk[..remaining]);
                        truncated = true;
                    } else {
                        collected.extend_from_slice(chunk);
                    }
                } else if !truncated {
                    truncated = true;
                }

                // 界面日志：容错解码，限制单条长度，校验 execution_id
                let text = String::from_utf8_lossy(chunk);
                let display: String = text.chars().take(MAX_RUNTIME_LOG_CHARS).collect();
                let display = if text.chars().count() > MAX_RUNTIME_LOG_CHARS {
                    format!("{}…[截断]", display)
                } else {
                    display
                };
                let trimmed = display.trim();
                if !trimmed.is_empty() {
                    let mut guard = state.lock().await;
                    if let Some(pipeline) = guard.as_mut() {
                        // In Stop / 会话切换后不得继续追加旧进程输出
                        if pipeline.status == PipelineStatus::Paused
                            || pipeline.status == PipelineStatus::Failed
                        {
                            // 仍继续读管道防堵塞，但不追加日志
                        } else if execution_id.is_empty()
                            || pipeline.execution_id == execution_id
                        {
                            append_runtime_log(
                                pipeline,
                                "info",
                                format!("[{}] {}", stream_name, trimmed),
                            );
                        }
                    }
                }
            }
            Err(error) => {
                let msg = format!("[读取 Claude Code {} 失败: {}]", stream_name, error);
                let mut guard = state.lock().await;
                if let Some(pipeline) = guard.as_mut() {
                    if execution_id.is_empty() || pipeline.execution_id == execution_id {
                        append_runtime_log(pipeline, "error", msg.clone());
                    }
                }
                if collected.len() < MAX_STREAM_BYTES {
                    collected.extend_from_slice(msg.as_bytes());
                }
                break;
            }
        }
    }

    if truncated {
        let marker = format!(
            "\n…[输出已截断，累计超过 {} 字节上限]",
            MAX_STREAM_BYTES
        );
        collected.extend_from_slice(marker.as_bytes());
        let mut guard = state.lock().await;
        if let Some(pipeline) = guard.as_mut() {
            if execution_id.is_empty() || pipeline.execution_id == execution_id {
                append_runtime_log(
                    pipeline,
                    "error",
                    format!(
                        "[{}] 输出已截断，累计超过 {} 字节上限",
                        stream_name, MAX_STREAM_BYTES
                    ),
                );
            }
        }
    }

    collected
}

async fn terminate_child_process(
    child: &mut tokio::process::Child,
    context: &str,
) -> Result<(), project::SubTaskError> {
    if let Err(kill_error) = child.start_kill() {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) | Err(_) => {
                return Err(project::SubTaskError::ExecutionFailed {
                    message: format!("{}时终止 Claude Code 失败：{}", context, kill_error),
                });
            }
        }
    }
    child
        .wait()
        .await
        .map_err(|error| project::SubTaskError::ExecutionFailed {
            message: format!("{}时等待 Claude Code 退出失败：{}", context, error),
        })?;
    Ok(())
}

/// 执行子任务的内部实现（可被暂停中断）
pub(crate) async fn execute_subtask_inner(
    project_path: &str,
    prompt: &str,
    subtask_id: &str,
    execution_id: &str,
    state: Arc<Mutex<Option<PipelineState>>>,
) -> Result<project::ExecutionResult, project::SubTaskError> {
    // 1. 执行前记录文件内容指纹
    let before_files = crate::test_runner::get_file_snapshot(project_path);
    // 2. 拼接完整 prompt（V1：精确执行已批准任务，信息不足时停止）
    let full_prompt = format!(
        "{}\n\n=== V1 执行约束 ===\n\
        1. 只执行上述任务，不得自行扩展文件范围或改变架构。\n\
        2. 信息不足或发现范围外问题时，必须停止并说明阻塞原因，不得自行猜测或扩展。\n\
        3. 完成后不要输出总结，直接结束。",
        prompt
    );
    // 3. 确定模型名（从环境变量读取，带白名单校验和降级兜底）
    let model_env = match std::env::var("METHEUS_MODEL") {
        Ok(model) => model,
        Err(_) => crate::constants::DEEPSEEK_WORKFLOW_MODEL.to_string(),
    };
    const VALID_MODELS: &[&str] = &[crate::constants::DEEPSEEK_WORKFLOW_MODEL];
    let model_name: String = if VALID_MODELS.contains(&model_env.as_str()) {
        model_env
    } else {
        eprintln!(
            "[execute_subtask] 警告：配置的模型名 \"{}\" 不在当前白名单中，使用统一默认模型 \"{}\"",
            model_env,
            crate::constants::DEEPSEEK_WORKFLOW_MODEL,
        );
        crate::constants::DEEPSEEK_WORKFLOW_MODEL.to_string()
    };
    // 4. 用 tokio::process::Command 启动 Claude Code（非阻塞）
    let mut child = tokio::process::Command::new("claude")
        .args([
            "--dangerously-skip-permissions",
            "--model",
            &model_name,
            "-p",
            &full_prompt,
        ])
        .kill_on_drop(true)
        .current_dir(project_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| project::SubTaskError::ExecutionFailed {
            message: format!(
                "无法启动 Claude Code CLI: {}\n请确认 claude 已安装并在 PATH 中",
                e
            ),
        })?;

    // 并行流式读取 stdout/stderr，运行期间持续追加到 PipelineState
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| project::SubTaskError::ExecutionFailed {
            message: "无法捕获 Claude Code stdout".to_string(),
        })?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| project::SubTaskError::ExecutionFailed {
            message: "无法捕获 Claude Code stderr".to_string(),
        })?;
    let stdout_state = state.clone();
    let stderr_state = state.clone();
    let stdout_execution_id = execution_id.to_string();
    let stderr_execution_id = execution_id.to_string();
    let stdout_reader = tokio::spawn(async move {
        stream_process_pipe(&mut stdout, "stdout", stdout_execution_id, stdout_state).await
    });
    let stderr_reader = tokio::spawn(async move {
        stream_process_pipe(&mut stderr, "stderr", stderr_execution_id, stderr_state).await
    });

    // 存储子进程 PID 到 PipelineState，供 stop_execution 快速终止使用
    {
        let child_pid = child.id();
        let mut guard = state.lock().await;
        if let Some(s) = guard.as_mut() {
            if execution_id.is_empty() || s.execution_id == execution_id {
                s.child_pid = child_pid;
            }
        }
    }
    // 5. 自动应答：信任确认 + 文件写入确认（异步写入 stdin）
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        if let Err(error) = stdin.write_all(b"1\n").await {
            let termination = terminate_child_process(&mut child, "写入执行确认失败").await;
            let _stdout = collect_pipe(stdout_reader, "stdout").await;
            let _stderr = collect_pipe(stderr_reader, "stderr").await;
            clear_child_pid(&state, execution_id).await;
            termination?;
            return Err(project::SubTaskError::ExecutionFailed {
                message: format!("写入 Claude Code 执行确认失败：{}", error),
            });
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        /* 安全上限：最大自动应答次数。Claude Code 通常只会在开始前询问 1-3 次确认，
        此处设 20 为兜底。后续可改为动态检测 stdout 中是否包含 "?" 或 "确认" 等
        提示语来决定是否需要继续应答。 */
        const MAX_AUTO_CONFIRM: u32 = 20;
        for _ in 0..MAX_AUTO_CONFIRM {
            if let Err(error) = stdin.write_all(b"yes\n").await {
                let termination = terminate_child_process(&mut child, "写入自动确认失败").await;
                let _stdout = collect_pipe(stdout_reader, "stdout").await;
                let _stderr = collect_pipe(stderr_reader, "stderr").await;
                clear_child_pid(&state, execution_id).await;
                termination?;
                return Err(project::SubTaskError::ExecutionFailed {
                    message: format!("写入 Claude Code 自动确认失败：{}", error),
                });
            }
        }
        // stdin 在这里 drop，关闭管道
    }
    // 6. 轮询等待进程结束，期间检查暂停标志和超时
    let start_time = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = collect_pipe(stdout_reader, "stdout").await;
                let stderr = collect_pipe(stderr_reader, "stderr").await;
                clear_child_pid(&state, execution_id).await;
                let stdout = String::from_utf8_lossy(&stdout).to_string();
                let stderr = String::from_utf8_lossy(&stderr).to_string();
                let success = status.success();
                // 获取新增、修改和删除的文件列表
                let after_files = crate::test_runner::get_file_snapshot(project_path);
                let file_changes = if success {
                    crate::test_runner::detect_changes(&before_files, &after_files, project_path)
                } else {
                    vec![]
                };
                let error_log = if success {
                    String::new()
                } else {
                    format!(
                        "Claude Code 执行失败 (exit code: {:?})\nstderr:\n{}",
                        status.code(),
                        stderr
                    )
                };
                // 完整提示词写入结果输出，但不得作为实时日志刷屏
                let combined_output = format!(
                    "=== 执行日志 ===\n小阶段 ID：{}\n\n=== 提示词 ===\n{}\n\n=== stdout ===\n{}\n=== stderr ===\n{}",
                    subtask_id, full_prompt, stdout, stderr
                );
                return Ok(project::ExecutionResult {
                    success,
                    output: combined_output,
                    error_log,
                    file_changes,
                });
            }
            Ok(None) => {
                // 进程还在运行 → 检查暂停/停止标志
                let (should_stop, is_failed) = {
                    let guard = state.lock().await;
                    guard
                        .as_ref()
                        .map_or((!execution_id.is_empty(), false), |s| {
                            if !execution_id.is_empty() && s.execution_id != execution_id {
                                (true, false)
                            } else if s.status == PipelineStatus::Failed {
                                (true, true)
                            } else if s.status == PipelineStatus::Paused {
                                (true, false)
                            } else {
                                (false, false)
                            }
                        })
                };
                if should_stop {
                    let termination = terminate_child_process(&mut child, "受控暂停").await;
                    let _stdout = collect_pipe(stdout_reader, "stdout").await;
                    let _stderr = collect_pipe(stderr_reader, "stderr").await;
                    clear_child_pid(&state, execution_id).await;
                    termination?;
                    if is_failed {
                        return Err(project::SubTaskError::ExecutionFailed {
                            message: "用户停止执行".to_string(),
                        });
                    }
                    return Err(project::SubTaskError::UserPaused);
                }
                // 检查整体超时
                if start_time.elapsed()
                    > std::time::Duration::from_secs(crate::constants::CLAUDE_CODE_TIMEOUT_SECS)
                {
                    eprintln!(
                        "[execute_subtask_inner] 子任务 {} 执行超时（已运行 {:.0}s，上限 {}s），强制终止",
                        subtask_id,
                        start_time.elapsed().as_secs(),
                        crate::constants::CLAUDE_CODE_TIMEOUT_SECS
                    );
                    let termination = terminate_child_process(&mut child, "执行超时").await;
                    let _stdout = collect_pipe(stdout_reader, "stdout").await;
                    let _stderr = collect_pipe(stderr_reader, "stderr").await;
                    clear_child_pid(&state, execution_id).await;
                    termination?;
                    return Err(project::SubTaskError::Timeout);
                }
                // 没暂停也没超时 → 等 500ms 再检查
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Err(e) => {
                let termination = terminate_child_process(&mut child, "进程状态检查失败").await;
                let _stdout = collect_pipe(stdout_reader, "stdout").await;
                let _stderr = collect_pipe(stderr_reader, "stderr").await;
                clear_child_pid(&state, execution_id).await;
                if let Err(termination_error) = termination {
                    return Err(project::SubTaskError::ExecutionFailed {
                        message: format!(
                            "Claude Code 进程异常: {}；{}",
                            e,
                            match termination_error {
                                project::SubTaskError::ExecutionFailed { message } => message,
                                project::SubTaskError::UserPaused => "用户暂停".to_string(),
                                project::SubTaskError::Timeout => "执行超时".to_string(),
                            }
                        ),
                    });
                }
                return Err(project::SubTaskError::ExecutionFailed {
                    message: format!("Claude Code 进程异常: {}", e),
                });
            }
        }
    }
}

/// Tauri command 壳：前端调用入口，内部委托给 execute_subtask_inner。
/// 前端直接调时没有暂停状态，传一个临时空 state。
#[tauri::command]
pub(crate) async fn execute_subtask(
    project_path: String,
    prompt: String,
    subtask_id: String,
    _milestone_id: String,
    _mid_stage_id: String,
) -> Result<project::ExecutionResult, String> {
    // 前端直接调用时，没有流水线上下文，传空 state
    let dummy_state = Arc::new(Mutex::new(None));
    execute_subtask_inner(&project_path, &prompt, &subtask_id, "", dummy_state)
        .await
        .map_err(|e| match e {
            project::SubTaskError::UserPaused => "用户暂停".to_string(),
            project::SubTaskError::ExecutionFailed { message } => message,
            project::SubTaskError::Timeout => "执行超时".to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{PipelineState, PipelineStatus};
    use std::io::Cursor;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn test_pipeline(execution_id: &str) -> PipelineState {
        PipelineState {
            execution_id: execution_id.to_string(),
            mid_stage_id: "mid-1".to_string(),
            status: PipelineStatus::Running,
            current_subtask_index: 0,
            total_subtasks: 1,
            subtask_statuses: vec![],
            current_log: String::new(),
            last_error: None,
            child_pid: None,
            project_name: String::new(),
            milestone_id: "ms-1".to_string(),
            plan_revision: 1,
            current_subtask_id: "st-1".to_string(),
            awaiting_confirmation: false,
            log_history: vec![],
        }
    }

    #[tokio::test]
    async fn stream_process_pipe_appends_live_output() {
        let state = Arc::new(Mutex::new(Some(test_pipeline("exec-1"))));
        let data = b"hello from claude\nline two\n";
        let collected = stream_process_pipe(
            Cursor::new(data.as_slice()),
            "stdout",
            "exec-1".to_string(),
            state.clone(),
        )
        .await;
        assert!(collected.starts_with(b"hello"));
        let guard = state.lock().await;
        let logs = &guard.as_ref().unwrap().log_history;
        assert!(!logs.is_empty());
        assert!(logs.iter().any(|e| e.text.contains("hello")));
    }

    #[tokio::test]
    async fn stream_process_pipe_drops_stale_execution_id() {
        let state = Arc::new(Mutex::new(Some(test_pipeline("exec-current"))));
        let data = b"stale output should not appear\n";
        let _ = stream_process_pipe(
            Cursor::new(data.as_slice()),
            "stdout",
            "exec-stale".to_string(),
            state.clone(),
        )
        .await;
        let guard = state.lock().await;
        assert!(guard.as_ref().unwrap().log_history.is_empty());
    }

    #[tokio::test]
    async fn stream_process_pipe_stops_append_when_paused() {
        let mut pipeline = test_pipeline("exec-1");
        pipeline.status = PipelineStatus::Paused;
        let state = Arc::new(Mutex::new(Some(pipeline)));
        let data = b"after stop output\n";
        let collected = stream_process_pipe(
            Cursor::new(data.as_slice()),
            "stdout",
            "exec-1".to_string(),
            state.clone(),
        )
        .await;
        // 管道仍被排空
        assert!(!collected.is_empty());
        let guard = state.lock().await;
        assert!(guard.as_ref().unwrap().log_history.is_empty());
    }

    #[tokio::test]
    async fn stream_process_pipe_truncates_oversized_output() {
        let state = Arc::new(Mutex::new(Some(test_pipeline("exec-1"))));
        let oversized = vec![b'x'; MAX_STREAM_BYTES + 100];
        let collected = stream_process_pipe(
            Cursor::new(oversized),
            "stdout",
            "exec-1".to_string(),
            state.clone(),
        )
        .await;
        assert!(collected.len() <= MAX_STREAM_BYTES + 200);
        let text = String::from_utf8_lossy(&collected);
        assert!(text.contains("输出已截断"));
    }
}
