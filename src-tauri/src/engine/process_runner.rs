use super::contract::{EngineError, ProcessOutput, ProcessSpec};
use crate::pipeline::{append_runtime_log, PipelineState, PipelineStatus};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

const MAX_RUNTIME_LOG_CHARS: usize = 2_000;
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
        Err(error) => format!("[读取执行引擎 {name} 输出任务失败: {error}]").into_bytes(),
    }
}

async fn stream_process_pipe(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    stream_name: &str,
    execution_id: String,
    state: Arc<Mutex<Option<PipelineState>>>,
) -> Vec<u8> {
    let mut collected = Vec::new();
    let mut truncated = false;
    let mut buffer = [0u8; 4_096];

    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(size) => {
                let chunk = &buffer[..size];
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

                let text = String::from_utf8_lossy(chunk);
                let display: String = text.chars().take(MAX_RUNTIME_LOG_CHARS).collect();
                let display = if text.chars().count() > MAX_RUNTIME_LOG_CHARS {
                    format!("{display}…[截断]")
                } else {
                    display
                };
                if !display.trim().is_empty() {
                    let mut guard = state.lock().await;
                    if let Some(pipeline) = guard.as_mut() {
                        if pipeline.status != PipelineStatus::Paused
                            && pipeline.status != PipelineStatus::Failed
                            && (execution_id.is_empty() || pipeline.execution_id == execution_id)
                        {
                            append_runtime_log(
                                pipeline,
                                "info",
                                format!("[{stream_name}] {}", display.trim()),
                            );
                        }
                    }
                }
            }
            Err(error) => {
                let message = format!("[读取执行引擎 {stream_name} 失败: {error}]");
                let mut guard = state.lock().await;
                if let Some(pipeline) = guard.as_mut() {
                    if execution_id.is_empty() || pipeline.execution_id == execution_id {
                        append_runtime_log(pipeline, "error", message.clone());
                    }
                }
                if collected.len() < MAX_STREAM_BYTES {
                    collected.extend_from_slice(message.as_bytes());
                }
                break;
            }
        }
    }

    if truncated {
        let marker = format!("\n…[输出已截断，累计超过 {MAX_STREAM_BYTES} 字节上限]");
        collected.extend_from_slice(marker.as_bytes());
        let mut guard = state.lock().await;
        if let Some(pipeline) = guard.as_mut() {
            if execution_id.is_empty() || pipeline.execution_id == execution_id {
                append_runtime_log(
                    pipeline,
                    "error",
                    format!("[{stream_name}] 输出已截断，累计超过 {MAX_STREAM_BYTES} 字节上限"),
                );
            }
        }
    }
    collected
}

async fn terminate_child(
    child: &mut tokio::process::Child,
    display_name: &str,
    context: &str,
) -> Result<(), EngineError> {
    if let Err(kill_error) = child.start_kill() {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) | Err(_) => {
                return Err(EngineError::ProcessFailed(format!(
                    "{context}时终止 {display_name} 失败：{kill_error}"
                )))
            }
        }
    }
    child.wait().await.map_err(|error| {
        EngineError::ProcessFailed(format!("{context}时等待 {display_name} 退出失败：{error}"))
    })?;
    Ok(())
}

