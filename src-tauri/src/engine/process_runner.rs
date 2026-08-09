use super::contract::{EngineError, OutputProtocol, ProcessOutput, ProcessSpec, ProgramSource};
use crate::pipeline::{
    append_runtime_log, append_runtime_log_with_context, set_runtime_debug_log, PipelineState,
    PipelineStatus,
};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

const MAX_RUNTIME_LOG_CHARS: usize = 2_000;
const MAX_FINAL_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_ERROR_TAIL_BYTES: usize = 64 * 1024;
const MAX_JSON_LINE_BYTES: usize = 512 * 1024;

fn event_text(value: &serde_json::Value) -> Option<&str> {
    let event_type = value.get("type").and_then(serde_json::Value::as_str);
    if event_type == Some("thought") {
        return None;
    }
    if event_type == Some("text") {
        return value.get("data").and_then(serde_json::Value::as_str);
    }
    for field in ["content", "text", "message", "result", "response", "data"] {
        if let Some(text) = value.get(field).and_then(serde_json::Value::as_str) {
            return Some(text);
        }
    }
    value
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(serde_json::Value::as_str)
}

fn event_correlation(value: &serde_json::Value) -> Option<String> {
    ["correlation_id", "call_id", "message_id", "id"]
        .into_iter()
        .find_map(|field| value.get(field).and_then(serde_json::Value::as_str))
        .map(str::to_string)
}

#[cfg(test)]
fn normalize_stdout(protocol: OutputProtocol, stdout: String) -> String {
    if protocol == OutputProtocol::RawText {
        return stdout;
    }
    let mut normalized = String::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(value) => {
                if let Some(text) = event_text(&value) {
                    let remaining = MAX_FINAL_OUTPUT_BYTES.saturating_sub(normalized.len());
                    if remaining == 0 {
                        break;
                    }
                    let bytes = text.as_bytes();
                    normalized.push_str(&String::from_utf8_lossy(
                        &bytes[..bytes.len().min(remaining)],
                    ));
                    if !text.ends_with('\n') && normalized.len() < MAX_FINAL_OUTPUT_BYTES {
                        normalized.push('\n');
                    }
                }
            }
            Err(_) => {
                let remaining = MAX_FINAL_OUTPUT_BYTES.saturating_sub(normalized.len());
                if remaining == 0 {
                    break;
                }
                let bytes = line.as_bytes();
                normalized.push_str(&String::from_utf8_lossy(
                    &bytes[..bytes.len().min(remaining)],
                ));
                if normalized.len() < MAX_FINAL_OUTPUT_BYTES {
                    normalized.push('\n');
                }
            }
        }
    }
    if normalized.is_empty() && !stdout.trim().is_empty() {
        stdout
    } else {
        normalized
    }
}

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
    output_protocol: OutputProtocol,
    execution_id: String,
    state: Arc<Mutex<Option<PipelineState>>>,
) -> Vec<u8> {
    let mut collected = Vec::new();
    let mut truncated = false;
    let mut pending_line = Vec::new();
    let mut discarding_oversized_line = false;
    let mut buffer = [0u8; 4_096];

    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(size) => {
                let chunk = &buffer[..size];
                if stream_name == "stderr" {
                    collected.extend_from_slice(chunk);
                    if collected.len() > MAX_ERROR_TAIL_BYTES {
                        let excess = collected.len() - MAX_ERROR_TAIL_BYTES;
                        collected.drain(..excess);
                        truncated = true;
                    }
                    append_stream_log(&state, &execution_id, "error", stream_name, chunk).await;
                } else if output_protocol == OutputProtocol::JsonLines {
                    for byte in chunk {
                        if *byte == b'\n' {
                            if discarding_oversized_line {
                                append_stream_log(
                                    &state,
                                    &execution_id,
                                    "error",
                                    stream_name,
                                    b"JSON Lines event exceeded its bounded line quota and was discarded",
                                )
                                .await;
                            } else {
                                consume_json_line(
                                    &pending_line,
                                    &mut collected,
                                    &mut truncated,
                                    &state,
                                    &execution_id,
                                    stream_name,
                                )
                                .await;
                            }
                            pending_line.clear();
                            discarding_oversized_line = false;
                        } else if !discarding_oversized_line {
                            if pending_line.len() < MAX_JSON_LINE_BYTES {
                                pending_line.push(*byte);
                            } else {
                                pending_line.clear();
                                discarding_oversized_line = true;
                            }
                        }
                    }
                } else {
                    let remaining = MAX_FINAL_OUTPUT_BYTES.saturating_sub(collected.len());
                    if remaining > 0 {
                        collected.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                    }
                    truncated |= chunk.len() > remaining;
                    append_stream_log(&state, &execution_id, "info", stream_name, chunk).await;
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
                if stream_name == "stderr" && collected.len() < MAX_ERROR_TAIL_BYTES {
                    collected.extend_from_slice(message.as_bytes());
                }
                break;
            }
        }
    }

    if stream_name == "stdout" && output_protocol == OutputProtocol::JsonLines {
        if discarding_oversized_line {
            append_stream_log(
                &state,
                &execution_id,
                "error",
                stream_name,
                b"JSON Lines event exceeded its bounded line quota and was discarded",
            )
            .await;
        } else if !pending_line.is_empty() {
            consume_json_line(
                &pending_line,
                &mut collected,
                &mut truncated,
                &state,
                &execution_id,
                stream_name,
            )
            .await;
        }
    }

    if truncated {
        let marker = if stream_name == "stderr" {
            format!("\n…[仅保留最近 {MAX_ERROR_TAIL_BYTES} 字节错误输出]")
        } else {
            format!("\n…[最终输出已截断，累计超过 {MAX_FINAL_OUTPUT_BYTES} 字节上限]")
        };
        collected.extend_from_slice(marker.as_bytes());
        let mut guard = state.lock().await;
        if let Some(pipeline) = guard.as_mut() {
            if execution_id.is_empty() || pipeline.execution_id == execution_id {
                append_runtime_log(
                    pipeline,
                    "error",
                    format!("[{stream_name}] 输出已按独立通道配额截断"),
                );
            }
        }
    }
    collected
}

