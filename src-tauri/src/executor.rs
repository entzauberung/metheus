use std::sync::Arc;
use tokio::sync::Mutex;
use crate::project;
use crate::pipeline::{PipelineState, PipelineStatus};

/// 执行子任务的内部实现（可被暂停中断）
pub(crate) async fn execute_subtask_inner(
    project_path: &str,
    prompt: &str,
    subtask_id: &str,
    state: Arc<Mutex<Option<PipelineState>>>,
) -> Result<project::ExecutionResult, project::SubTaskError> {
    // 1. 执行前记录文件列表
    let before_files = crate::test_runner::get_tracked_files(project_path);
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

    // 存储子进程 PID 到 PipelineState，供 stop_execution 快速终止使用
    {
        let child_pid = child.id();
        let mut guard = state.lock().await;
        if let Some(s) = guard.as_mut() {
            s.child_pid = child_pid;
        }
    }
    // 5. 自动应答：信任确认 + 文件写入确认（异步写入 stdin）
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(b"1\n").await.ok();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        /* 安全上限：最大自动应答次数。Claude Code 通常只会在开始前询问 1-3 次确认，
        此处设 20 为兜底。后续可改为动态检测 stdout 中是否包含 "?" 或 "确认" 等
        提示语来决定是否需要继续应答。 */
        const MAX_AUTO_CONFIRM: u32 = 20;
        for _ in 0..MAX_AUTO_CONFIRM {
            stdin.write_all(b"yes\n").await.ok();
        }
        // stdin 在这里 drop，关闭管道
    }
    // 6. 轮询等待进程结束，期间检查暂停标志和超时
    let start_time = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // 进程已结束 → 读取 stdout/stderr
                let output = child.wait_with_output().await.map_err(|e| {
                    project::SubTaskError::ExecutionFailed {
                        message: format!("读取 Claude Code 输出失败: {}", e),
                    }
                })?;
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let success = status.success();
                // 获取改动文件列表
                let after_files = crate::test_runner::get_tracked_files(project_path);
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
                    guard.as_ref().map_or((false, false), |s| {
                        if s.status == PipelineStatus::Failed {
                            (true, true)
                        } else if s.status == PipelineStatus::Paused {
                            (true, false)
                        } else {
                            (false, false)
                        }
                    })
                };
                if should_stop {
                    // 用户点了暂停或停止 → 强制终止 Claude Code
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    // 清理 PID
                    {
                        let mut guard = state.lock().await;
                        if let Some(s) = guard.as_mut() {
                            s.child_pid = None;
                        }
                    }
                    if is_failed {
                        return Err(project::SubTaskError::ExecutionFailed {
                            message: "用户停止执行".to_string(),
                        });
                    }
                    return Err(project::SubTaskError::UserPaused);
                }
                // 检查整体超时
                if start_time.elapsed() > std::time::Duration::from_secs(crate::constants::CLAUDE_CODE_TIMEOUT_SECS) {
                    eprintln!(
                        "[execute_subtask_inner] 子任务 {} 执行超时（已运行 {:.0}s，上限 {}s），强制终止",
                        subtask_id,
                        start_time.elapsed().as_secs(),
                        crate::constants::CLAUDE_CODE_TIMEOUT_SECS
                    );
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(project::SubTaskError::Timeout);
                }
                // 没暂停也没超时 → 等 500ms 再检查
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Err(e) => {
                let _ = child.start_kill();
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
    execute_subtask_inner(&project_path, &prompt, &subtask_id, dummy_state)
        .await
        .map_err(|e| match e {
            project::SubTaskError::UserPaused => "用户暂停".to_string(),
            project::SubTaskError::ExecutionFailed { message } => message,
            project::SubTaskError::Timeout => "执行超时".to_string(),
        })
}