pub(super) async fn run_process(
    spec: ProcessSpec,
    project_path: &str,
    execution_id: &str,
    state: Arc<Mutex<Option<PipelineState>>>,
) -> Result<ProcessOutput, EngineError> {
    let mut command = tokio::process::Command::new(&spec.program);
    command
        .args(&spec.args)
        .kill_on_drop(true)
        .current_dir(project_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(if spec.stdin_payload.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        });

    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            EngineError::NotInstalled(format!(
                "无法启动 {}：未在 PATH 中找到可执行文件",
                spec.display_name
            ))
        } else {
            EngineError::StartFailed(format!("无法启动 {}：{error}", spec.display_name))
        }
    })?;

    if let Some(payload) = spec.stdin_payload {
        let mut stdin = child.stdin.take().ok_or_else(|| {
            EngineError::ProtocolError(format!("无法打开 {} stdin", spec.display_name))
        })?;
        stdin.write_all(payload.as_bytes()).await.map_err(|error| {
            EngineError::ProtocolError(format!("写入 {} stdin 失败：{error}", spec.display_name))
        })?;
        drop(stdin);
    }

    let mut stdout = child.stdout.take().ok_or_else(|| {
        EngineError::ProtocolError(format!("无法捕获 {} stdout", spec.display_name))
    })?;
    let mut stderr = child.stderr.take().ok_or_else(|| {
        EngineError::ProtocolError(format!("无法捕获 {} stderr", spec.display_name))
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

    {
        let mut guard = state.lock().await;
        if let Some(pipeline) = guard.as_mut() {
            if execution_id.is_empty() || pipeline.execution_id == execution_id {
                pipeline.child_pid = child.id();
            }
        }
    }

    let started_at = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = collect_pipe(stdout_reader, "stdout").await;
                let stderr = collect_pipe(stderr_reader, "stderr").await;
                clear_child_pid(&state, execution_id).await;
                return Ok(ProcessOutput {
                    stdout: String::from_utf8_lossy(&stdout).to_string(),
                    stderr: String::from_utf8_lossy(&stderr).to_string(),
                    exit_code: status.code(),
                    success: status.success(),
                });
            }
            Ok(None) => {
                let stop_state = {
                    let guard = state.lock().await;
                    guard
                        .as_ref()
                        .map_or(Some(PipelineStatus::Paused), |pipeline| {
                            if !execution_id.is_empty() && pipeline.execution_id != execution_id {
                                Some(PipelineStatus::Paused)
                            } else if matches!(
                                pipeline.status,
                                PipelineStatus::Failed | PipelineStatus::Paused
                            ) {
                                Some(pipeline.status.clone())
                            } else {
                                None
                            }
                        })
                };
                if let Some(stop_state) = stop_state {
                    let termination =
                        terminate_child(&mut child, spec.display_name, "受控停止").await;
                    let _ = collect_pipe(stdout_reader, "stdout").await;
                    let _ = collect_pipe(stderr_reader, "stderr").await;
                    clear_child_pid(&state, execution_id).await;
                    termination?;
                    return if stop_state == PipelineStatus::Failed {
                        Err(EngineError::ProcessFailed("用户停止执行".to_string()))
                    } else {
                        Err(EngineError::Cancelled)
                    };
                }

                if started_at.elapsed()
                    > std::time::Duration::from_secs(
                        crate::constants::EXECUTION_ENGINE_TIMEOUT_SECS,
                    )
                {
                    let termination =
                        terminate_child(&mut child, spec.display_name, "执行超时").await;
                    let _ = collect_pipe(stdout_reader, "stdout").await;
                    let _ = collect_pipe(stderr_reader, "stderr").await;
                    clear_child_pid(&state, execution_id).await;
                    termination?;
                    return Err(EngineError::Timeout);
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Err(error) => {
                let termination =
                    terminate_child(&mut child, spec.display_name, "进程状态检查失败").await;
                let _ = collect_pipe(stdout_reader, "stdout").await;
                let _ = collect_pipe(stderr_reader, "stderr").await;
                clear_child_pid(&state, execution_id).await;
                termination?;
                return Err(EngineError::ProcessFailed(format!(
                    "{} 进程异常：{error}",
                    spec.display_name
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::path::{Path, PathBuf};

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("metheus-engine-{label}-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).expect("应能创建测试目录");
            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[cfg(unix)]
    fn write_fake_cli(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, format!("#!/bin/sh\nset -eu\n{body}\n")).expect("应能写入假 CLI");
        let mut permissions = std::fs::metadata(path)
            .expect("应能读取假 CLI 元数据")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("应能设置假 CLI 执行权限");
    }

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
    async fn stream_output_is_truncated_and_stale_logs_are_dropped() {
        let state = Arc::new(Mutex::new(Some(test_pipeline("current"))));
        let oversized = vec![b'x'; MAX_STREAM_BYTES + 100];
        let collected = stream_process_pipe(
            Cursor::new(oversized),
            "stdout",
            "stale".to_string(),
            state.clone(),
        )
        .await;
        assert!(String::from_utf8_lossy(&collected).contains("输出已截断"));
        assert!(state.lock().await.as_ref().unwrap().log_history.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stdin_process_completes_and_reports_nonzero_exit() {
        let directory = TestDirectory::new("stdin");
        let success_cli = directory.path.join("success-cli");
        write_fake_cli(
            &success_cli,
            "payload=$(cat)\nprintf '%s' \"$payload\" > prompt.txt\nprintf 'complete\\n'",
        );
        let state = Arc::new(Mutex::new(Some(test_pipeline("success"))));
        let output = run_process(
            ProcessSpec {
                display_name: "Fake",
                program: success_cli.into_os_string(),
                args: vec![],
                stdin_payload: Some("approved prompt".to_string()),
            },
            directory.path.to_str().unwrap(),
            "success",
            state.clone(),
        )
        .await
        .expect("假 CLI 应成功");
        assert!(output.success);
        assert!(output.stdout.contains("complete"));
        assert_eq!(
            std::fs::read_to_string(directory.path.join("prompt.txt")).unwrap(),
            "approved prompt"
        );
        assert_eq!(state.lock().await.as_ref().unwrap().child_pid, None);

        let failure_cli = directory.path.join("failure-cli");
        write_fake_cli(&failure_cli, "printf 'expected failure\\n' >&2\nexit 7");
        let failure_state = Arc::new(Mutex::new(Some(test_pipeline("failure"))));
        let output = run_process(
            ProcessSpec {
                display_name: "Fake",
                program: failure_cli.into_os_string(),
                args: vec![],
                stdin_payload: None,
            },
            directory.path.to_str().unwrap(),
            "failure",
            failure_state,
        )
        .await
        .expect("非零退出应作为结构化结果返回");
        assert!(!output.success);
        assert_eq!(output.exit_code, Some(7));
        assert!(output.stderr.contains("expected failure"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stdout_quota_error_from_fake_cli_is_classified() {
        let directory = TestDirectory::new("quota");
        let cli = directory.path.join("quota-cli");
        write_fake_cli(
            &cli,
            "printf 'API Error: 402 Insufficient Balance\\n'\nexit 1",
        );
        let state = Arc::new(Mutex::new(Some(test_pipeline("quota"))));
        let output = run_process(
            ProcessSpec {
                display_name: "Fake",
                program: cli.into_os_string(),
                args: vec![],
                stdin_payload: None,
            },
            directory.path.to_str().unwrap(),
            "quota",
            state,
        )
        .await
        .expect("配额错误应保留为结构化进程输出");
        assert!(!output.success);
        assert!(output.stdout.contains("402 Insufficient Balance"));
        assert_eq!(
            crate::engine::classify_process_failure(
                output.exit_code,
                &output.stdout,
                &output.stderr,
            ),
            crate::project::EngineFailureKind::QuotaExceeded
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn paused_pipeline_cancels_process_and_clears_pid() {
        let directory = TestDirectory::new("cancel");
        let cli = directory.path.join("slow-cli");
        write_fake_cli(&cli, "exec sleep 10");
        let state = Arc::new(Mutex::new(Some(test_pipeline("cancel"))));
        let running_state = state.clone();
        let project_path = directory.path.clone();
        let task = tokio::spawn(async move {
            run_process(
                ProcessSpec {
                    display_name: "Fake",
                    program: cli.into_os_string(),
                    args: vec![],
                    stdin_payload: None,
                },
                project_path.to_str().unwrap(),
                "cancel",
                running_state,
            )
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        state.lock().await.as_mut().unwrap().status = PipelineStatus::Paused;
        let result = task.await.unwrap();
        assert!(matches!(result, Err(EngineError::Cancelled)));
        assert_eq!(state.lock().await.as_ref().unwrap().child_pid, None);
    }
}