async fn append_stream_log(
    state: &Arc<Mutex<Option<PipelineState>>>,
    execution_id: &str,
    level: &str,
    stream_name: &str,
    content: &[u8],
) {
    append_stream_log_with_correlation(state, execution_id, level, stream_name, None, content)
        .await;
}

async fn append_stream_log_with_correlation(
    state: &Arc<Mutex<Option<PipelineState>>>,
    execution_id: &str,
    level: &str,
    stream_name: &str,
    correlation_id: Option<String>,
    content: &[u8],
) {
    let text = String::from_utf8_lossy(content);
    let mut characters = text.chars();
    let display: String = characters.by_ref().take(MAX_RUNTIME_LOG_CHARS).collect();
    let display = if characters.next().is_some() {
        format!("{display}…[截断]")
    } else {
        display
    };
    if display.trim().is_empty() {
        return;
    }
    let mut guard = state.lock().await;
    if let Some(pipeline) = guard.as_mut() {
        if pipeline.status != PipelineStatus::Paused
            && pipeline.status != PipelineStatus::Failed
            && (execution_id.is_empty() || pipeline.execution_id == execution_id)
        {
            let text = format!("[{stream_name}] {}", display.trim());
            if level == "debug" {
                set_runtime_debug_log(pipeline, stream_name, correlation_id, text);
            } else {
                append_runtime_log_with_context(pipeline, level, stream_name, correlation_id, text);
            }
        }
    }
}

fn append_final_output(collected: &mut Vec<u8>, content: &[u8], add_newline: bool) -> bool {
    let remaining = MAX_FINAL_OUTPUT_BYTES.saturating_sub(collected.len());
    let copied = content.len().min(remaining);
    collected.extend_from_slice(&content[..copied]);
    let mut truncated = copied < content.len();
    if add_newline {
        if collected.len() < MAX_FINAL_OUTPUT_BYTES {
            collected.push(b'\n');
        } else {
            truncated = true;
        }
    }
    truncated
}

async fn consume_json_line(
    line: &[u8],
    collected: &mut Vec<u8>,
    truncated: &mut bool,
    state: &Arc<Mutex<Option<PipelineState>>>,
    execution_id: &str,
    stream_name: &str,
) {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.iter().all(u8::is_ascii_whitespace) {
        return;
    }
    match serde_json::from_slice::<serde_json::Value>(line) {
        Ok(value) if value.get("type").and_then(serde_json::Value::as_str) == Some("thought") => {
            let thought = value
                .get("data")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("thought event");
            append_stream_log_with_correlation(
                state,
                execution_id,
                "debug",
                stream_name,
                event_correlation(&value),
                thought.as_bytes(),
            )
            .await;
        }
        Ok(value) => {
            if let Some(text) = event_text(&value) {
                *truncated |=
                    append_final_output(collected, text.as_bytes(), !text.ends_with('\n'));
                append_stream_log_with_correlation(
                    state,
                    execution_id,
                    "info",
                    stream_name,
                    event_correlation(&value),
                    text.as_bytes(),
                )
                .await;
            }
        }
        Err(_) => {
            *truncated |= append_final_output(collected, line, true);
            append_stream_log(state, execution_id, "info", stream_name, line).await;
        }
    }
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
                )));
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
        .envs(spec.environment.iter().map(|(key, value)| (key, value)))
        .kill_on_drop(true)
        .current_dir(project_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(if spec.stdin_payload.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        });
    for key in &spec.environment_remove {
        command.env_remove(key);
    }

    let mut busy_retries = 0;
    let mut child = loop {
        match command.spawn() {
            Ok(child) => break child,
            #[cfg(unix)]
            Err(error) if error.raw_os_error() == Some(26) && busy_retries < 3 => {
                busy_retries += 1;
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let source = match spec.program_source {
                    ProgramSource::PathSearch => "PATH",
                    ProgramSource::SettingsOverride => "设置的覆盖路径",
                };
                return Err(EngineError::NotInstalled(format!(
                    "无法从 {source} 启动 {}",
                    spec.display_name
                )));
            }
            Err(error) => {
                return Err(EngineError::StartFailed(format!(
                    "无法启动 {}：{error}",
                    spec.display_name
                )));
            }
        }
    };

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
    let stdout_protocol = spec.output_protocol;
    let stdout_reader = tokio::spawn(async move {
        stream_process_pipe(
            &mut stdout,
            "stdout",
            stdout_protocol,
            stdout_execution_id,
            stdout_state,
        )
        .await
    });
    let stderr_reader = tokio::spawn(async move {
        stream_process_pipe(
            &mut stderr,
            "stderr",
            OutputProtocol::RawText,
            stderr_execution_id,
            stderr_state,
        )
        .await
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
                        Err(EngineError::cancelled())
                    };
                }

                if started_at.elapsed() > std::time::Duration::from_secs(spec.timeout_secs) {
                    let termination =
                        terminate_child(&mut child, spec.display_name, "执行超时").await;
                    let _ = collect_pipe(stdout_reader, "stdout").await;
                    let _ = collect_pipe(stderr_reader, "stderr").await;
                    clear_child_pid(&state, execution_id).await;
                    termination?;
                    return Err(EngineError::timeout());
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
        let oversized = vec![b'x'; MAX_FINAL_OUTPUT_BYTES + 100];
        let collected = stream_process_pipe(
            Cursor::new(oversized),
            "stdout",
            OutputProtocol::RawText,
            "stale".to_string(),
            state.clone(),
        )
        .await;
        assert!(String::from_utf8_lossy(&collected).contains("输出已截断"));
        assert!(state.lock().await.as_ref().unwrap().log_history.is_empty());
    }

    #[tokio::test]
    async fn oversized_thought_line_cannot_displace_later_final_output() {
        let state = Arc::new(Mutex::new(Some(test_pipeline("json-lines"))));
        let thought = "x".repeat(MAX_JSON_LINE_BYTES + 1);
        let input = format!(
            "{}\n{}\n",
            serde_json::json!({"type":"thought","data":thought}),
            serde_json::json!({"type":"text","data":"FINAL_RESULT"}),
        );
        let collected = stream_process_pipe(
            Cursor::new(input.into_bytes()),
            "stdout",
            OutputProtocol::JsonLines,
            "json-lines".to_string(),
            state.clone(),
        )
        .await;
        assert_eq!(String::from_utf8(collected).unwrap(), "FINAL_RESULT\n");
        let guard = state.lock().await;
        let logs = &guard.as_ref().unwrap().log_history;
        assert!(logs.iter().any(|entry| entry.level == "error"));
        assert!(logs.iter().any(|entry| entry.text.contains("FINAL_RESULT")));
    }

    #[tokio::test]
    async fn thought_uses_structured_live_slot_without_entering_normal_history() {
        let state = Arc::new(Mutex::new(Some(test_pipeline("thought-live"))));
        let input = format!(
            "{}\n",
            serde_json::json!({
                "type": "thought",
                "data": "bounded thought",
                "correlation_id": "turn-7"
            }),
        );
        let collected = stream_process_pipe(
            Cursor::new(input.into_bytes()),
            "stdout",
            OutputProtocol::JsonLines,
            "thought-live".to_string(),
            state.clone(),
        )
        .await;
        assert!(collected.is_empty());
        let guard = state.lock().await;
        let pipeline = guard.as_ref().unwrap();
        assert!(pipeline.log_history.is_empty());
        let current: serde_json::Value = serde_json::from_str(&pipeline.current_log).unwrap();
        assert_eq!(current["kind"], "runtime_log");
        assert_eq!(current["level"], "debug");
        assert_eq!(current["source"], "stdout");
        assert_eq!(current["correlation_id"], "turn-7");
        assert!(current["text"]
            .as_str()
            .unwrap()
            .contains("bounded thought"));
    }

    #[tokio::test]
    async fn thought_flood_preserves_normal_history_and_final_output_budget() {
        let mut pipeline = test_pipeline("thought-flood");
        append_runtime_log(&mut pipeline, "error", "critical history".to_string());
        let state = Arc::new(Mutex::new(Some(pipeline)));
        let mut input = String::new();
        for index in 0..250 {
            input.push_str(
                &serde_json::json!({
                    "type": "thought",
                    "data": format!("thought-{index}"),
                    "id": format!("thought-{index}")
                })
                .to_string(),
            );
            input.push('\n');
        }
        input.push_str(
            &serde_json::json!({
                "type": "text",
                "data": "FINAL_RESULT",
                "id": "final-1"
            })
            .to_string(),
        );
        input.push('\n');

        let collected = stream_process_pipe(
            Cursor::new(input.into_bytes()),
            "stdout",
            OutputProtocol::JsonLines,
            "thought-flood".to_string(),
            state.clone(),
        )
        .await;
        assert_eq!(String::from_utf8(collected).unwrap(), "FINAL_RESULT\n");
        let guard = state.lock().await;
        let logs = &guard.as_ref().unwrap().log_history;
        assert_eq!(logs.len(), 2);
        assert!(logs.iter().any(|entry| entry.text == "critical history"));
        let final_entry = logs
            .iter()
            .find(|entry| entry.text.contains("FINAL_RESULT"))
            .expect("final log");
        assert_eq!(final_entry.level, "info");
        assert_eq!(final_entry.source, "stdout");
        assert_eq!(final_entry.correlation_id.as_deref(), Some("final-1"));
        assert!(logs.iter().all(|entry| entry.level != "debug"));
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
                environment: vec![],
                environment_remove: vec![],
                output_protocol: OutputProtocol::RawText,
                program_source: ProgramSource::SettingsOverride,
                timeout_secs: 5,
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
                environment: vec![],
                environment_remove: vec![],
                output_protocol: OutputProtocol::RawText,
                program_source: ProgramSource::SettingsOverride,
                timeout_secs: 5,
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
                environment: vec![],
                environment_remove: vec![],
                output_protocol: OutputProtocol::RawText,
                program_source: ProgramSource::SettingsOverride,
                timeout_secs: 5,
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
                    environment: vec![],
                    environment_remove: vec![],
                    output_protocol: OutputProtocol::RawText,
                    program_source: ProgramSource::SettingsOverride,
                    timeout_secs: 5,
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
        assert!(matches!(
            result,
            Err(EngineError::Cancelled {
                execution_result: None
            })
        ));
        assert_eq!(state.lock().await.as_ref().unwrap().child_pid, None);
    }

    #[test]
    fn json_lines_are_mapped_to_common_text_output() {
        let output = normalize_stdout(
            OutputProtocol::JsonLines,
            "{\"type\":\"text\",\"data\":\"hello \"}\n{\"type\":\"text\",\"data\":\"world\"}\n{\"type\":\"end\"}\n".to_string(),
        );
        assert_eq!(output, "hello \nworld\n");
    }

    #[test]
    fn thought_channel_does_not_consume_final_output_budget() {
        let thought = "x".repeat(256 * 1024);
        let input = format!(
            "{}\n{}\n",
            serde_json::json!({"type":"thought","data":thought}),
            serde_json::json!({"type":"text","data":"FINAL_RESULT"}),
        );
        let output = normalize_stdout(OutputProtocol::JsonLines, input);
        assert_eq!(output, "FINAL_RESULT\n");
    }

    #[test]
    fn json_line_split_across_transport_chunks_is_complete_after_collection() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"{\"type\":\"text\",\"da");
        bytes.extend_from_slice(b"ta\":\"complete\"}\n");
        let output = normalize_stdout(OutputProtocol::JsonLines, String::from_utf8(bytes).unwrap());
        assert_eq!(output, "complete\n");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn process_specific_timeout_stops_fake_cli() {
        let directory = TestDirectory::new("timeout");
        let cli = directory.path.join("slow-cli");
        write_fake_cli(&cli, "exec sleep 10");
        let state = Arc::new(Mutex::new(Some(test_pipeline("timeout"))));
        let result = run_process(
            ProcessSpec {
                display_name: "Fake",
                program: cli.into_os_string(),
                args: vec![],
                stdin_payload: None,
                environment: vec![],
                environment_remove: vec![],
                output_protocol: OutputProtocol::RawText,
                program_source: ProgramSource::SettingsOverride,
                timeout_secs: 1,
            },
            directory.path.to_str().unwrap(),
            "timeout",
            state,
        )
        .await;
        assert!(matches!(
            result,
            Err(EngineError::Timeout {
                execution_result: None
            })
        ));
    }
}
